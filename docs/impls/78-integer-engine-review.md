# Integer Engine Review — 2026-09-02

Feature 78 · P2 · GitHub issue [#78](https://github.com/yicheng47/pulse/issues/78) · spec [`features/78-integer-engine-hardening.md`](../features/78-integer-engine-hardening.md). The review record behind the spec: what the bit-perfect path does byte by byte and call by call, what was verified against the dependencies rather than assumed, what was found, and how the fixes should land. Reviewed at `37dfb4b` (after the quit-release refactor `f5150fd`); `cargo test -p pulse-engine` 138 green. Companion to [`arch/pulse-engine.md`](../arch/pulse-engine.md) §9 and the M3 record in [`archive/bit-perfect/`](archive/bit-perfect/README.md).

## 1. Verdict

The sample path is bit-exact and the claim stands. Between the decoder's integer output and the HAL's buffer there is no multiply, no float, no dither, no gain — only zero padding, sign extension, and copies. The alignment math in `IntPacker` is correct for every container flag word the stage-1 probe found, the DoP packer matches the DoP spec, and the hardware proof (the Matrix reading "DSD DoP 2.8MHz") validates the whole chain because DoP markers do not survive a single flipped bit.

Everything found is at the edges: how the engine handles the device around the sample path, how the controller drives it across track boundaries, and how the app decides which devices get the engine. Three of those are worth code (§4.1–4.3), one is a documented policy worth understanding (§4.4), and the rest are minor (§4.5).

## 2. The path, byte by byte

### 2.1 Stations

A sample passes six stations. The first four run on the controller thread; the fifth is the HAL's realtime IO thread; the sixth is the driver.

| # | Station | Code | What happens to the sample |
|---|---|---|---|
| 1 | Container decode | `decode.rs` `PcmDecoder::next_pcm` → symphonia | FLAC and ALAC produce `i32` with the value shifted up by `32 − bits`; WAV and AIFF produce typed `i16` / `i24` / `i32` |
| 2 | Interleave to bytes | `decode.rs` `write_interleaved_bytes` | `>> 16` to `i16`, `>> 8` to `i24`, or a straight copy; native-endian bytes, source width, channels interleaved |
| 3 | Pack to the device container | `integer_engine.rs` `IntegerEngine::feed` → `IntPacker::pack` | zero bytes below, the source bytes, sign bytes above — no arithmetic |
| 4 | Ring | rtrb `Producer<u8>::push_partial_slice` / `Consumer<u8>::pop_partial_slice` | byte copy, bounded at 4 s of device frames |
| 5 | IOProc | `raw_sink.rs` `output_io_proc` | `pop_partial_slice` straight into the HAL's buffer, zero-fill any shortfall, bump two atomics |
| 6 | HAL → driver → USB | virtual format == physical integer format, hog held, non-mixable | nothing converts; the driver puts the words on the wire |

Station 6 is the whole point of M3: because the stream's virtual format is set to the same integer ASBD as its physical format, the buffer the IOProc fills is the buffer the driver reads. That is what turned 0003's raw-HAL noise (integer bytes dropped into a float32 buffer) into a memcpy.

### 2.2 Worked example: 24-bit FLAC on the Matrix

Take two 24-bit samples: the most negative value `0x800000` (−8,388,608) and `0x123456` (+1,193,046).

1. symphonia's FLAC decoder stores them as `i32` shifted left by `32 − 24 = 8`: `0x80000000` and `0x12345600` (`symphonia-bundle-flac-0.5.5/src/decoder.rs:173`).
2. `write_promoted_s32_as_i24` shifts right by 8 (arithmetic, so the sign survives): `0xFF800000` and `0x00123456`. `i24::from` clamps, a no-op here, and `to_ne_bytes` drops the top byte of the little-endian `i32`: `00 00 80` and `56 34 12`.
3. The Matrix's preferred format is `0x54` — `IsSignedInteger | IsAlignedHigh | IsNonMixable`, 24 valid bits in a 4-byte container. `IntPacker::new` computes `low_zero_bytes = 4 − 3 = 1`, `high_sign_bytes = 0`. `pack` emits `00 00 00 80` and `00 56 34 12`.
4. The IOProc copies the 8-byte stereo frame into a buffer whose virtual format is that same `0x54` ASBD. The driver sends the 32-bit word; a 24-bit DAC takes the top 24 bits: `0x800000` and `0x123456`. Exact.

The Matrix's other accepted format, `0x4c` (`IsPacked`, 32 valid bits), produces byte-identical output: `device_bytes_per_sample = 4`, `low_zero_bytes = 4 − 3 = 1`. So the exact-width preference in `integer_candidate_rank` is harmless on this device; it matters only on devices where a 24-bit-in-3-byte format exists beside a 32-bit one.

A low-aligned container — flags with neither `IsAlignedHigh` nor `IsPacked`, 24 valid bits in 4 bytes — gets `low_zero_bytes = 3 − 3 = 0` and `high_sign_bytes = 1`: `00 00 80 FF` and `56 34 12 00`. Right-justified signed samples must be sign-extended, and the test `packer_sign_extends_24_bit_in_a_low_aligned_32_bit_container` pins that.

### 2.3 Worked example: DSD64 DFF into DoP

A DFF file interleaves DSD bytes per channel, MSB-first. For one channel, take two consecutive DSD bytes `A5` then `3C`. The DoP word packs the earlier byte above the later one under a marker: `marker << 16 | earlier << 8 | later` = `0x05A53C`. Little-endian in 3 bytes: `3C A5 05`. In the Matrix's `0x54` container: `00 3C A5 05`. The next frame carries marker `0xFA`; markers alternate per frame and both channels of a frame share the marker (`decode_dsd.rs` `take_marker`).

The DAC watches the top byte of each 24-bit word. A run of alternating `05` / `FA` is the DoP signature; it switches to DSD and strips the markers. A corrupted marker byte breaks the run and the DAC falls back to PCM (the packed frames sound like hiss); a corrupted data byte plays as a wrong DSD bit. Neither happened on the Matrix, which is why one display readout proves the whole chain.

DSF differs only at station 1: 4096-byte per-channel blocks, LSB-first, so every byte is bit-reversed through the LUT in `reverse_bits` before packing. `next_pcm` clips each read to the current block so a frame never straddles a channel-block boundary.

### 2.4 The five invariants

Bit-perfect holds because all five hold at once. Each finding below is about protecting one of them on hardware or paths not yet exercised.

1. **No arithmetic after decode.** Stations 2–5 only shift, pad, sign-extend, and copy. The integer engine has no `GainProcessor`; `set_volume` writes the device's hardware control or does nothing (`VolumeDomain::Fixed`).
2. **Container at least as wide as the source.** `integer_candidate` rejects anything narrower; `IntPacker::new` rejects anything wider than 4 bytes. A 32-bit source on a 24-bit device refuses rather than truncates (§4.5).
3. **Virtual == physical.** `set_format` writes the same ASBD to `kAudioStreamPropertyPhysicalFormat` and `kAudioStreamPropertyVirtualFormat`, waiting on readback for each (`hal.rs` `set_stream_format`), because a refused write returns `noErr` and simply never settles.
4. **Hog held and mixing off.** Nobody else's audio is mixed in, and the HAL has no reason to insert a converter. Hog is mandatory: `IntegerEngine::open` refuses without it.
5. **The callback is a copy.** No allocation, lock, or syscall in `output_io_proc`; `pop_partial_slice`, `fill(0)`, two relaxed atomics, return.

### 2.5 Dependencies verified, not assumed

- symphonia FLAC: `symphonia-bundle-flac-0.5.5/src/decoder.rs:173–174` — `let shift = 32 - bits_per_sample; self.buf.transform(|sample| sample << shift)`. ALAC: `symphonia-codec-alac-0.5.5/src/lib.rs:572–574`, the same shift. So the `>> 16` and `>> 8` in `decode.rs` restore exact values.
- symphonia PCM (WAV/AIFF 24-bit): `symphonia-codec-pcm-0.5.5/src/lib.rs:291` reads `i24 << 8` and converts back to `i24` — net identity. `symphonia-core-0.5.5/src/audio.rs:949`: `impl RawSample for i24 { type RawType = [u8; 3] }`, so `RawSampleBuffer<i24>::as_bytes` is 3 bytes per sample, not 4.
- rtrb 0.3.4 `chunks.rs`: `push_partial_slice` (281), `pop_partial_slice` (408), `read_chunk(..).commit_all()` (342, 651) exist with the semantics the code relies on.
- Core Audio `AudioHardware.h` (MacOSX 26.5 SDK), the `AudioDeviceIOProc` doc at lines 765–780: `outOutputData` carries one `AudioBuffer` per output stream; "for streams that are disabled, the AudioBuffer's mData field will be NULL but the mDataByteSize field will still say how much data would have been there".
- `cargo test -p pulse-engine`: 138 passed at `37dfb4b`. The `decode.rs` seek tests pin first-sample values for 16/24/32-bit WAV and a FLAC fixture, which is an end-to-end check on stations 1–2.

## 3. Device state, call by call

Since `f5150fd` the sink, the format-restore guard, and the hog guard live in `IntegerDeviceResources` behind `IntegerReleaseHandle` (`integer_engine.rs:131–160`), a mutex the app's quit path can lock from another thread with a deadline. Every engine method locks it; the realtime callback never touches it. Release order inside `release_resources` (`integer_engine.rs:197`) is sink stop and drop → format restore (every output stream's physical and virtual ASBD, then mixing) → hog drop, and the struct's field order preserves that on the plain drop path too.

