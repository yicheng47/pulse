# Seek lands earlier than the requested position (coarse seek, no decode-forward)

> Bug · P2 · filed 2026-08-16 as GitHub issue #49 (closed when tracking moved to docs, 2026-08-29)

## Description
`PcmDecoder::seek` uses Symphonia's `SeekMode::Coarse`, which lands on the nearest seek point at or before the target, and Pulse never decodes forward to the requested frame. The controller is honest about it — it reports the actual landing position, and the requested/actual bookkeeping prevents the error from compounding across pause/seek cycles — but the user experience is scrubbing to 1:30 and hearing playback start at, say, 1:27. The error magnitude depends on the file's seek-point spacing.

## Expected Behavior
Seek lands at the requested position (within one packet): coarse seek to the prior seek point, then decode and discard frames up to the requested time before any PCM reaches the ring.

## Steps To Reproduce
1. Play a long FLAC (a sparse seek table makes it worse).
2. Scrub to a precise position; compare where the UI position settles against the scrub target.

## Relevant Code
- `crates/pulse-engine/src/decode.rs:69` — `SeekMode::Coarse`, returns `actual_ts`, no decode-forward.
- `crates/pulse-engine/src/controller.rs:337` — seek paths consume the actual landing position; the requested/actual split works, the landing itself is early.

## Environment
- OS: macOS (Darwin 25.5)
- Device / DAC: any
- Input file format: FLAC/ALAC/AIFF/WAV; worst with sparse seek tables
- Pulse version: v0.1.2

## Verification
Code review. The test `paused_seek_resumes_from_the_original_target_without_compounding_seek_error` documents the early-landing model explicitly.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

