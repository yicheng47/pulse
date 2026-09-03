# 78 — Integer engine hardening: staged build plan

> Impl note for [feature 78](../features/78-integer-engine-hardening.md) (issue [#78](https://github.com/yicheng47/pulse/issues/78)). The contract is the spec; the mechanism behind every stage is the review record [`78-integer-engine-review.md`](78-integer-engine-review.md), double-checked against the code on 2026-09-03 (all findings confirmed; two additions in its Log). This is the core of the app, so the merge gate is different from the usual flow: **Jason reviews every PR personally.** Each stage is one branch, one PR into `main`, one `codex-crew` mission, run one at a time — the crew works in the shared checkout, and every stage starts from the `main` that includes the previous one.

## Stage order and why

| Stage | Finding | Touches | Size | Benefit |
|---|---|---|---|---|
| 1 | §4.2 in-place format change | `controller.rs` | one line + tests | every album rate change and every PCM↔DSD boundary on the Matrix |
| 2 | §4.3 AUTO gate parity | `hal.rs`, `device.rs`, app `settings.rs` / `logic.rs` / tag | small, crosses the crate line | a USB device with integer-but-mixable formats stops erroring under AUTO |
| 3 | §4.1 stream-indexed IOProc fill | `integer_engine.rs`, `raw_sink.rs` | medium, realtime callback | correctness on multi-stream devices; no phantom dropout counter |
| 4 | §4.5 minor | `error.rs`, `hal.rs`, `integer_engine.rs`, `controller.rs` | small | honest hog errors, 5× fewer pump wakeups |
| 5 | mono sources (2026-09-03 addition) | `integer_engine.rs` | small | pending Jason's decision — not fired until he picks |

Stage 1 goes first because it is the smallest diff with the largest audible payoff and it is the one the Matrix can prove. Stages 2–4 are independent of each other and of 1; the order is by user impact. Each stage must leave `make verify` green and be mergeable on its own.

## Rules for every stage

- Branch from the current `main` (`git pull` first), named `feature/78-stage-<n>-<slug>`. Scope is exactly the stage below — no drive-by refactors, no touching another stage's files, no changes to `IntPacker`, the decoders, or the DoP packer (spec non-goal).
- Verification is `make verify` (`check`, `test`, `clippy`, `fmt-check`, `check-units`) plus the stage's listed tests. Never launch the app (`make run` / `cargo run` / `open`) and never produce audio; hardware checks are Jason's after merge.
- PR mode is authorized: after the reviewer reports a clean working-tree diff, commit task-scoped changes, push, open a PR into `main` titled `78 · stage <n> — <name>`, and drive CI green (`gh pr checks <pr> --watch`). The PR body carries the summary, changed files, checks run, and the reviewer attention points from this note — never a Claude session link. Do not merge and do not delete branches; the merge is Jason's after his own read of the diff.
- Do not edit `docs/**` (this note, the roadmap, and the review Log are lead-maintained) and do not create GitHub issues; report deferred items in the final Runner message.
- Final Runner message: branch, PR URL, changed files, checks run, the attention points for Jason's review, and anything deferred.

## Stage 1 — Boundary reuse

**Change.** `rebuild_for_preloaded` (`crates/pulse-engine/src/controller.rs:1343`) replaces `self.release_backend()?` with `self.stop_active()?`. Nothing else in production code changes.

**Why it is enough.** `stop_active` (`:1427`) stops the sink and puts the backend back into `self.backend`; `start_backend` (`:966`) then reaches `take_or_open_backend` (`:1032`), which returns the same box because the device and engine kind match, and calls `backend.start(next.format)` — `set_format` + `play` on both backends (`:289`, `:341`). `IntegerEngine::set_format` (`integer_engine.rs:269`) already handles a format change on a live engine: it stops and drops the sink, writes the new nominal rate, physical and virtual formats on the still-hogged device, and builds a fresh ring; `play` creates the new sink. `Engine::set_format` (`engine.rs:88`) does the equivalent on the universal path. Because `release_backend` is never called, `bit_perfect_active` and the volume state stay put, so no `BitPerfectStateChanged` / `VolumeStateChanged` flicker at the boundary, no hog gap, no restore-then-reapply of formats. The ring is already drained when the rebuild runs (gated on `backend_position >= fed_frames` in `pump`, `:1269`), so `RawSink::stop`'s drain discards nothing audible.

**Failure paths to keep.** If `start` fails after reuse, `start_backend`'s error arms already release the box (`release_backend_value`) and `pump`'s `?` reaches `fail()` (`:1410`), whose `release_backend` resets the flags and emits `Error`. On a universal-exclusive backend, `exclusive_start_can_fallback` (`:1705`) still routes to `start_shared_fallback`, which opens a fresh shared backend because the box was taken. Both must be covered by tests, not assumed.

**Tests** (`controller.rs` test module; `FakeLog` already counts `releases`, `stops`, `backend_starts`, `opened_devices`):

- Extend `format_mismatch_rebuilds_backend_and_still_advances` (`:4878`): assert `releases == 0`, `opened_devices.len() == 1`, `backend_starts == [TEST_FORMAT, ALT_FORMAT]`, `stops >= 1`, and that no `BitPerfectStateChanged` or `VolumeStateChanged` event appears between `Playing` and `Advanced`.
- Add the `EngineKind::BitPerfect` variant of the same scenario: `BitPerfectStateChanged { active: true }` fires exactly once (at first play) and `VolumeStateChanged` does not repeat across the boundary; `releases == 0`.
- Add a start failure after reuse (`fail_bitperfect_start_device` set after the first start, before the boundary): playback ends in `Error`, `releases == 1`, `BitPerfectStateChanged { active: false }` is emitted, and the backend slot is empty afterwards.
- Add the universal-exclusive variant with `fail_exclusive_start_device`: `ExclusiveModeFallback` fires and playback continues on the shared backend (`backend_starts` shows the fallback start, `engine_kinds` shows the shared kind).

**Reviewer attention points.** The `expect("active playback must have a backend")` in `stop_active` holds because `pump` runs only with `active` and `backend` set. The same-format seamless path (`begin_seamless_transition`) and `finish_playback` / `fail` are untouched. `retains_device_when_paused` is unaffected. Nothing in the app crate changes.

**Jason's hardware acceptance after merge.** Matrix Mini-i Pro 4, display visible: a 44.1/16 track, then 96/24, then DSD64, queued as one album — one rate change per boundary, no flash back to the pre-Pulse format, the bit-perfect indicator steady, DSD readout on the third track; after Stop, Audio MIDI Setup shows the original format restored.

## Stage 2 — AUTO gate parity

**Change.** One predicate decides whether an `AudioStreamBasicDescription` is an integer wire format the engine will accept; the probe, the stored capabilities, the AUTO resolver, and the Devices page tag all read it.

- `crates/pulse-engine/src/hal.rs`: new `pub(crate) fn is_integer_wire_format(format: &AudioStreamBasicDescription) -> bool` — linear PCM, signed integer, not float, non-mixable, little-endian, interleaved, `mBitsPerChannel > 0` and a multiple of 8, `mChannelsPerFrame > 0`, `mBytesPerFrame` divisible by `mChannelsPerFrame`, container (`mBytesPerFrame / mChannelsPerFrame`) ≤ 4. `integer_candidate` (`integer_engine.rs:479`) calls it and keeps only the source-dependent checks (channel count, `mBitsPerChannel ≥ source width`, container ≥ source bytes, rate). `maximum_physical_format_capabilities` (`hal.rs:925`; callers at `:595`, `:1208`, `:1227`) additionally reports whether any available physical format passes the predicate. `max_bits_per_channel` keeps its meaning — it is the capability line and the Exclusive gate, and the DELLs' "24-bit integer" is true for AUHAL exclusive.
- `crates/pulse-engine/src/device.rs:28`: `OutputDeviceCapabilities` gains `integer_wire_formats: bool`.
- `crates/pulse-app/src/backend/settings.rs:139`: `StoredDeviceCapabilities` gains `integer_wire_formats: Option<bool>` (serde `integerWireFormats`); `is_complete` requires it, so stored data without the field reprobes exactly as `transport` did; `supports_bit_perfect` requires `Some(true)`.
- `crates/pulse-app/src/backend/playback/logic.rs:324` `automatic_output_mode`: Bit-perfect requires `integer_wire_formats` (and the transport gate as today); Exclusive still keys on `max_bits_per_channel.is_some()`. `automatic_stored_output_mode` (`:341`) follows through `supports_bit_perfect`. Wherever `bit_perfect_available` is derived (`logic.rs:398`, `devices.rs:208`) reads the new field so the `NO INTEGER PATH` tag (`ui/surfaces.rs:155`) is honest.

**Tests.** A predicate table over the stage-1 probe flag words: `0x54` and `0x4c` pass; `0x14`, `0x0c`, `0x04` fail (no `IsNonMixable`); a big-endian variant, a non-interleaved variant, and a 5-byte container fail. Probe: integer formats present but none non-mixable → `max_bits_per_channel` still `Some`, `integer_wire_formats == false`. Resolver: USB transport, integer formats present, no wire format → `Exclusive`, not `BitPerfect`. Settings: a stored entry without `integerWireFormats` → `stored_capabilities` returns `None` (sibling of `stored_capabilities_without_transport_require_a_reprobe`, `settings.rs:867`); with `false` → not bit-perfect. Existing hal tests at `:1208` / `:1227` updated for the new return shape.

**Reviewer attention points.** Old `settings.json` files must load (missing field → reprobe, never a parse error). The Matrix (`0x54`) and every device in the stage-1 probe table resolve exactly as before. `integer_candidate` must accept and reject exactly the same formats as today — the predicate is an extraction, not a behavior change on the engine side.

**Jason's check after merge.** Devices page: the Matrix still shows Bit-perfect under AUTO; a DELL display shows `NO INTEGER PATH`; a fresh `settings.json` round-trips the new field.

## Stage 3 — Stream-indexed IOProc fill

**Change.** The IOProc fills only the negotiated stream's buffer; every other stream is silenced and never counted.

- `crates/pulse-engine/src/integer_engine.rs`: `select_integer_format` returns `(stream_index, stream_id, format)`, the index being the stream's position in `hal::output_streams` (output scope, `hal.rs:739`). `set_format` stores it beside `device_format`; `play` passes it to `RawSink::start`.
- `crates/pulse-engine/src/raw_sink.rs`: `RawSink::start` takes `stream_index: usize` into `CallbackContext`. `output_io_proc` becomes: for `index == stream_index`, pop the ring into the buffer, zero-fill any shortfall, count shortfall or a null `mData` as underrun, and advance `position_frames` from that buffer only; for every other index, `fill(0)` when `mData` is non-null and count nothing. One integer compare per buffer; no allocation, lock, or syscall added.
- **Stream usage is mandatory, not optional.** Between `AudioDeviceCreateIOProcID` and `AudioDeviceStart`, set `kAudioDevicePropertyIOProcStreamUsage` (output scope) with `mIOProc` = the IOProc ID, `mNumberStreams` = the output stream count, `mStreamIsOn[i] = (i == stream_index)`; a failed write is an error, not a warning. Reason: `AudioHardware.h` documents the null-buffer contract only for streams marked off through this property, and the buffer-list order matching `kAudioDevicePropertyStreams` is convention (mpv relies on it too), not text. Also read `kAudioDevicePropertyStreamConfiguration` (output scope) at sink start and refuse with `UnsupportedFormat` if its `mNumberBuffers` differs from the stream count, so a mismatch is an error before the first callback rather than audio on the wrong output.
- The `FormatRestoreGuard` already captures every output stream; no change.

**Tests.** Extend `io_proc_copies_ring_bytes_zero_fills_and_counts_frames` (`raw_sink.rs:217`) with a two-buffer list (a local `#[repr(C)]` struct with two `AudioBuffer`s, because the crate's `AudioBufferList` type has a one-element array): selected index 1 → buffer 0 zeroed, buffer 1 carries the ring bytes, position and underrun count buffer 1 only; buffer 0 null → nothing added to underrun; selected buffer null → its byte size counts as underrun. `select_integer_format` index selection through the `FormatProperties` seam (`hal.rs:143`) if it reaches; otherwise a unit test on the ranking helper with the index threaded.

**Reviewer attention points.** The realtime rules (no allocation, no lock, no syscall in the callback). The `AudioHardwareIOProcStreamUsage` layout — a flexible `mStreamIsOn[1]` tail, so the property value is a hand-built byte buffer of header plus `n` `u32`s. Single-stream devices behave exactly as today (index 0, one buffer). Failure of the usage write or the configuration check must release cleanly through the existing `play` error arm (ring reset).

**Jason's check after merge.** Any two-output-stream device if one is at hand (an interface with separate stereo pairs, or an aggregate): audio only on the selected pair, silence on the other, no dropout counter movement. On the Matrix: no change.

## Stage 4 — Minor items

- **Honest hog errors.** `EngineError::Hogged(pid)` stays for "another process holds it". Two new variants in `crates/pulse-engine/src/error.rs`: one for "the hog write was accepted but the device stayed free" (virtual and aggregate devices; today `Hogged(-1)` from `HogGuard::acquire`, `hal.rs:100`) and one for "this process already holds the hog" (today `Hogged(own pid)` from `IntegerEngine::open`, `integer_engine.rs:246`). Both map to `PlaybackErrorKind::Device { hog_pid: None }` (`event.rs:48`) so the app's `handle_device_failure` (`queue_control.rs:377`) stops printing a pid that is not another process. Display strings say what happened.
- **Pump cadence.** `FEED_RETRY_DELAY` (`controller.rs:20`) 2 ms → 10 ms. Check the controller tests' `wait_for` budgets still hold; nothing else changes.

**Tests.** The two hog cases through the `HogGuard` seam or a unit test on the mapping in `event.rs`; the existing controller suite green at the new delay.

**Reviewer attention points.** No change to `HogGuard::drop` ownership logic. Error text is user-visible — read it as a user would.

## Stage 5 — Mono sources (decision pending)

Found in the 2026-09-03 double-check. `integer_candidate` requires `mChannelsPerFrame == source.channels`, the decoder passes the file's channel count through unchanged, and nothing upmixes, so a mono FLAC under Bit-perfect fails with `NoMatchingPhysicalFormat` on every play and a stereo→mono boundary mid-album fails inside `rebuild_for_preloaded`; there is no fallback for the `BitPerfect` kind. The universal path plays it (`hal.rs:907` accepts ≥ channels).

**My pick: dual-mono in the packer.** Accept a 2-channel integer format for a 1-channel source (ranking still prefers an exact channel match), give `IntPacker` an `output_channels` and write each mono sample into both slots — a copy, not arithmetic, so every channel's samples are untouched and the bit-perfect claim holds per channel. Tests: the packer duplicates; the candidate accepts stereo for mono and still rejects every other mismatch; a mono→stereo boundary rebuilds without error. The alternative is to keep refusing and route mono tracks to the universal engine, which loses the integer path for material that any DAC would play as dual-mono anyway. Jason decides at the stage 4 review; the mission is not fired before then.

## Mission goal template

Each stage's mission goal is the "Rules for every stage" section plus the stage section, verbatim, with these pointers: spec `docs/features/78-integer-engine-hardening.md`, review `docs/impls/78-integer-engine-review.md`, this note, and `AGENTS.md`. Title: `78 · stage <n> — <name>`. Crew: `codex-crew` (codex coder, Claude reviewer).

## Log

- **2026-09-03** — Review double-checked against the code at `b5f6ac1`: every claim and citation confirmed, `cargo test -p pulse-engine` 138 green; two additions recorded in the review's Log (mandatory `IOProcStreamUsage`, mono refusal). Plan written; PR-per-stage with Jason's personal review agreed. The `arch/pulse-engine.md` pause-lock sentence rewritten to cite feature 71's pending observation (pulled out of stage 4 as a lead doc edit).
