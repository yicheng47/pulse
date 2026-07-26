# AUHAL Playback

> Third product stage: make `pulse-cli play <file>` produce clean native-rate playback through Core Audio's Hardware AudioUnit path.

## Context

Stage 1 proved file and device inspection. Stage 2 proved hog mode, nominal sample-rate switching, and physical format selection. The first Stage 3 attempt used raw HAL IOProc playback, but Matrix hardware exposed a client-format mismatch: the physical stream could be integer while the callback buffer stayed float32.

The raw IOProc smoke produced heavy noise because Pulse wrote signed-integer PCM bytes into a float32 client buffer. A temporary Core Audio probe showed this state:

```text
current virtual:  44100 Hz / lpcm / flags 0x9 / 32 bits / 2 ch / 8 Bframe
current physical: 44100 Hz / lpcm / flags 0xc / 32 bits / 2 ch / 8 Bframe
```

Follow-up probing showed `kAudioStreamPropertyVirtualFormat` reported as settable and `AudioObjectSetPropertyData` returned `noErr`, but the Matrix stream stayed at float32 even while Pulse owned hog mode:

```text
virtual settable: 1
toggle hog status: 0, owner = current pid
target virtual: 44100 Hz / lpcm / flags 0x4c / 32 bits / 2 ch / 8 Bframe
set virtual status: 0
current virtual remains: 44100 Hz / lpcm / flags 0x9 / 32 bits / 2 ch / 8 Bframe
```

The product decision for this stage is to move one level up the stack: AUHAL / Hardware AudioUnit for playback, direct HAL for device control and diagnostics.

## First Slice

The AUHAL playback slice is intentionally narrow:

- Acquire hog mode through `Engine::open`.
- Set nominal sample rate through `Engine::set_format`.
- Ask HAL to select/probe a compatible physical stream format when possible.
- Convert decoded interleaved integer PCM to interleaved native-endian float32 in `Engine::feed`.
- Store float32 bytes in an `rtrb` SPSC ring buffer.
- Start a Hardware AudioUnit through `coreaudio-rs`.
- Set the AudioUnit client stream format to interleaved float32.
- Let the AudioUnit render callback copy available bytes into Core Audio output buffers.
- Fill underruns with silence.

## Playback Packing

The AUHAL backend supports source PCM widths that Symphonia emits for the current file scope:

- 16-bit signed integer PCM scales by `1 / 32768`.
- 24-bit signed integer PCM is sign-extended, then scales by `1 / 8388608`.
- 32-bit signed integer PCM scales by `1 / 2147483648`.

The output ring always stores interleaved float32 frames. For stereo, one source 16-bit frame is 4 bytes, but one AUHAL client frame is 8 bytes.

This is not a bit-perfect integer path. It is a clean playback path with explicit conversion at the app/Core Audio boundary.

## Hardware Smoke Result

Matrix Audio `mini-i Series` was visible as Core Audio output device `154` in one run and `201` in another. Core Audio device IDs are not stable across reconnects.

The validation path successfully took hog mode, switched nominal sample rate to 44.1 kHz, and selected a 44.1 kHz / 32-bit / 2-channel signed-integer physical format for `/Users/jason/Downloads/03. Ex-Factor.flac`.

The raw IOProc implementation produced noisy audio. The AUHAL implementation replaced it, and the Matrix hardware smoke confirmed the same track now plays with correct sound while the DAC reports the expected 44.1 kHz sample rate.

## Remaining Smoke

- Add a 24-bit hardware smoke with a 24/96 FLAC.
- Add underrun counters to CLI diagnostics.
- Add bounded `--duration` and JSON output for agent/MCP use.
- Add property listeners instead of polling for production-grade format changes.
