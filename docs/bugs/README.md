# Bug Notes

One note per open bug, written with the `bug` skill's template (description, expected, repro, relevant code, environment, verification) and a priority. Bugs are ordered into milestones in [`docs/roadmap.md`](../roadmap.md); a fixed bug's note moves to [`archive/`](archive/) with a line naming the commit. Nothing here is mirrored to GitHub — since 2026-08-29 this folder is the tracker.

## Open

- [`builtin-speakers-exclusive-volume.md`](builtin-speakers-exclusive-volume.md) - P2. Exclusive mode on the built-in speakers disables the macOS volume control and freezes the hardware level.
- [`seek-lands-early.md`](seek-lands-early.md) - P2. Seek lands earlier than the requested position (coarse seek, no decode-forward).
- [`macos-deployment-target.md`](macos-deployment-target.md) - P2. Validate or raise the declared macOS 12 deployment target.
- [`app-menu-items-disabled.md`](app-menu-items-disabled.md) - P2. About / Settings… / Check for Updates… are disabled unless something in the window has focus; fix is global `cx.on_action` handlers.
- [`album-card-image-clipping.md`](album-card-image-clipping.md) - P3. Album card corners: put the radius on the image layer, as the artist avatar fix did.
- [`track-artist-sql-copies.md`](track-artist-sql-copies.md) - P3. The track-artist identity expression is written three times in `repo/tracks.rs`; hoist it beside `EFFECTIVE_ALBUM_ARTIST_SQL` and gate it.

## Fixed

- [`archive/playback-row-transport-feedback.md`](archive/playback-row-transport-feedback.md) - P2. Transport hover/pressed feedback, prev/next availability from the queue, stable play button; `3d85c64`.
- [`archive/effective-artist-sql-copies.md`](archive/effective-artist-sql-copies.md) - P3. One shared identity fragment; feature 25 phase 2, `aa211ec`.
