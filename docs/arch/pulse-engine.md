# Pulse Engine Architecture

> Companion to [`tech-stack.md`](tech-stack.md). The stack doc records technology choices; this doc explains how playback hangs together, which layer owns which responsibility, and why the boundaries exist.

## 1. Overview

`pulse-engine` is the UI-agnostic playback engine. It should own the complete playback behavior: device discovery, decode, native-rate configuration, low-level AUHAL output, playback state, transport commands, queue mechanics, and progress/events.

The Tauri app and `pulse-cli` should both drive the same Rust engine API. They are adapters, not playback owners.

The current implemented core already proves clean native-rate playback through Core Audio's Hardware AudioUnit path, also called AUHAL. Pulse still talks directly to Core Audio HAL for device control, but the audio callback is owned by an AudioUnit rather than a raw `AudioDeviceCreateIOProcID` callback.

That is a pragmatic move up the stack. Raw HAL IOProc gives maximum control, but Matrix hardware exposed a client-format mismatch: the physical stream could be integer while the callback buffer stayed float32. Writing integer bytes into that float32 client buffer produced heavy noise. AUHAL makes the client contract explicit: Pulse feeds interleaved float32, and Core Audio converts from that client format to the device's physical format.

This means the current playback path should not claim hard bit-perfect integer output. The honest claim is native-rate, exclusive Core Audio playback with no Pulse-side DSP. A future lower-level backend can revisit raw integer HAL once we have more hardware validation.

## 2. Runtime Picture

The desired product runtime is a long-lived controller inside the Rust process, not a separate OS server process.

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ React UI or pulse-cli                                                        │
│                                                                              │
│  - play / pause / resume / seek / stop / next / previous                     │
│  - select output device                                                      │
│  - receive state, position, errors, now-playing updates                      │
└──────────────────────────────┬───────────────────────────────────────────────┘
                               │ adapter API
                               ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ Tauri backend or CLI adapter                                                 │
│                                                                              │
│  - translates UI/CLI input into PlaybackCommand                              │
│  - translates PlaybackEvent into Tauri events or CLI output                  │
│  - does not own playback state machine                                       │
└──────────────────────────────┬───────────────────────────────────────────────┘
                               │ command channel
                               ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ pulse-engine::PlaybackController                                             │
│                                                                              │
│  controller thread / normal Rust code                                        │
│  - owns transport state                                                      │
│  - owns queue/current source                                                 │
│  - owns selected output device                                               │
│  - spawns or drives decode work                                              │
│  - commands the low-level Engine                                             │
│  - emits PlaybackEvent                                                       │
└──────────────────────────────┬───────────────────────────────────────────────┘
                               │ low-level engine API
                               ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ pulse-engine low-level playback path                                         │
│                                                                              │
│  decode.rs                  hal.rs                                           │
│  normal thread              control plane                                    │
│  - open FLAC/ALAC/AIFF/WAV  - enumerate devices                              │
│  - read native format       - take hog mode                                  │
│  - decode integer PCM       - set nominal sample rate                        │
│  - produce PCM chunks       - set/probe physical format                      │
│                                                                              │
│  Engine                     lock-free SPSC ring                              │
│  normal thread              bounded f32 buffer                               │
│  - PCM -> f32 packing       - producer from Engine::feed                     │
│  - bounded rtrb producer    - consumer from AUHAL callback                   │
│                                                                              │
│  auhal.rs                                                                    │
│  AudioUnit render callback                                                    │
│  - drain f32 bytes from ring                                                  │
│  - copy into AUHAL output buffer                                              │
│  - fill underruns with silence                                                │
│  - update atomics and return before deadline                                  │
└──────────────────────────────┬───────────────────────────────────────────────┘
                               │ float32 client stream
                               ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ Core Audio Hardware AudioUnit / AUHAL                                         │
