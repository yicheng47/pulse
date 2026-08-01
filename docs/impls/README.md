# Implementation Notes

Implementation notes are tactical plans for concrete build slices. Architecture docs explain the system shape; impl notes explain how a specific change should land, what files it touches, and how to verify it.

Notes stay here while their stage is in flight. Once a stage ships — or a decision note's decision is applied — it moves to [`archive/`](archive/) keeping its number, so the directory listing answers "what is being worked on right now". Enduring decisions should not be left behind in an archived note; fold them into `docs/arch/` before archiving.

Active notes:

- [`ROADMAP.md`](ROADMAP.md) - canonical implementation stage order.
- [`IMPL_LOG.md`](IMPL_LOG.md) - running progress record. Append an entry per working session: what happened, what's next, blockers.
- [`MANUAL_ACCEPTANCE_CHECKLIST.md`](MANUAL_ACCEPTANCE_CHECKLIST.md) - the human-only MVP acceptance gate; 96 checks across playback, hardware, library, queue, playlists, and search.
- [`0015-mvp-states-and-hardening.md`](0015-mvp-states-and-hardening.md) - stage 13: loading states, decode failure, hogged/unavailable device, and missing-file handling.

Archived notes:

- [`archive/0001-engine-validation-cli.md`](archive/0001-engine-validation-cli.md) - first stage: prove `pulse-engine` through `pulse-cli`.
- [`archive/0002-hal-format-validation.md`](archive/0002-hal-format-validation.md) - second stage: prove hog mode and physical format switching.
- [`archive/0003-auhal-playback.md`](archive/0003-auhal-playback.md) - third stage: AUHAL playback from decoded PCM.
- [`archive/0004-cli-config.md`](archive/0004-cli-config.md) - fourth stage: persisted CLI defaults for playback harness ergonomics.
- [`archive/0006-playback-controller.md`](archive/0006-playback-controller.md) - sixth stage: UI-agnostic playback controller inside `pulse-engine`.
- [`archive/0007-gpui-native-ui-pivot.md`](archive/0007-gpui-native-ui-pivot.md) - app shell pivoted from Tauri 2 + React to a native GPUI `crates/pulse-app`. Applied; rules now in [`arch/tech-stack.md`](../arch/tech-stack.md).
- [`archive/0008-gpui-ce-dependency.md`](archive/0008-gpui-ce-dependency.md) - swapped the abandoned crates.io `gpui` for rev-pinned GPUI-CE. Applied; rules now in [`arch/tech-stack.md`](../arch/tech-stack.md).
- [`archive/0005-product-design.md`](archive/0005-product-design.md) - Pencil-led product design baseline. Retired 2026-08-01 as too broad to close; every surface it named now exists.
- [`archive/0009-playback-row.md`](archive/0009-playback-row.md) - stage 7: playback row MVP.
- [`archive/0010-app-shell.md`](archive/0010-app-shell.md) - stage 7.5: sidebar, top bar, routed body, macOS menu.
- [`archive/0011-output-device-management.md`](archive/0011-output-device-management.md) - stage 8: device list, selection, UID persistence.
- [`archive/0012-library-scan-and-store.md`](archive/0012-library-scan-and-store.md) - stages 9-10: storage roots, scanner, SQLite store.
- [`archive/0013-library-ui.md`](archive/0013-library-ui.md) - stage 11: Albums, Album Detail, Tracks, Storage surfaces.
- [`archive/0014-queue-playlists-search.md`](archive/0014-queue-playlists-search.md) - stage 12: queue, playlists, search.
