# Seamless In-Track Transport

Feature 76 · P2 · GitHub issue [#76](https://github.com/yicheng47/pulse/issues/76). Filed from Jason's report, 2026-09-02: seek within a song, and pause/resume, should not tear down and rebuild what they don't have to.

## Motivation

The transport paths already keep the right device state — hog is held via backend reuse (`take_or_open_backend`), and `set_format` early-returns on an unchanged format, so there is no rate switch or re-probe on a same-song seek. What churns is everything else. A seek while playing stops the IOProc (`stop_active` → `AudioDeviceStop`), throws away the live decoder, builds a new one from the factory — reopen the file, reparse the container, then seek — and restarts the device. Pause does the same teardown (`stop_active` drops the decoder), so resume also pays the factory-fresh decoder plus a full `start_path`. On NAS files the reopen+reparse is the dominant transport latency, and the device stop/start is the audible blink.

`RawSink::stop` drains the ring, so the current design is correct — no stale-audio burst — just heavier than needed.

## Scope

- **In-place seek (playing, same track)**: keep the sink running; call `seek()` on the active decoder instead of discarding it; flush the ring from the producer side; rebase position accounting (`base_position_ms`, `fed_frames`, backend-position deltas — the sink's frame counter does not reset); the intentional refill gap is not counted as a dropout (feature 17's "only while data is owed" seam is the hook). The IOProc keeps running and zero-fills until data lands.
- **Pause keeps the decoder**: `pause` on a device-retaining backend stops the IOProc as today but keeps the active decoder and its position state alive. `resume` restarts the sink (`play()` → `restart()`) and continues feeding — no factory call, no reopen, no re-seek.
- **Seek while paused**: seeks the retained decoder in place (the `prepared_decoder` pre-seek machinery collapses into this).
- **Fallback**: any in-place failure (decoder seek error, backend gone) falls back to the current full-rebuild path — the rebuild is demoted from the only path to the recovery path.
- DoP note: zero-filled frames carry no markers, so DSD still drops and re-locks across a seek gap — physics, unchanged, accepted per feature 71. Pause/resume on DoP keeps its current hold behavior.

## Non-Goals

- Cross-track seeks and track changes — the existing start/gapless paths stay.
- Format-changing transitions — full rebuild remains correct there.
- No change to pause's device semantics (hold-on-pause stays per feature 32; the manual-release affordance is feature 77).
- No universal-engine (AUHAL) behavior redesign; it takes the shared in-place path only where it already fits.

## Implementation Phases

1. **Pause/resume keeps the decoder**: split `stop_active` so pausing a device-retaining backend parks the active playback (decoder + accounting) instead of dropping it; resume restarts the sink and continues. Fallback to rebuild on any error.
2. **In-place seek**: producer-side ring flush seam on the backend, in-place decoder seek for playing and paused states, position rebase, dropout suppression for the refill gap.

## Verification

- `make verify` green.
- Fake-seam tests (the feature 16/17 fakes): resume after pause performs zero decoder-factory calls; a playing seek performs zero factory calls and zero backend stop/start calls; position events rebase correctly across seek; the refill gap adds nothing to `dropout_frames`; a decoder seek failure falls back to the rebuild path and still plays.
- Jason's ear test on the Matrix (later, not gating the merge): PCM seek has no blink; DSD seek re-locks as before; pause/resume unchanged audibly but faster to resume on NAS files.