│                                                                              │
│  - accepts Pulse's float32 client stream                                      │
│  - talks to the selected hardware device                                      │
│  - performs any client-to-physical conversion                                 │
└──────────────────────────────────────────────────────────────────────────────┘
```

SQLite, React component state, library scanning, artwork, lyrics, metadata enrichment, and sidebar navigation belong outside the playback engine. The engine may receive a `PathBuf`, queue item ID, title for diagnostics, or selected device ID, but it should not know about album pages, design components, or Tauri event names.

## 3. Audio Terms In Software Terms

**PCM** is an array of samples. For stereo, it is usually interleaved: left sample, right sample, left sample, right sample. A "frame" means one sample per channel, so one stereo frame contains two samples.

**Sample rate** is how many frames per second the DAC consumes: 44100, 48000, 96000, 192000, etc. If the file is 44.1 kHz and the device is set to 48 kHz, something must resample. Pulse treats native sample-rate switching as a core requirement.

**Bit depth** is the integer width of each source sample: usually 16-bit or 24-bit for local FLAC/ALAC/WAV/AIFF. The AUHAL path converts those integer samples to float32 before the Core Audio callback.

**Float32 client format** is the application-side format used by the Hardware AudioUnit. On macOS this is the compatible path: apps feed float32, and Core Audio maps that to the device's physical format.

**Physical format** is the stream format used by the hardware side of the device. Pulse can inspect and request this through HAL, but AUHAL's callback buffer is not the same thing as the physical format.

**Hog mode** is exclusive ownership of an output device. In ordinary app terms, it is a lease that prevents other processes from sharing the same output device while Pulse plays.

**Bit-perfect** means the engine and operating system do not transform sample values. The current AUHAL path is not where we should make that claim, because it deliberately crosses a float32 client boundary.

## 4. Thread Model

There are four practical execution contexts once the controller exists.

### 4.1 Adapter Thread

The adapter is `pulse-cli` today and the Tauri backend for the desktop app. It can parse commands, receive UI invocations, serialize output, and forward events.

Adapter responsibilities:

- Translate user intent into `PlaybackCommand`.
- Subscribe to `PlaybackEvent`.
- Surface errors and state to CLI or UI.
- Avoid owning playback state beyond display state.

### 4.2 Playback Controller Thread

The controller is normal Rust code. It can allocate, lock its own state, wait on command channels, open files, and coordinate workers.

Controller responsibilities:

- Own current state: idle, loading, playing, paused, stopping, ended, error.
- Own queue and current item.
- Own selected output device and current stream format.
- Decide when to open/reopen the low-level engine.
- Decide when to spawn decode, pause feeding, resume feeding, seek, or advance.
- Emit state, position, now-playing, ended, and error events.

### 4.3 Decode And Packing Work

Decode is normal blocking work. Symphonia reads containers and codecs, turns compressed audio into integer PCM, and passes interleaved PCM bytes to `Engine::feed`.

`Engine::feed` is also normal-thread work. It converts source PCM into interleaved native-endian float32 and pushes those bytes into a bounded `rtrb` ring buffer.

This is where allocation, file I/O, seeking, and format conversion belong. None of that should happen in the realtime callback.

### 4.4 AudioUnit Render Callback

AUHAL calls the render callback when the device needs more frames. This is the hard realtime-ish boundary.

Allowed inside the callback:

- Read already-packed float32 bytes from the `rtrb` consumer.
- Copy bytes into the provided output buffer.
- Fill missing bytes with silence.
- Update atomics such as playback position or underrun counters.

Forbidden inside the callback:

- Heap allocation.
- Locks, including mutexes and RwLocks.
- Filesystem, logging, stdout, stderr, or syscalls.
- Waiting on channels or condition variables.
- Calling Tauri, React, SQLite, HTTP, tag parsing, or metadata code.
- Format negotiation or device enumeration.
- Anything whose runtime is not tightly bounded.

This rule is not an optimization preference. Violating it creates audio dropouts.

## 5. Crate Layout

Current modules:

```text
crates/pulse-engine/
  Cargo.toml
  src/
    lib.rs
    auhal.rs
    device.rs
    decode.rs
    hal.rs
    levels.rs
    error.rs
