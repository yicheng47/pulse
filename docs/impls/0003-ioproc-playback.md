# IOProc Playback

> Third product stage: make `pulse-cli play <file>` drive raw HAL IOProc output from decoded PCM.

## Context

Stage 1 proved file and device inspection. Stage 2 proved hog mode, nominal sample-rate switching, and physical format selection. Stage 3 connects the data plane: decode thread to ring buffer to Core Audio's realtime IOProc callback.

This is the first stage where Pulse can make sound. It is also the first stage where realtime constraints matter.

## First Slice

The first playback slice is intentionally narrow:

- Acquire hog mode through `Engine::open`.
- Set nominal sample rate and physical stream format through `Engine::set_format`.
- Create an `rtrb` byte ring buffer sized for several seconds of PCM.
- Start a raw `AudioDeviceCreateIOProcID` callback through `Engine::play`.
- Decode exact signed-integer PCM packets through Symphonia.
- Feed interleaved raw PCM bytes into the ring buffer from the CLI thread.
- Let the IOProc copy available bytes into Core Audio output buffers.
- Fill underruns with silence.

## Deliberate Restrictions

The first `play` implementation only supports direct byte-copy output:

- Signed integer decoder output only.
- Interleaved physical output only.
- Native-endian physical output only.
- Physical bit depth must equal source bit depth.
- Physical bytes per frame must equal source bytes per frame.

This means 24-bit source into a 32-bit physical container can validate in stage 2 but should still fail playback until the packer is implemented. That is preferable to silently shifting or padding samples incorrectly.

## Follow-Ups

- Add a packer for 24 meaningful bits inside 32-bit physical containers.
- Support non-interleaved output buffers if needed by real devices.
- Add underrun counters to CLI diagnostics.
- Add bounded `--duration` and JSON output for agent/MCP use.
- Add property listeners instead of polling for production-grade format changes.