| Transition | Controller | Engine | HAL calls |
|---|---|---|---|
| First play | `start_path` → `start_backend` → factory | `open`: `HogGuard::acquire`, `FormatRestoreGuard::capture`, mixing off. `set_format`: `select_integer_format`, nominal rate, physical, virtual, new ring. `play`: `RawSink::start` | hog toggle + readback; capture reads; mixing write + poll; rate write + poll; physical write + poll; virtual write + poll; `AudioDeviceCreateIOProcID`; `AudioDeviceStart` |
| Same-format next track | `begin_seamless_transition` swaps the decoder, keeps feeding | nothing | nothing |
| Different-format next track | `rebuild_for_preloaded` (`controller.rs:1343`): `release_backend` then `start_backend` | `release_resources`, then a fresh `open` + `set_format` + `play` | see §4.2 |
| Pause | `pause` → `stop_active` (drops the decoder) | `pause` → `RawSink::stop`: `AudioDeviceStop`, then drain the ring | stop |
| Resume | `resume` → `start_path` (new decoder, seek) → `start_backend` reuses the box | `set_format` early-returns on the same format; `play` → `restart` (counters to 0) | `AudioDeviceStart` |
| Seek while playing | `stop_active` → `start_path` | as pause + resume | stop, start |
| Stop, device or mode switch, quit | `release_backend` / `shutdown` | `release_resources` | stop, destroy IOProc, restore per stream, mixing, hog |