```

Expected controller-era modules:

```text
crates/pulse-engine/src/
  lib.rs
  backend.rs or engine.rs     low-level AUHAL Engine currently in lib.rs
  controller.rs               long-lived playback command loop
  command.rs                  PlaybackCommand and command response types
  event.rs                    PlaybackEvent and subscription surface
  state.rs                    PlaybackState, PlaybackStatus, timing snapshots
  queue.rs                    queue/current item model, still UI-agnostic
  source.rs                   playable file source metadata used by engine
  auhal.rs
  device.rs
  decode.rs
  hal.rs
  levels.rs
  error.rs
```

The naming can shift during implementation, but the boundary should not: low-level AUHAL primitives and high-level playback control both live in `pulse-engine`; Tauri and CLI remain adapters.

## 6. Public Boundary

There are two API layers.

The low-level API exists today and is useful for focused hardware validation:

```rust
let decoded = decode::open(path)?;
let device = device::default_output_device()?;

let mut engine = Engine::open(device.id)?;
engine.set_format(decoded.format)?;
engine.play()?;

// Decode loop feeds already-decoded interleaved source PCM.
let accepted = engine.feed(&pcm);
```

The product API should be controller-oriented:

```rust
let controller = PlaybackController::spawn(config);
let events = controller.subscribe();

controller.send(PlaybackCommand::PlayFile { path })?;
controller.send(PlaybackCommand::Pause)?;
controller.send(PlaybackCommand::Resume)?;
controller.send(PlaybackCommand::Seek { position_ms: 90_000 })?;
controller.send(PlaybackCommand::Stop)?;
```

The controller may expose sync command acknowledgements for CLI tests, but the normal product flow should be command in, event out.

## 7. Command And Event Model

Initial commands:

```rust
pub enum PlaybackCommand {
    PlayFile { path: PathBuf },
    Pause,
    Resume,
    Stop,
    Seek { position_ms: u64 },
    SetOutputDevice { device_id: device::DeviceId },
}
```

Queue commands can land once single-file transport is stable:

```rust
pub enum QueueCommand {
    SetQueue { items: Vec<PlayableSource>, start_index: usize },
    PlayIndex { index: usize },
    Next,
    Previous,
    Clear,
}
```

Initial events:

```rust
pub enum PlaybackEvent {
    StateChanged(PlaybackState),
    NowPlaying { source: PlayableSource, format: PcmFormat },
    Position { position_ms: u64, duration_ms: Option<u64> },
    DeviceChanged { device: device::Device },
    Ended,
    Error { message: String },
}
```

The exact Rust shapes can change, but the behavior should stay boring: commands are imperative, events are observable facts, and UI state is derived from events.

## 8. Module Responsibilities

### 8.1 `controller.rs`

`controller.rs` owns transport behavior.

Responsibilities:

- Own the command receiver and event sender.
- Own current playback state.
- Own the current queue item and decode worker handle.
- Start, pause, resume, stop, and seek playback.
- Handle end-of-track and advance policy.
- Restart low-level engine when the output device or source format changes.
- Emit events frequently enough for UI progress without spamming.

### 8.2 `state.rs`

`state.rs` defines state snapshots and transition-friendly enums.

Minimum useful states:

- `Idle`
- `Loading`
- `Playing`
- `Paused`
- `Stopping`
- `Ended`
- `Error`

The state model should distinguish "paused with an engine/source that can resume" from "stopped and must reopen/redecode."

### 8.3 `queue.rs` / `source.rs`

These modules define engine-level playable inputs.

They may know about:

- File path.
- Stable source ID supplied by the caller.
- Optional display title/artist/album for diagnostics/events.
- Duration if known.

They must not know about:

- SQLite row layout.
- Album grid UI.
- Artwork loading.
- Tauri window names.

### 8.4 `device.rs`

`device.rs` is the discovery layer for Core Audio output devices.

Current public shape:

```rust
pub type DeviceId = u32;

