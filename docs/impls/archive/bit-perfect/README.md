# Bit-perfect engine (M3 → v0.3.0)

The staged implementation of [feature 32](../../../features/archive/32-integer-engine.md), with [feature 31](../../../features/archive/31-volume-transparency.md) (volume transparency) beside it and [feature 71](../../../features/71-dsd-over-dop.md) (DSD over DoP) layered on top after acceptance. This directory supersedes the single-file plan 0016 — the milestone is big enough to run stage by stage, each stage a doc and a mission. The journey is logged in [`IMPL_LOG.md`](IMPL_LOG.md).

## Goal

A second playback sink: raw HAL `AudioDeviceCreateIOProcID` with hog, mixing disabled, integer physical format, and — the step 0003 missed — **virtual format set equal to the integer physical format**, so the IOProc buffer takes source integers directly. Selected per device (feature 08 pattern), proven by the DoP test on the Matrix.

## Prior art in this repo

- `../archive/0002-hal-format-validation.md` proved hog + physical-format switching; that machinery is today's `hal.rs` (`HogGuard`, `set_nominal_sample_rate`, `set_matching_physical_format`) and is reused wholesale.
- `../archive/0003-auhal-playback.md` records the raw-HAL noise failure that forced the AUHAL pivot. Root cause in hindsight: integer bytes written into a float32 **virtual** format buffer. This milestone's whole premise is fixing that properly instead of avoiding it.
- The controller already abstracts the engine: `trait PlaybackBackend { start, feed, position, underrun_frames, take_hardware_volume, set_volume, stop }` behind a `BackendFactory` (`controller.rs:127`). The integer engine is a second implementor; the state machine, queue, and event logic are untouched.

## References

Read-only clones in `~/repos/audio/`: `mpv`, `MPD`, and `SFBAudioEngine` still need cloning (not present as of 2026-08-31 — clone before the stage 1 reading pass); `hummingbird/`, already there, is a fourth reference — a native macOS bit-perfect player. GPL sources are ideas only, no code.

## Process

- Each stage runs as a Runner **peer-coding mission with codex-crew** — one mission at a time, in stage order; the next stage's mission starts only after the prior stage merges.
- Stage 1 is the milestone gate: if no device accepts an integer virtual format, the milestone records the finding in feature 32 and stops (see Risks).
- Stage 4's Devices-page control requires the Pencil design pass approved by Jason **before** its mission starts.
- Merges follow the direct-merge conventions; after each stage the lead appends the outcome to [`IMPL_LOG.md`](IMPL_LOG.md). Crews report in their Runner mission and do not edit the log.

## Stages

- [`stage-1-probe.md`](stage-1-probe.md) — hal virtual-format/mixing wrappers + the `integer_probe` example; hardware findings recorded in feature 32. **The gate.**
- [`stage-2-hal-guards.md`](stage-2-hal-guards.md) — `FormatRestoreGuard` (RAII restore of physical + virtual + mixing state).
- [`stage-3-engine.md`](stage-3-engine.md) — `RawSink`, `IntegerEngine`, `IntPacker`; controller `EngineKind` and pause retention.
- [`stage-4-app.md`](stage-4-app.md) — settings, capability plumbing, Devices-page Engine control, feature 31 indicator states.
- [`stage-5-acceptance.md`](stage-5-acceptance.md) — DoP payload prep and the hardware acceptance pass on the Matrix.

## Risks

- **Modern macOS may refuse integer virtual formats** (Apple has narrowed hog-mode privileges before). Stage 1 answers cheaply; if refused everywhere, the milestone records the finding in the spec and stops — the honest outcome is "not possible on this OS," not a float engine wearing the label.
- IOProc buffer-list semantics (multiple streams, non-interleaved layouts, variable frame counts) — mpv's callback is the reference for the edge cases.
- Restore-on-kill: the guard cannot run on SIGKILL/crash. macOS releases hog on process death; formats may persist — document, and consider a launch-time restore of any device Pulse left configured.
- Alignment flags vary by device; the packer must be driven by the probed flags, not assumptions.
- The `SetOutputDevice` protocol change ripples through app dispatch and the controller test suite; budget for test updates.
