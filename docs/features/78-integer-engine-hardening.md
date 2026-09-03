# Integer Engine Hardening

Feature 78 · P2 · GitHub issue [#78](https://github.com/yicheng47/pulse/issues/78). Filed from the 2026-09-02 review of the bit-perfect path. The review record — what the path does byte by byte, what was verified, each finding's mechanism, and the build plan — is [`impls/78-integer-engine-review.md`](../impls/78-integer-engine-review.md); this spec is the short contract.

## Motivation

The integer engine's sample path is clean: nothing between decode and the IOProc touches a sample value, and the DoP acceptance proved it on hardware. The review found the remaining risk at the edges. Track boundaries with a format change tear the device down and rebuild it when the engine can renegotiate in place, which costs extra format switches, a hog gap, and up to six seconds of settle time on every album rate change or PCM↔DSD boundary. The AUTO resolver's idea of "has an integer path" is looser than the engine's, so a device can be auto-resolved to Bit-perfect and then refuse at start with no fallback. And the IOProc fills every buffer in the HAL's list from one ring, which is only correct on single-stream devices — every device probed so far, but not every device a user will plug in under a bit-perfect label.

## Scope

- **In-place format change at track boundaries.** `rebuild_for_preloaded` stops the active playback and reuses the backend through `start_backend`, so `set_format` renegotiates the rate and formats on the held device instead of releasing, restoring, re-hogging, and re-capturing. Applies to both engines; the integer engine is the one that benefits.
- **Gate parity.** One predicate, in `hal.rs`, decides whether an `AudioStreamBasicDescription` is an integer wire format the engine will accept (linear PCM, signed integer, non-mixable, little-endian, interleaved, container ≤ 32 bits, whole-byte width). The capability probe reports whether the device offers at least one, the stored capabilities carry it (absent → reprobe, as transport did), and the Exclusive resolver (`resolve_engine_kind`, feature 81) plus the Devices page's `NO INTEGER PATH` tag read it. The existing `max_bits_per_channel` keeps its meaning for the capability line and for Exclusive. Landed inside feature 81 phase 2 (`995e426`).
- **Stream-indexed IOProc fill.** `select_integer_format` reports the chosen stream's index (its position in the device's output stream list); `RawSink` carries it; the callback pops the ring only into that stream's buffer, zero-fills every other non-null buffer, and counts underruns only against the selected stream. A disabled stream's null buffer no longer counts as dropout. The sink marks every other stream off through `kAudioDevicePropertyIOProcStreamUsage` (mandatory — the header pins the null-buffer contract to it) and refuses at start if `kAudioDevicePropertyStreamConfiguration` reports a different buffer count than the stream list (2026-09-03).
- **Mono sources (decision pending, 2026-09-03).** A 1-channel source refuses on the integer engine today because `integer_candidate` requires an exact channel match, and there is no fallback for the Bit-perfect kind. Proposed: accept a 2-channel integer format for a mono source and duplicate each sample into both slots in the packer — a copy, bit-exact per channel. Jason decides before stage 5 of the plan.
- **Minor items** carried by the same mission if cheap: a distinct error for "hog write accepted but device stayed free" and for "this process already holds the hog" so the UI does not print `hogged by pid -1` or our own pid; a longer pump retry sleep (10 ms) against the 4-second ring.

## Non-Goals

- No change to `IntPacker`, the decoders, or the DoP packer — the sample path is not touched.
- DoP zero-fill on underrun and on restart stays feature 71's documented policy; a DSD-silence fill is recorded as an option in the review, not scheduled.
- Same-track seek and pause/resume are feature 76. Manual device release is feature 77.
- The universal engine's missing format-restore guard in Exclusive mode is noted in the review and left alone.

## Implementation Phases

Staged in [`impls/78-integer-engine-plan.md`](../impls/78-integer-engine-plan.md): one branch, one PR into `main`, and one `codex-crew` mission per phase, run one at a time; **Jason reviews and merges each PR personally** (decided 2026-09-03). The phases below are the plan's stages 1–4; the mono item is its stage 5.

1. **Boundary reuse** (`controller.rs`): `rebuild_for_preloaded` uses `stop_active` + `start_backend`; extend `format_mismatch_rebuilds_backend_and_still_advances` to assert zero releases and no `BitPerfectStateChanged` flicker; add the integer-kind variant.
2. **Gate parity** (`hal.rs`, `device.rs`, app `logic.rs`/`settings.rs`): extract `is_integer_wire_format`, use it from `integer_candidate` and the probe, add the capability field + stored-capability reprobe, wire the resolver and the tag — landed via feature 81 phase 2 (`995e426`).
3. **Stream index** (`integer_engine.rs`, `raw_sink.rs`): return the index from format selection, thread it into `RawSink::start`, restructure the callback loop, extend the IOProc unit test with a two-buffer list.
4. **Minor items** as above.

## Verification

- `make verify` green; new unit tests per phase (release count and event sequence at a format-change boundary; predicate table with the stage-1 flag words `0x54`, `0x4c`, `0x14`, `0x0c`, `0x04`; two-buffer IOProc fill with a disabled second stream).
- Hardware, Matrix Mini-i Pro 4: play a 44.1/16 track followed by a 96/24 track and then a DSD64 track with the display visible — one rate change per boundary, no PCM flash back to the pre-Pulse format, the bit-perfect indicator steady. Audio MIDI Setup shows the original format restored only after Stop or quit.
- Hardware, if available: any device with two output streams (an interface exposing separate stereo pairs, or an aggregate) — audio only on the selected pair, silence on the other, no phantom dropout counter.
