# Bug Notes

One note per open bug, written with the `bug` skill's template (description, expected, repro, relevant code, environment, verification) and a priority. Bugs are ordered into milestones in [`docs/roadmap.md`](../roadmap.md); a fixed bug's note moves to [`archive/`](archive/) with a line naming the commit. Nothing here is mirrored to GitHub — since 2026-08-29 this folder is the tracker.

## Open

- [`builtin-speakers-exclusive-volume.md`](builtin-speakers-exclusive-volume.md) - P2. Exclusive mode on the built-in speakers disables the macOS volume control and freezes the hardware level.
- [`seek-lands-early.md`](seek-lands-early.md) - P2. Seek lands earlier than the requested position (coarse seek, no decode-forward).

## Fixed

- [`archive/macos-deployment-target.md`](archive/macos-deployment-target.md) - P2. Floor raised to macOS 13.0 instead of validating 12; `84cf129`.
- [`archive/album-card-image-clipping.md`](archive/album-card-image-clipping.md) - P3. Radius on the cover `img`; `84cf129`.
- [`archive/track-artist-sql-copies.md`](archive/track-artist-sql-copies.md) - P3. One `EFFECTIVE_TRACK_ARTIST_SQL`, gated; `84cf129`.
- [`archive/app-menu-items-disabled.md`](archive/app-menu-items-disabled.md) - P2. App menu items and Edit ▸ Search enabled without focus; Cmd-M / Cmd-W fixed alongside; `f51accf`.
- [`archive/playback-row-transport-feedback.md`](archive/playback-row-transport-feedback.md) - P2. Transport hover/pressed feedback, prev/next availability from the queue, stable play button; `3d85c64`.
- [`archive/effective-artist-sql-copies.md`](archive/effective-artist-sql-copies.md) - P3. One shared identity fragment; feature 25 phase 2, `aa211ec`.
