# Engine Validation CLI

> First product stage: prove the playback engine before building product UI.

## Context

Pulse's first real risk is not React, Tauri, library scanning, or visual polish. The hard part is whether a Rust engine can drive a real macOS output device in hog mode, switch to the source file's native sample rate and physical integer format, and feed the DAC without Core Audio inserting conversion.

This needs to be proven before the app shell gets serious. If the engine cannot be validated from a small CLI, the Tauri app would only hide the failure behind more moving parts.

`pulse-cli` exists for this reason. It is a development and validation harness, not a user-facing product surface.

## Goal

Make this command work:

```sh
pulse-cli play <file.flac>
```

Expected result: the Matrix Mini-i Pro 4 shows the source file's native rate/depth while Pulse owns the device in exclusive mode.

## Approach

Build the path from the outside inward:

1. Device discovery: list Core Audio output devices and pick the default output device.
2. File probe: use Symphonia to read native sample rate, bit depth, channel count, and codec/container.
3. HAL control plane: take hog mode, switch nominal sample rate, select a matching physical integer format, and confirm Core Audio reports the new state.
4. Data plane: decode PCM on a normal thread, push into an `rtrb` ring buffer, and let the IOProc drain into the device buffer.
5. Validation: run known files at 44.1, 48, 96, and 192 kHz; confirm the DAC readout changes per file.

The first pass should be conservative. Reject unsupported device formats rather than adding clever conversion. Silent fallback is worse than failure because it undermines the bit-perfect claim.

## Commands

Initial CLI surface:

```sh
pulse-cli devices
pulse-cli probe <file>
pulse-cli play <file>
```

Later, after the core path works:

```sh
pulse-cli validate <file>
```

Do not add library management, playlists, settings, metadata editing, or UI-like behavior to `pulse-cli`.

## Implementation Steps

### Step 1: Implement device listing

Files:

- `crates/pulse-engine/src/device.rs`
- `crates/pulse-engine/src/hal.rs`
- `crates/pulse-cli/src/main.rs`

Add `pulse-cli devices` and make it print Core Audio output devices with IDs and names. Include the default device marker if practical.

Verification:

- Running `pulse-cli devices` lists the Matrix DAC when connected.
- Running without the Matrix still lists built-in or system output devices.

### Step 2: Implement file probing

Files:

- `crates/pulse-engine/src/decode.rs`
- `crates/pulse-cli/src/main.rs`

Add `pulse-cli probe <file>` and make it print the decoded stream's native format.

Verification:

- FLAC reports correct sample rate, bit depth, and channel count.
- ALAC, AIFF, and WAV are either correctly probed or fail with a useful decode error.

### Step 3: Implement HAL format switching

Files:

- `crates/pulse-engine/src/hal.rs`
- `crates/pulse-engine/src/device.rs`
- `crates/pulse-engine/src/error.rs`

Implement the control plane:

- Take hog mode.
- Query physical formats.
- Set nominal sample rate.
- Select a matching integer physical format.
- Wait for Core Audio property notifications.
- Re-read the active format before returning success.

Verification:

- `pulse-cli play` can switch the Matrix DAC between files with different sample rates.
- System audio cannot mix while Pulse owns the device.
- Unsupported formats produce `NoMatchingFormat` rather than fallback conversion.

### Step 4: Implement IOProc playback

Files:

- `crates/pulse-engine/src/ioproc.rs`
- `crates/pulse-engine/src/lib.rs`
- `crates/pulse-engine/src/levels.rs`

Implement raw HAL IOProc lifecycle and ring-buffer draining.

Verification:

- Playback starts and stops cleanly.
- Stopping playback releases the device.
- Underrun behavior fills silence and does not block the callback.

### Step 5: Wire `pulse-cli play`

Files:

- `crates/pulse-cli/src/main.rs`
- `crates/pulse-engine/src/decode.rs`
- `crates/pulse-engine/src/lib.rs`

Make `pulse-cli play <file>` run the whole path:

```text
probe file -> open default device -> set native format -> decode -> feed -> play
```

Verification:

- Known 44.1, 48, 96, and 192 kHz files play.
- Known 16-bit and 24-bit files play.
- The DAC shows the expected native rate/depth.

## Non-Goals

- No Tauri UI work.
- No library database.
- No metadata editing.
- No playlists or queue model.
- No DSP, volume normalization, crossfade, or EQ.
- No DSD, DoP, DSF/DFF, FFmpeg, libmpv, or GPL dependencies.

## Validation Matrix

| File | Expected DAC Readout | Result |
|------|----------------------|--------|
| FLAC 44.1 kHz / 16-bit / 2ch | 44.1 kHz / 16-bit | |
| FLAC 48 kHz / 24-bit / 2ch | 48 kHz / 24-bit | |
| FLAC 96 kHz / 24-bit / 2ch | 96 kHz / 24-bit | |
| FLAC 192 kHz / 24-bit / 2ch | 192 kHz / 24-bit | |
| ALAC 44.1 kHz / 16-bit / 2ch | 44.1 kHz / 16-bit | |
| AIFF 44.1 kHz / 16-bit / 2ch | 44.1 kHz / 16-bit | |
| WAV 44.1 kHz / 16-bit / 2ch | 44.1 kHz / 16-bit | |

## Open Questions

- How should the engine represent 24-bit audio when the DAC exposes 24 meaningful bits in a 32-bit physical container?
- Should `play` default to the system output device or require `--device` once multiple output devices are present?
- What is the smallest useful underrun metric to expose without adding app-shell concepts to the engine?