pub struct Device {
    pub id: DeviceId,
    pub uid: String,
    pub name: String,
}

pub fn list_output_devices() -> Result<Vec<Device>, EngineError>;
pub fn default_output_device() -> Result<Device, EngineError>;
```

This module should stay about identity and discovery. It should not own playback state.

### 8.5 `decode.rs`

`decode.rs` is the file-to-PCM layer.

It uses Symphonia because Pulse's v1 format scope is PCM-only: FLAC, ALAC, AIFF, WAV.

Responsibilities:

- Open supported files.
- Inspect native sample rate, bit depth, channels, and frame count.
- Decode compressed frames into interleaved integer PCM.
- Seek by frame/time when the controller needs it.
- Report decode failures as `EngineError::Decode`.

### 8.6 `hal.rs`

`hal.rs` is the direct Core Audio HAL control plane.

It wraps `AudioObjectGetPropertyData`, `AudioObjectSetPropertyData`, and related property calls behind small Rust functions.

Responsibilities:

- Acquire and release hog mode.
- Read default and named output devices.
- Check whether an output device has output channels.
- Switch nominal sample rate.
- Query supported physical formats.
- Set the preferred physical format when possible.
- Wait for async device state changes before trusting them.

This module is still direct `objc2-core-audio`, not `coreaudio-rs`, because we want explicit control and diagnostics around device state.

### 8.7 `auhal.rs`

`auhal.rs` owns the playback callback lifecycle.

Responsibilities:

- Create a Hardware Output AudioUnit for the selected device.
- Set the AudioUnit client stream format to interleaved float32.
- Register a render callback using `coreaudio-rs`.
- Start and stop the AudioUnit.
- Drain float32 bytes from the `rtrb` consumer inside the callback.
- Fill underruns with silence.
- Track playback position and underruns with atomics.

The callback must remain tiny. Format conversion happens before bytes enter the ring.

### 8.8 `levels.rs`

`levels.rs` is the future analysis layer for RMS, peak, and spectrum data.

The important boundary is that analysis cannot slow playback. The callback may eventually copy a small preallocated tap, but FFT and UI-friendly smoothing must run elsewhere.

## 9. Playback Lifecycles

### 9.1 Current Low-Level CLI Lifecycle

Happy path today:

1. Caller opens a file with `decode::open(path)`.
2. Caller resolves an output device with CLI config, `device::default_output_device()`, or an explicit device ID.
3. `Engine::open(device_id)` takes hog mode.
4. `Engine::set_format(format)` switches nominal sample rate and asks HAL for a compatible physical format.
5. `Engine::set_format(format)` creates an `rtrb` ring sized for float32 output.
6. `Engine::play()` creates and starts the AUHAL sink.
7. Decode streams integer PCM chunks.
8. `Engine::feed(pcm)` converts accepted frames to float32 and pushes them into the ring.
9. Core Audio pulls float32 frames from the render callback on its own schedule.
10. CLI waits until AUHAL has pulled all accepted frames, then calls `Engine::pause()`.

The current `pause()` is closer to "stop the AUHAL sink" than product pause/resume semantics.

### 9.2 Controller Lifecycle

Happy path after Stage 6:

1. Adapter sends `PlaybackCommand::PlayFile`.
2. Controller emits `Loading`.
3. Controller probes the source format and duration.
4. Controller opens or reuses the selected output device.
5. Controller configures the low-level engine for the source native format.
6. Controller starts AUHAL and decode feeding.
7. Controller emits `NowPlaying` and `Playing`.
8. Controller emits position snapshots while playing.
9. `Pause` stops feeding and stops or suspends AUHAL according to the chosen implementation.
10. `Resume` continues from the preserved logical position.
11. `Seek` stops current feeding, seeks decode to the requested frame/time, resets the engine ring, and resumes according to prior state.
12. End-of-track emits `Ended` or advances the queue.

The controller owns the difference between pause, stop, and end. The low-level callback should not know those concepts.

## 10. Backpressure And Underruns

Decode is producer. AUHAL callback is consumer. The buffer is bounded. That is intentional: unbounded queues hide bugs, allocate memory, and increase latency.

When the producer is faster than the device, `feed` accepts fewer source frames than requested. The decode loop can retry later. When the producer is slower than the device, the callback fills missing bytes with silence. The callback never waits for decode.

The current CLI behavior is simple: keep trying to feed until the current decoded packet is accepted, sleeping briefly on backpressure.

The controller should make backpressure an internal detail. UI should see `Playing`, position, and underrun counters/events if useful; it should not participate in feeding.

## 11. Pause, Resume, And Seek Semantics

Product pause means the user can resume from the same logical playback position without restarting the track from zero.

The first implementation can choose a conservative approach:

- On pause, stop AUHAL, stop or park decode, and record the logical frame position.
- On resume, reopen/reset the low-level engine at the same source format, seek decode to the recorded frame, and continue feeding.
- On seek, use the same rebuild path with a different target frame.

This is simpler and more correct than trying to preserve partially buffered ring contents across pause/resume. Later we can optimize by keeping AUHAL alive and feeding silence while paused if that proves necessary for device stability.

## 12. Format Examples

For a 44.1 kHz / 16-bit / stereo FLAC:

```text
source PCM frame:
  i16 left + i16 right = 4 source bytes