Two things to hold onto from this table. First, pause and seek are already cheap on the device: hog, formats, and mixing stay put, only the IOProc stops. (Feature 76 removes the remaining decoder rebuild and device stop.) Second, the one heavy row is the different-format boundary, and it is heavy by controller policy, not by engine capability — that is §4.2.

## 4. Findings

### 4.1 Stream-indexed IOProc fill

**What the code does.** `output_io_proc` (`raw_sink.rs:181–195`) loops over every buffer in the HAL's `AudioBufferList` and pops the ring into each in turn. `select_integer_format` (`integer_engine.rs:455`) negotiates the format on exactly one stream and returns its id, but that id never reaches the sink.

**Why it matters.** The HAL hands the IOProc one buffer per output stream, in the order of `kAudioDevicePropertyStreams`. On a single-stream device there is one buffer and the loop is correct. On a device with two output streams — a pro interface exposing separate stereo pairs, an aggregate, some DACs with a second S/PDIF stream — consecutive frames from the ring alternate between the two buffers, so each output gets every other chunk of the music. Worse, the second stream's virtual format was never changed: it is still the HAL's default float32, so the integer bytes it receives are read as floats. That is 0003's noise, back on one output.

**The disabled-stream corollary.** Per the header contract (§2.5), a disabled stream's buffer arrives with `mData == NULL` and a nonzero `mDataByteSize`. The null branch at `raw_sink.rs:184` adds that size to `underrun_bytes` on every cycle, so a device with one active and one disabled stream reports a continuous dropout stream to the UI while playing perfectly.

