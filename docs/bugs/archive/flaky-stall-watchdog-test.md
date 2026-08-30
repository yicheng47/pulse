# Flaky engine test: stall watchdog test races the CI runner's clock

> Bug · P2 · filed 2026-08-30 from CI (runs 33285256407, 33285876602, 33285937067 on `main`). Local note only — no GitHub issue. **Fixed** 2026-08-30, commit `a9dbef5` — the worker takes an injected clock and the stall tests drive fake time.

## Description

`controller::tests::continuous_progress_across_seamless_boundary_does_not_stall` (added by feature 16 phase 2) fails intermittently on the GitHub macOS runner with "continuous backend progress stalled: timed out waiting for audio output progress" (`crates/pulse-engine/src/controller.rs` ~2183). The test advances the fake backend's position by 500 frames after each `sleep(TEST_STALL_TIMEOUT / 4)` (25 ms) while the worker's output-stall watchdog is set to 100 ms. On a loaded runner a single >100 ms gap between two bumps arms and fires the watchdog. Three consecutive docs-only pushes to `main` went red; the same commits pass locally.

## Expected Behavior

Engine tests must not depend on the test thread keeping pace with wall-clock time. The worker takes an injected clock (`Instant::now` in production); the stall tests advance a fake clock explicitly and assert on fake time.

## Relevant Code

- `crates/pulse-engine/src/controller.rs` — `pump`'s stall watchdog (`stalled_since`, `output_stall_timeout`), `WorkerSettings`, `spawn_with_dependencies`; the tests `continuous_progress_across_seamless_boundary_does_not_stall`, `stalled_output_at_seamless_boundary_still_times_out`, and the three older `assert_no_error_for` stall tests (~1296–1405).

## Environment

- CI: GitHub Actions `macos` runner; local runs green.
- Pulse version: `main` at `df3c280`.

## Verification

Three CI logs show the same single failing test; the assertion is the watchdog's timeout error surfacing as `PlaybackEvent::Error`.
