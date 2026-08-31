# Stage 1 — probe

The milestone gate: a dev-only example that answers, per device, whether current macOS accepts an integer **virtual** format while hogged. No engine code exists until this stage's findings are archived in [feature 32](../../features/32-integer-engine.md).

Prerequisite reading pass: mpv `audio/out/ao_coreaudio_exclusive.c`, MPD `src/output/plugins/OSXOutputPlugin.cxx`, SFBAudioEngine, and hummingbird (`~/repos/audio/` — clone the missing ones). GPL sources are ideas only, no code.

## hal additions

Needed by the probe itself:

- `kAudioStreamPropertyVirtualFormat` + `kAudioStreamPropertyAvailableVirtualFormats` get/set wrappers (stream-scoped, same shape as the physical pair).
- `kAudioDevicePropertySupportsMixing` get/set (device-scoped; not every device has it — probe with `AudioObjectHasProperty` first).

## Probe algorithm

Deliverable: `crates/pulse-engine/examples/integer_probe.rs`. Per output device: capabilities → output streams → available physical formats; hog; **save** current physical format, virtual format, and mixing state; for each signed-integer physical candidate at 44.1/48/96/192kHz: set physical, attempt virtual = same ASBD, read both back, record accepted/refused with the full flags word (`IsSignedInteger`, `IsPacked`, `IsAlignedHigh` matter for the packer); **restore** everything; drop the hog. Print a table.

## Run matrix

Run on: the Matrix Mini-i Pro 4 (the target), built-in speakers (expected refusal — exercises the gating story; also answers whether they report integer *physical* formats, which decides feature 32's provisional capability gate), and the Lotoo if docked. Paste the findings table into feature 32's spec — the `IntPacker` (stage 3) and the capability gate (stage 4) are driven by the probed flags, not assumptions.

## Gate

If no device accepts an integer virtual format on current macOS, the milestone pivots per the [README](README.md)'s risk note: record the finding in feature 32 and stop.

## Verification

- `make verify` green.
- After a probe run, every device shows its prior formats and mixing state in Audio MIDI Setup, and the hog is released.
- Findings table archived in feature 32's spec before stage 3 starts.
