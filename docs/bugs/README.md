# Bug Notes

One note per open bug, written with the `bug` skill's template (description, expected, repro, relevant code, environment, verification) and a priority. Bugs are ordered into milestones in [`docs/roadmap.md`](../roadmap.md); a fixed bug's note moves to [`archive/`](archive/) with a line naming the commit. Nothing here is mirrored to GitHub — since 2026-08-29 this folder is the tracker.

## Open

- [`builtin-speakers-exclusive-volume.md`](builtin-speakers-exclusive-volume.md) - P2. Hogged devices: Pulse now drives the hardware volume control (`38734b8`); awaiting hardware validation.

## Fixed

- [`archive/seek-lands-early.md`](archive/seek-lands-early.md) - P2. Decode-forward after the seek; `a9dbef5`.
- [`archive/flaky-stall-watchdog-test.md`](archive/flaky-stall-watchdog-test.md) - P2. Injected clock for the engine worker; `a9dbef5`.
- [`archive/stale-row-selection-after-advance.md`](archive/stale-row-selection-after-advance.md) - P2. Library row selection follows playback; `cf02027`.
- [`archive/macos-deployment-target.md`](archive/macos-deployment-target.md) - P2. Floor raised to macOS 13.0 instead of validating 12; `84cf129`.
- [`archive/album-card-image-clipping.md`](archive/album-card-image-clipping.md) - P3. Radius on the cover `img`; `84cf129`.
- [`archive/track-artist-sql-copies.md`](archive/track-artist-sql-copies.md) - P3. One `EFFECTIVE_TRACK_ARTIST_SQL`, gated; `84cf129`.
- [`archive/app-menu-items-disabled.md`](archive/app-menu-items-disabled.md) - P2. App menu items and Edit ▸ Search enabled without focus; Cmd-M / Cmd-W fixed alongside; `f51accf`.
- [`archive/playback-row-transport-feedback.md`](archive/playback-row-transport-feedback.md) - P2. Transport hover/pressed feedback, prev/next availability from the queue, stable play button; `3d85c64`.
- [`archive/effective-artist-sql-copies.md`](archive/effective-artist-sql-copies.md) - P3. One shared identity fragment; feature 25 phase 2, `aa211ec`.
