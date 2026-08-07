# 0015 — MVP states and failure handling

> Stage 13. All MVP *features* are built (stages 1–12 merged). This stage closes the gap between "works when everything is fine" and "shippable": no silent failure, no dead-end state. Narrower than the roadmap row implies, because stage 11's Storage work already delivered several of the listed states.

## Already done — do not rebuild

Delivered by stages 11–12 and the #16 fixes: empty/first-run states (Albums, Playlists, Storage), offline storage roots, scan failure with inline Retry, scan progress, device-unavailable fallback to system default, saved-device-missing messaging, and the float-only capability line.

## Actually remaining

Four gaps, each needing a Pencil pass before implementation (standing roadmap rule: no frontend surface before a design exists).

1. **Loading states.** The library opens against SQLite synchronously and the v1→v2 backfill runs at startup — on the full NAS library that first launch can block visibly (flagged in the 0013 log). Needs: an app-open state while the store/backfill resolves, and a list-level state while a page query runs. A determinate progress form already exists in the scanning design (`NK6lk`) — derive from it rather than inventing a spinner language.
2. **Decode failure.** `EngineError::Decode` exists and surfaces as raw text in the playback row's error line. Needs a designed presentation naming the file, and a queue decision: skip to the next track and report, rather than halting the queue. Skipping is the right behavior — one corrupt file must not end playback — but it must be visible, not silent.
3. **Hogged / unavailable device during playback.** `EngineError::Hogged(pid)` exists. Today's copy is developer-facing ("device hogged by pid 42"). Needs plain-language presentation plus a recovery path (retry, or switch output), and correct handling when the active device disappears mid-playback rather than at launch.
4. **Missing file.** A library row whose file has been moved or deleted between scans. Playback must fail gracefully with a designed state, and the row should be visually marked. Decide and log: mark-on-play-failure only, or a cheap existence check at play time. Do not delete rows — that would collide with the offline-root guarantee, where files are legitimately absent.

## Decisions (2026-08-07, during implementation)

- Page queries stay synchronous on the UI thread; the query-time list loading state from item 1 is therefore dead by construction and only the app-open state exists. Rationale, measurements, and escape hatches: [`../arch/thread-model.md`](../arch/thread-model.md).
- Playback errors carry a structured `PlaybackErrorKind` on the event (Option A) rather than the app parsing display strings; auhal start failures got their own `EngineError::AudioUnit` variant in the process.
- Missing-file handling: cheap existence check at play time plus mark-on-play-failure. Marks clear on root removal and after a scan that actually verified presence (completed, removals not suppressed) — cancelled, offline, or failed scans keep them. Rows are never deleted, per the offline-root guarantee.

## Scope discipline

- No new engine error variants unless a gap is proven; this is mostly presentation plus queue behavior.
- Keep the error language consistent with what shipped: plain sentences, `danger` for genuine failures, `warning`/neutral for expected conditions. The #11 lesson — do not style an expected condition as an error.
- Missing-file and decode-failure both interact with the queue (`crates/pulse-app/src/queue.rs`); the advance-on-failure logic belongs in the queue view-model with unit tests, not scattered in views.

## Phases

1. Pencil pass for the four states above, in `design/pulse-desktop.pen`.
2. Queue-level failure behavior + view-model tests (skip-and-report, missing-file marking).
3. UI wiring for all four, plus loading states.

## Verification

- `make verify` green (105 passing + 2 ignored at spec time; 130 by the end of the stage-13 build).
- Forced-failure fixtures without committing binaries: truncate a generated WAV in a temp dir for decode failure; delete a file after scanning for missing-file; both are unit-testable at the queue/view-model layer.
- Manual: play a deliberately corrupted file, unplug the DAC mid-playback, hog the device from another process, rename a file after scanning, and cold-launch against the full NAS library to see the loading state.

## Then stage 14 (v0 release)

Hardware smoke matrix on the Matrix Mini-i Pro 4, packaging the `.app` (icon asset already added in stage 12), release notes, and a final design-vs-implementation cleanup pass. The [`MANUAL_ACCEPTANCE_CHECKLIST.md`](MANUAL_ACCEPTANCE_CHECKLIST.md) is the acceptance gate for both stages — its 96 checks should be walked before tagging v0.
