# Feature Specs

Product features that are not part of a numbered roadmap stage. Roadmap stages live in [`docs/impls/`](../impls/); this folder is for features discovered outside that sequence — usually during acceptance passes.

Each spec is `{number}-{slug}.md` and states motivation, scope, non-goals, phases, and verification. Numbering is global and never reused; implemented specs move to [`archive/`](archive/).

## Active

- [`20-now-playing-page.md`](20-now-playing-page.md) - P2, issue #56. Click the playback row's track cluster to open an immersive now-playing view: full-size album art, track identity, quality badge; playback row stays as the transport. Lyrics land here later.
- [`19-mcp-server.md`](19-mcp-server.md) - P2, issue #55. `pulse mcp` stdio server on the app binary, Quill-style: library reads always on; playlist CRUD and rescan behind a Settings write gate; WAL for cross-process safety.
- [`17-playback-dropout-reporting.md`](17-playback-dropout-reporting.md) - P2, issue #50. Read side for the callback's write-only underrun counter: dropout events through the controller, startup-priming excluded, surfaced in the app.
- [`16-gapless-playback.md`](16-gapless-playback.md) - P1, issue #46. Keep the AUHAL sink and ring alive across same-format track transitions via controller next-track lookahead, flipping NowPlaying at the audible boundary; format changes keep the rebuild path.
- [`11-artists-page.md`](11-artists-page.md) - P2, issue #38. Top-level Artists index (effective-album-artist identity, counts, type-to-filter) whose rows open Albums filtered by that artist; reverses mvp.md's no-artists-page scope cut.
- [`09-collapsible-library-sidebar.md`](09-collapsible-library-sidebar.md) - P3, issue #34. Design and implement the compact state behind the library sidebar's existing Pencil collapse affordance.

## Implemented

- [`18-polite-shared-mode.md`](archive/18-polite-shared-mode.md) - P1, issue #51. Shared mode stops switching the device's nominal rate (a system-wide side effect that left AirPods-class devices unable to play even CD-rate tracks) and lets AUHAL resample; native-rate switching became exclusive-mode behavior.
- [`15-in-app-update-install.md`](archive/15-in-app-update-install.md) - P2, issue #52. Adopt Sparkle 2 as the updater, retiring the hand-rolled checker: EdDSA-signed appcast on GitHub Releases, inside-out signing for the embedded framework, standard Sparkle UI in v1. Also gives the DMG an Applications drag target.
- [`14-activity-brand-mark.md`](archive/14-activity-brand-mark.md) - P2, issue #43. Swap the hand-drawn waveform for the lucide `activity` glyph across sidebar, settings tiles, and the dock icon — one mark everywhere; artwork only.
- [`13-progress-strip-playback-bar.md`](archive/13-progress-strip-playback-bar.md) - P2, issue #40. Qobuz-style bar anatomy: full-width 3px seek strip with a hover thumb, split compact times, and larger controls centered between symmetric side slots.
- [`12-sidebar-brand-header.md`](archive/12-sidebar-brand-header.md) - P3, issue #39. Runner-style sidebar header: flat brand glyph (no icon tile), larger wordmark, 56px headroom under the traffic lights.
- [`10-genre-filter-picker.md`](archive/10-genre-filter-picker.md) - P2. Move arbitrary embedded genre values out of the top-level chip row and into one searchable picker with album counts.
- [`08-per-device-exclusive-mode.md`](archive/08-per-device-exclusive-mode.md) - P2, issue #33. Exclusive mode becomes per-device: capability-probed defaults (integer-format DACs → exclusive, float-only Bluetooth → shared), overrides set from an Auto-aware control in the output device popover (the global Settings toggle is deleted), and exclusive-failure fallback to shared with a notice.
- [`07-shuffle-repeat.md`](archive/07-shuffle-repeat.md) - P2, issue #32. Wire the deliberately inert Shuffle and Repeat controls through the queue, album, and playlist surfaces.
- [`06-settings-surface.md`](archive/06-settings-surface.md) - P1, issue #30. Gives the sidebar's inert Settings entry a destination: a dedicated page whose sidebar is the section nav, with General / Update / About pages. Added Pulse's first toggle component.
- [`05-update-check.md`](archive/05-update-check.md) - P2, issue #27. Check GitHub Releases on launch and notify through the existing notice banner. Superseded by feature 15 (Sparkle).
- [`04-dev-prod-data-isolation.md`](archive/04-dev-prod-data-isolation.md) - P1, issue #26. Debug builds resolve `pulse-dev` instead of `pulse` so `cargo run` can never open or migrate the released app's library.
- [`03-queue-popover.md`](archive/03-queue-popover.md) - P1. The queue button's missing hit target: anchored popover with NOW PLAYING, jumpable UP NEXT rows, remove and Clear.
- [`02-macos-packaging.md`](archive/02-macos-packaging.md) - P0, issue #19. Hand-rolled `.app` bundling, Developer ID signing, notarization, DMG, and tag-triggered release CI. Bundle identifier `com.wycstudios.pulse`.
- [`01-volume-control.md`](archive/01-volume-control.md) - P1, issue #17. Software gain in the engine's float32 render path with a unity default, mute, and a playback-row slider. Hardware/device volume and a fixed-output purist mode remain future work.
