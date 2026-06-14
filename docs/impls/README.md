# Implementation Notes

Implementation notes are tactical plans for concrete build slices. Architecture docs explain the system shape; impl notes explain how a specific change should land, what files it touches, and how to verify it.

Active notes:

- [`ROADMAP.md`](ROADMAP.md) - canonical implementation stage order.
- [`0001-engine-validation-cli.md`](0001-engine-validation-cli.md) - first stage: prove `pulse-engine` through `pulse-cli`.
- [`0002-hal-format-validation.md`](0002-hal-format-validation.md) - second stage: prove hog mode and physical format switching.
- [`0003-auhal-playback.md`](0003-auhal-playback.md) - third stage: AUHAL playback from decoded PCM.
- [`0004-cli-config.md`](0004-cli-config.md) - fourth stage: persisted CLI defaults for playback harness ergonomics.
- [`0005-product-design.md`](0005-product-design.md) - fifth stage: Pencil-led product design before frontend or app-settings backend implementation.