Engine::feed:
  convert each i16 sample to f32 in [-1.0, 1.0)

AUHAL client frame:
  f32 left + f32 right = 8 output bytes

HAL control plane:
  nominal sample rate requested as 44100 Hz
  physical format requested/probed where the device supports it
```

For a 96 kHz / 24-bit / stereo FLAC:

```text
source PCM frame:
  i24 left + i24 right = 6 source bytes

Engine::feed:
  sign-extend each i24 sample and scale to f32

AUHAL client frame:
  f32 left + f32 right = 8 output bytes
```

The AUHAL boundary is why the current implementation prioritizes clean playback and device behavior over hard bit-perfect claims.

## 13. Testing Strategy

Unit tests can validate parsing, state transitions, command handling, decode loops, PCM-to-float packing, ring-buffer behavior, and error mapping. They cannot prove playback quality.

Controller tests should use fake low-level engine and fake decode sources first. Hardware should not be required to test pause/resume/seek state transitions.

Hardware smoke tests are required for:

- Device enumeration.
- Hog mode acquisition and release.
- Native sample-rate switching.
- Clean audible playback through the Matrix DAC.
- DAC front-panel sample-rate reporting.
- Pause/resume without restarting from zero.
- Seek to the middle of a file.
- Stop releases the device.
- 16/44.1 and 24/96 files.

The validation language must match the backend. AUHAL can be validated for clean native-rate playback. It should not be marketed as raw bit-perfect integer output.

## 14. Current Non-Goals

- DSD, DoP, DSF, DFF.
- Video playback or video library support.
- Streaming integrations.
- libmpv, FFmpeg, or GPL audio dependencies.
- App UI owning playback state directly.
- Tauri-specific types inside `pulse-engine`.
- SQLite/library schema inside `pulse-engine`.
- Raw HAL integer IOProc in this stage.

## 15. Immediate Target

The next engine target is a playback controller that can drive a single local file through the same proven AUHAL path while supporting real transport commands.

Minimum target:

```text
PlayFile(path)
Pause
Resume
Seek(position)
Stop
State/position/error events
```

Once that is reliable through both a CLI smoke path and Tauri commands, the designed desktop playback row can be wired without moving product logic into React.
