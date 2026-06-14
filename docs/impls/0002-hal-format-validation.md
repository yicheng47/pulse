# HAL Format Validation

> Second product stage: prove Pulse can own a Core Audio output device and put it into a file-compatible physical format before implementing playback.

## Context

Stage 1 proved that `pulse-cli` can enumerate devices and probe file format metadata. Stage 2 moves from inspection into device control: hog mode, nominal sample-rate switching, and physical output format selection.

This stage still does not play audio. That is intentional. If device ownership and format switching are wrong, playback would make the failure harder to isolate.

## Goal

Make this command work:

```sh
pulse-cli validate-format <file>
```

Expected result: Pulse takes hog mode, confirms the file's sample rate is supported, switches the device nominal sample rate, selects a matching integer physical stream format, reports the chosen stream format, then releases hog mode.

## Scope

- Acquire and release `kAudioDevicePropertyHogMode` safely.
- Query `kAudioDevicePropertyAvailableNominalSampleRates`.
- Set `kAudioDevicePropertyNominalSampleRate`.
- Query output streams via `kAudioDevicePropertyStreams`.
- Query stream formats via `kAudioStreamPropertyAvailablePhysicalFormats`.
- Set `kAudioStreamPropertyPhysicalFormat`.
- Poll until Core Audio reports the requested rate and physical format.

## Matching Rules

The first pass is intentionally conservative:

- Format ID must be linear PCM.
- Format must be integer, not float.
- Format must support the requested sample rate.
- Format must have at least the requested channel count.
- Format must have at least the requested bit depth.

The "at least bit depth" rule keeps 24-bit files compatible with devices that expose 24 meaningful bits in a 32-bit physical container. In the AUHAL stage this physical format is diagnostic/control-plane state; the callback itself receives float32 client buffers.

## Non-Goals

- No playback.
- No decoder loop.
- No JSON output yet.
- No Tauri UI.
- No automatic restoration of the previous device format.

## Verification

- `pulse-cli validate-format <file>` succeeds on a connected output device for supported PCM files.
- If another process owns hog mode, the command returns `Hogged(pid)`.
- If no physical format can carry the file, the command returns `NoMatchingFormat`.
- The command releases hog mode on return.