**Evidence it is real.** Every device in the stage-1 probe table has exactly one output stream, which is why this never showed. mpv's exclusive Core Audio output keeps a `stream_idx` and reads only `out_data->mBuffers[p->stream_idx]` in its render callback (`ao_coreaudio_exclusive.c`, `render_cb_compressed`), with the comment that devices can have multiple sub-streams and it only selects the first that qualifies.

**Fix.** `select_integer_format` returns the stream's index alongside its id (the position in `hal::output_streams`). `RawSink::start` takes the index into `CallbackContext`. The loop becomes: for the selected index, pop the ring and count underrun on shortfall or null; for every other buffer, `fill(0)` if non-null and count nothing. Optionally set `kAudioDevicePropertyIOProcStreamUsage` so the HAL marks the unused streams off, which makes their buffers arrive null — the loop above already handles that. Cost in the callback: one integer compare per buffer.

**Test.** Extend `io_proc_copies_ring_bytes_zero_fills_and_counts_frames` with a two-buffer list (a local `#[repr(C)]` struct with two `AudioBuffer`s, since the crate's `AudioBufferList` type has a one-element array): selected index 1, buffer 0 must be zeroed, buffer 1 must carry the ring bytes, position and underrun must count only buffer 1. A second case with buffer 0 null must add nothing to underrun.

**Severity.** Correctness on hardware nobody has tested; silent on the Matrix. Low likelihood for the current audience, but this is exactly the class of device a "bit-perfect" label attracts.

### 4.2 In-place format change at track boundaries

**What the code does.** When the next preloaded track's format differs, `pump` waits until the backend has played everything fed, then calls `rebuild_for_preloaded` (`controller.rs:1343`), which runs `release_backend` and then `start_backend`. `release_backend` stops the sink and calls `backend.release()`, and `start_backend` finds no backend and asks the factory for a new one.

**The call sequence on the integer engine, today.** For a 96/24 track followed by a 44.1/16 one, with the device at float32 44.1 kHz before Pulse started:

1. `release_resources`: `AudioDeviceStop`, `AudioDeviceDestroyIOProcID`, then restore the saved physical format (float32 at 44.1 kHz — a rate switch from 96 kHz), restore the saved virtual format, restore mixing, drop the hog.
2. `IntegerEngine::open`: acquire the hog again, capture the formats again (the ones just restored), mixing off again.
3. `set_format`: nominal rate 44.1 kHz (already there), physical integer 44.1 kHz (a format switch), virtual integer (another write).
4. `play`: `AudioDeviceCreateIOProcID`, `AudioDeviceStart`.

That is two physical-format switches and up to two rate switches where one of each would do, each write waiting on readback with a 2 s ceiling (`hal.rs:47`), and a window between steps 1 and 2 where the device is free for any other process to hog. The controller also clears `bit_perfect_active` and the volume state in `release_backend`, so the UI's bit-perfect indicator drops and returns at every such boundary.

**Why the engine does not need it.** `IntegerEngine::set_format` (`integer_engine.rs:269`) already handles a format change on a live engine: it stops and drops the sink, writes the new nominal rate and formats on the still-hogged device, and builds a fresh ring; `play` then creates a new sink. `AuhalEngine::set_format` on the universal path does the equivalent. Both backends' `start(format)` is exactly "renegotiate if needed, then play".

**Where it bites.** Every album change between rate families (44.1 → 96, 96 → 192) and every PCM ↔ DSD boundary, since a DoP track is 176.4/24 and a CD track is 44.1/16. On a DAC with a muting relay, that is extra clicks; on any DAC it is a longer gap and a brief PCM flash of the pre-Pulse format on the display.

**Fix.** In `rebuild_for_preloaded`, replace `self.release_backend()?` with `self.stop_active()?`. `start_backend` then reuses the box through `take_or_open_backend` (same device, same kind) and `backend.start(next.format)` renegotiates. The bit-perfect and volume state stay steady because `release_backend` is never called. `stop_active` on the integer engine drains a ring that is already empty (the rebuild is gated on `backend_position >= fed_frames`), so nothing audible is lost.

**Why it was built this way.** The stage-3 note records the choice: "format changes take the rebuild path, which also exercises the restore guard." That was a reasonable way to get restore coverage before the DoP proof; with `f5150fd`'s explicit release paths and their tests, the guard has coverage of its own.

**Test.** `format_mismatch_rebuilds_backend_and_still_advances` asserts only the `Advanced` and `Position` events, so the change does not break it; extend it to assert `releases == 0` and that no `BitPerfectStateChanged` event fires across the boundary, and add the `EngineKind::BitPerfect` variant. Hardware: the three-track sequence in the spec's verification, watching the Matrix's display for exactly one change per boundary.

### 4.3 AUTO gate parity

**The two predicates.** The app resolves a device with no stored mode to Bit-perfect when the probe reports an integer path and the transport is not display-class or Bluetooth (`logic.rs:324` `automatic_output_mode`; `settings.rs:146` for stored capabilities). The "integer path" half comes from `hal.rs:925` `maximum_physical_format_capabilities`, which accepts any linear-PCM physical format with `IsSignedInteger` set. The engine, when it actually opens, filters with `integer_candidate` (`integer_engine.rs:479`):

| Check | Probe (`maximum_physical_format_capabilities`) | Engine (`integer_candidate`) |
|---|---|---|
| Linear PCM, signed integer, not float | yes | yes |
| `IsNonMixable` set | — | required |
| Little-endian | — | required |
| Interleaved | — | required |
| Whole-byte width, container ≤ 4 bytes, bytes-per-frame divisible by channels | — | required |
| Channel count equals the source | — | required |
| Rate available | — | required |

**Evidence.** The stage-1 table is the proof: all three DELL displays advertise 16/20/24-bit signed-integer physical formats at flags `0x0c` / `0x04` — no non-mixable twin — and refused every integer virtual write. The probe reports them as "24-bit integer"; only `transport.supports_bit_perfect()` (`device.rs:54`) keeps them out of Bit-perfect, because they happen to be DisplayPort. A USB or Thunderbolt device with the same shape passes both halves of the gate, auto-resolves to Bit-perfect, and fails at start with `NoMatchingPhysicalFormat`. There is no fallback: `exclusive_start_can_fallback` (`controller.rs:1705`) applies only to `Universal { exclusive_mode: true }`, so the result is an `Error` state and a toast, every time that device is selected, until the user pins a mode by hand.

**Fix.** One predicate in `hal.rs`, say `is_integer_wire_format(&AudioStreamBasicDescription) -> bool`, holding the flag and layout checks that do not depend on the source (`integer_candidate` keeps the source-dependent ones: channels, width, rate). The probe reports `integer_wire_formats: bool` — any available physical format passing it — beside `max_bits_per_channel`; `OutputDeviceCapabilities` and `StoredDeviceCapabilities` carry it, with `None` in stored data forcing a reprobe exactly as `transport` did (`stored_capabilities_without_transport_require_a_reprobe`). `automatic_output_mode` requires it for Bit-perfect; the Devices page's `NO INTEGER PATH` tag reads it. `max_bits_per_channel` keeps its current meaning, because it is also the capability line and the Exclusive gate, and the DELLs' "24-bit integer" is true for AUHAL exclusive.

**Test.** A table test over the stage-1 flag words: `0x54` and `0x4c` pass, `0x14`, `0x0c`, `0x04` fail, a big-endian or non-interleaved variant fails. A resolver test: integer formats present but no wire format → Exclusive, not Bit-perfect.

### 4.4 DoP, zero-fill, and the pause-lock claim

This one is a policy, already decided in feature 71 ("zero-filled frames carry no markers, so the DAC drops out of DSD and re-locks when data resumes — a brief mute/click, accepted"). It is here because the mechanism is worth understanding and because one sentence in the architecture doc gets ahead of the hardware evidence.

**Mechanism.** The IOProc's only fallback is `fill(0)`. For PCM that is digital silence, the right answer. For DoP a run of `00 00 00` words has no markers; the DAC's DoP detector drops out after a handful of markerless frames, the DAC switches back to PCM mode (with whatever mute or relay that involves), plays the zeros as silence, and re-locks only after the marker run resumes. So a few milliseconds of underrun costs a mode switch each way. The same happens at every `restart`: `RawSink::start` and `restart` begin IO before the pump has fed anything, so the first callbacks are zeros.

**The option, if it is ever revisited.** A DoP-aware fill: instead of zeros, emit DSD idle bytes under the correct alternating markers. The callback would need the marker phase, which it can derive from `position_frames` parity if the packer's phase is aligned to frame 0 of the ring, and a device-agreed idle byte pattern — implementers use a DC-free pattern, and it should be confirmed on the Matrix rather than taken from folklore. It stays out of scope for 78.

**The claim to keep unclaimed.** `arch/pulse-engine.md` §9 says pause keeps "for DoP — the DAC's DSD lock". After `AudioDeviceStop` no data flows to the device at all, and whether a given DAC holds DSD mode through a data gap is a DAC firmware question, not something the engine controls. Feature 71's phase 4 lists "pause holds the lock" as a Matrix observation still pending; the architecture sentence should follow that observation, not precede it.

### 4.5 Minor

- **`Hogged` payloads that mislead.** `IntegerEngine::open` (`integer_engine.rs:246`) returns `Hogged(own pid)` when `HogGuard::acquire` reports this process already held the hog, and `acquire` itself (`hal.rs:100`) returns `Hogged(-1)` when the hog write was accepted but the device stayed free, which virtual and aggregate devices do. Both render as "device hogged by pid N". Two distinct variants, or a `HogRefused` with a reason, would read honestly and let the app's `PlaybackErrorKind::Device { hog_pid }` stop offering a pid that is not another process.
- **32-bit sources refuse on 24-bit devices.** Invariant 2 rejects a 32-bit integer WAV on a device whose widest container is 24 bits. That is the right call under "bit-perfect or refuse", but the universal path would have played it; the refusal message names the format, which is enough.
- **Pump cadence.** `FEED_RETRY_DELAY` (`controller.rs:20`) is 2 ms. Against a 4-second ring (`reset_ring`, `integer_engine.rs:429`) the pump wakes about 500 times a second while the ring is full, each wake checking `slots()` and packing a sliver. 10 ms would cut that fivefold with no risk at this depth; it is shared with the universal path.
- **Stop-then-drain threading.** `RawSink::stop` (`raw_sink.rs:133`) drains the consumer on the caller's thread right after `AudioDeviceStop`. That is the one place two threads touch the ring's consumer, and it is sound only because the HAL's stop does not return while the IOProc is executing. `AudioHardware.h` does not document that; the `unsafe impl Send` comment at `raw_sink.rs:39–41` encodes the same assumption, so it is now stated in the code.
- **The universal engine's Exclusive mode has no restore guard.** `AuhalEngine::set_format` (`auhal_engine.rs:88–94`) writes a physical format through `set_matching_physical_format` and ignores the result, and `AuhalEngine` holds no `FormatRestoreGuard`, so after Exclusive playback the device keeps Pulse's format. The integer engine restores; the universal one does not. Outside the bit-perfect claim; noted for consistency.

## 5. Checked and right

The boundaries of the review, so the next reader knows what not to re-derive.

- `IntPacker::new` arithmetic for all nine source-width × container combinations covered by the tests, including the low-aligned sign-extension case and the aligned-high 16-into-24-in-32 case.
- `IntegerEngine::feed` never pushes a partial frame: `writable_frames` is computed from `slots()` before packing, and the consumer only ever pops whole frames, so the byte ring stays frame-aligned.
- `integer_candidate` accepts a format whose `mSampleRate` is 0 with a range (`kAudioStreamAnyRate`) and pins the requested rate into the ASBD before the write.
- Pause and resume land on the audible position: `logical_position_ms` reads the IOProc's consumed-frame counter, the ring is drained on stop, and `start_path` re-seeks a fresh decoder to that position with fresh counters. The dropout baseline is re-established after every restart (`priming_and_drain_tail_underruns_are_not_counted`, `pause_resume_preserves_the_track_tally_and_rebaselines_the_new_sink`).
- Gapless splicing is engine-agnostic and works on the integer path: `begin_seamless_transition` swaps decoders under a live ring; `advance_transition_if_audible` gates the `Advanced` event on the consumed-frame counter. A 176.4/24 PCM track and a DSD64 DoP track share a `PcmFormat`, so that boundary also splices, and the DAC handles the mode change from the marker stream.
- RAII on both release paths: explicit `release_resources` and the plain drop both restore formats while the hog is still held, then release the hog; `HogGuard::drop` checks ownership before toggling; mixing is captured and restored.
- The realtime callback obeys all five rules; the only shared state is two relaxed atomics and the ring.
- The DSD gate in the app reads the resolved engine kind (`playback_output_mode` is set from `output_mode_for_engine_kind`), so AUTO-resolved Bit-perfect passes it.
- `HogGuard` semantics and the mandatory-hog refusal; `FormatRestoreGuard` capturing every output stream, not just the selected one.

## 6. Build plan

Engine-only except phase 2's app wiring. One crew mission, phases in this order; each phase is independently mergeable.

1. **Boundary reuse** — `controller.rs` `rebuild_for_preloaded`: `stop_active` instead of `release_backend`. Tests: extend `format_mismatch_rebuilds_backend_and_still_advances` (release count 0, no `BitPerfectStateChanged` across the boundary), add the `BitPerfect` variant, and a universal-exclusive variant to show the shared fallback still works if `start` fails after reuse.
2. **Gate parity** — `hal.rs`: `is_integer_wire_format`; `integer_candidate` calls it; `maximum_physical_format_capabilities` returns the wire-format bit; `device.rs` `OutputDeviceCapabilities::integer_wire_formats`; app `settings.rs` `StoredDeviceCapabilities` gains the optional field with the reprobe rule; `logic.rs` `automatic_output_mode` / `automatic_stored_output_mode` and the `NO INTEGER PATH` tag read it. Tests: the flag-word table; the resolver case; the stored-capabilities reprobe case.
3. **Stream index** — `integer_engine.rs` `select_integer_format` returns `(index, id, format)`; `raw_sink.rs` `RawSink::start` takes the index; the callback loop per §4.1; optional `IOProcStreamUsage`. Tests: the two-buffer IOProc cases.
4. **Minor** — the hog error variants and the 10 ms retry; the architecture doc's pause-lock sentence rewritten to cite feature 71's pending observation.

## 7. Verification

- `make verify` green after each phase.
- Matrix Mini-i Pro 4, display visible: 44.1/16 → 96/24 → DSD64, queued as one album. Expect one rate change per boundary, no flash back to the pre-Pulse format, the bit-perfect indicator steady through both boundaries, and the DSD readout at the third track. After Stop, Audio MIDI Setup shows the original format restored.
- Any two-stream device, if one is at hand: audio only on the selected pair, silence on the other, no dropout counter movement.
- A device that advertises integer physical formats without a non-mixable twin (the DELLs qualify if the transport gate is bypassed in a dev build): resolves to Exclusive, never errors at start.

## Log

- **2026-09-03** — Double-checked against the code at `b5f6ac1` by the lead session: every finding, line reference, symphonia/rtrb/`AudioHardware.h` citation, and the packer arithmetic confirmed; `cargo test -p pulse-engine` 138 green. Two additions. (a) §4.1's buffer-order premise is convention, not text: `kAudioDevicePropertyStreams` says nothing about IOProc order; what the header pins is `kAudioDevicePropertyStreamConfiguration` ("corresponds to what will be passed into the IOProc") and the null buffer for streams marked off through `kAudioDevicePropertyIOProcStreamUsage` — so the fix makes stream usage mandatory and checks the configuration's buffer count at sink start. (b) Mono sources refuse under Bit-perfect with no fallback: `integer_candidate` requires channel equality, the decoder passes the file's channel count through, nothing upmixes, and `exclusive_start_can_fallback` never applies to the `BitPerfect` kind — a mono FLAC errors on every play and a stereo→mono boundary fails inside `rebuild_for_preloaded`; the universal path accepts (`hal.rs:907`). Recorded as stage 5 of [`78-integer-engine-plan.md`](78-integer-engine-plan.md), pending Jason's call. Build plan superseded by that note: PR per stage, Jason reviews each personally.
- **2026-09-02** — Review performed at `37dfb4b` after fast-forwarding past `f5150fd`; findings unchanged by the release-handle refactor. Issue #78 filed; this note and the spec written.
