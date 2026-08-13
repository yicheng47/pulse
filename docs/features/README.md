# Feature Specs

Product features that are not part of a numbered roadmap stage. Roadmap stages live in [`docs/impls/`](../impls/); this folder is for features discovered outside that sequence — usually during acceptance passes.

Each spec is `{number}-{slug}.md` and states motivation, scope, non-goals, phases, and verification.

- [`10-genre-filter-picker.md`](10-genre-filter-picker.md) - P2. Move arbitrary embedded genre values out of the top-level chip row and into one searchable picker with album counts.
- [`09-collapsible-library-sidebar.md`](09-collapsible-library-sidebar.md) - P3, issue #34. Design and implement the compact state behind the library sidebar's existing Pencil collapse affordance.
- [`08-per-device-exclusive-mode.md`](08-per-device-exclusive-mode.md) - P2, issue #33. Persist exclusive/shared playback mode by Core Audio device UID instead of one global toggle.
- [`07-shuffle-repeat.md`](07-shuffle-repeat.md) - P2, issue #32. Wire the deliberately inert Shuffle and Repeat controls through the queue, album, and playlist surfaces.
- [`06-settings-surface.md`](06-settings-surface.md) - P1. Gives the sidebar's inert Settings entry a destination: a dedicated page whose sidebar is the section nav, with General / Update / About pages. Designed; adds Pulse's first toggle component.
- [`05-update-check.md`](05-update-check.md) - P2. Check GitHub Releases on launch and notify through the existing notice banner. GitHub stays the artifact host; self-installing updates (Sparkle) are deliberately deferred.
- [`04-dev-prod-data-isolation.md`](04-dev-prod-data-isolation.md) - P1. Debug builds resolve `pulse-dev` instead of `pulse` so `cargo run` can never open or migrate the released app's library.
- [`03-queue-popover.md`](03-queue-popover.md) - P1. The queue button's missing hit target: anchored popover with NOW PLAYING, jumpable UP NEXT rows, remove and Clear.
- [`02-macos-packaging.md`](02-macos-packaging.md) - P0, issue #19. Hand-rolled `.app` bundling, Developer ID signing, notarization, DMG, and tag-triggered release CI. Bundle identifier `com.wycstudios.pulse`.
- [`01-volume-control.md`](01-volume-control.md) - P1, issue #17. Software gain in the engine's float32 render path with a unity default, mute, and a playback-row slider. Hardware/device volume and a fixed-output purist mode are explicitly future work. Blocked on a Pencil pass for the playback bar.
