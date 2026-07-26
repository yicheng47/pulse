# Implementation Notes

Implementation notes are tactical plans for concrete build slices. Architecture docs explain the system shape; impl notes explain how a specific change should land, what files it touches, and how to verify it.

Notes stay here while their stage is in flight. Once a stage ships, its note moves to [`archive/`](archive/) keeping its number, so the directory listing answers "what is being worked on right now". Enduring decisions should not be left behind in an archived note; fold them into `docs/arch/` before archiving.

Active notes:

- [`ROADMAP.md`](ROADMAP.md) - canonical implementation stage order.
- [`0005-product-design.md`](0005-product-design.md) - fifth stage: Pencil-led product design. Partially complete; more surfaces still need design passes.
- [`0007-gpui-native-ui-pivot.md`](0007-gpui-native-ui-pivot.md) - app shell pivots from Tauri 2 + React to a native GPUI `crates/pulse-app`. Stage 7 is in flight.
- [`0008-gpui-ce-dependency.md`](0008-gpui-ce-dependency.md) - swap the abandoned crates.io `gpui` for rev-pinned GPUI-CE; carries live risks for stage 7.

Archived notes:

- [`archive/0001-engine-validation-cli.md`](archive/0001-engine-validation-cli.md) - first stage: prove `pulse-engine` through `pulse-cli`.
- [`archive/0002-hal-format-validation.md`](archive/0002-hal-format-validation.md) - second stage: prove hog mode and physical format switching.
- [`archive/0003-auhal-playback.md`](archive/0003-auhal-playback.md) - third stage: AUHAL playback from decoded PCM.
- [`archive/0004-cli-config.md`](archive/0004-cli-config.md) - fourth stage: persisted CLI defaults for playback harness ergonomics.
- [`archive/0006-playback-controller.md`](archive/0006-playback-controller.md) - sixth stage: UI-agnostic playback controller inside `pulse-engine`.
