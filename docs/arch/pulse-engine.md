# Pulse Engine Architecture

> Companion to [`tech-stack.md`](tech-stack.md). The stack doc records the technology choices; this doc explains how the engine hangs together and why those boundaries exist. It assumes you are a strong software engineer, not an audio engineer.

## 1. Overview

`pulse-engine` is the playback core. It is deliberately independent from Tauri: `pulse-cli` and the future desktop app should both drive the same Rust API.

The current engine goal is clean native-rate playback through Core Audio's Hardware AudioUnit path, also called AUHAL. Pulse still talks directly to Core Audio HAL for device control, but the audio callback is now owned by an AudioUnit rather than by a raw `AudioDeviceCreateIOProcID` callback.

That is a pragmatic move up the stack. Raw HAL IOProc gives maximum control, but Matrix hardware exposed a client-format mismatch: the physical stream could be integer while the callback buffer stayed float32. Writing integer bytes into that float32 client buffer produced heavy noise. AUHAL makes the client contract explicit: Pulse feeds interleaved float32, and Core Audio converts from that client format to the physical device format.

This means the first playback milestone should not claim hard bit-perfect integer output. The honest claim is native-rate, exclusive Core Audio playback with no Pulse-side DSP. A future lower-level engine can revisit raw integer HAL once we have more hardware validation.

## 2. Runtime Picture

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ pulse-cli or future Tauri backend                                            │
│                                                                              │
│  - chooses file                                                              │
│  - chooses output device                                                     │
│  - calls Engine::open / set_format / play / feed / pause                     │
└──────────────────────────────┬───────────────────────────────────────────────┘
                               │ public Rust API
                               ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ pulse-engine                                                                 │
│                                                                              │
│  ┌──────────────────────────┐       ┌─────────────────────────────────────┐  │
│  │ decode.rs                 │       │ hal.rs                              │  │
│  │ normal thread             │       │ control plane                       │  │
│  │                           │       │                                     │  │
│  │ - open FLAC/ALAC/AIFF/WAV │       │ - enumerate devices                 │  │
│  │ - read native format      │       │ - take hog mode                     │  │
│  │ - decode integer PCM      │       │ - set nominal sample rate           │  │
│  │ - pass frames to Engine   │       │ - set/probe physical format         │  │
│  └────────────┬─────────────┘       └──────────────────┬──────────────────┘  │
│               │ PCM bytes                                │ Core Audio props   │
│               ▼                                          ▼                   │
│  ┌──────────────────────────┐       ┌─────────────────────────────────────┐  │
│  │ lib.rs                    │       │ macOS Core Audio HAL                │  │
│  │ normal thread             │       │ output device                       │  │
│  │ - PCM -> f32 packing      │       └──────────────────┬──────────────────┘  │
│  │ - bounded rtrb producer   │                          │ physical device    │
│  └────────────┬─────────────┘                          │ state              │
│               │ f32 bytes                               ▼                   │
│               ▼                                                              │
│  ┌──────────────────────────┐                                                │
│  │ lock-free SPSC ring       │                                                │
│  │ bounded f32 buffer        │                                                │
│  └────────────┬─────────────┘                                                │
│               │ rtrb consumer                                                │
│               ▼                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐    │
│  │ auhal.rs                                                             │    │
│  │ AudioUnit render callback                                             │    │
│  │                                                                      │    │
│  │ - drain f32 bytes from ring                                          │    │
│  │ - copy into AUHAL output buffer                                      │    │
│  │ - fill underruns with silence                                        │    │
│  │ - return before the deadline                                         │    │
│  └──────────────────────────────┬───────────────────────────────────────┘    │
│                                 │ float32 client stream                       │
│                                 ▼                                            │
│  ┌──────────────────────────────────────────────────────────────────────┐    │
│  │ Core Audio Hardware AudioUnit / AUHAL                                 │    │
│  │ - accepts Pulse's float32 client stream                               │    │
│  │ - talks to the selected hardware device                               │    │
│  │ - performs any client-to-physical conversion                          │    │
│  └──────────────────────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────────────────────┘
```

SQLite, React, Tauri events, library scanning, artwork, lyrics, and metadata enrichment are intentionally absent from this picture. They belong outside the engine.

## 3. Audio Terms In Software Terms

**PCM** is just an array of samples. For stereo, it is usually interleaved: left sample, right sample, left sample, right sample. A "frame" means one sample per channel, so one stereo frame contains two samples.

**Sample rate** is how many frames per second the DAC consumes: 44100, 48000, 96000, 192000, etc. If the file is 44.1 kHz and the device is set to 48 kHz, something must resample. Pulse still treats native sample-rate switching as a core requirement.

**Bit depth** is the integer width of each source sample: usually 16-bit or 24-bit for local FLAC/ALAC/WAV/AIFF. The AUHAL path converts those integer samples to float32 before the Core Audio callback.

**Float32 client format** is the application-side format used by the Hardware AudioUnit. On macOS this is the boring, compatible path: apps feed float32, and Core Audio maps that to the device's physical format.

**Physical format** is the stream format used by the hardware side of the device. Pulse can still inspect and request this through HAL, but AUHAL's callback buffer is not the same thing as the physical format.

**Hog mode** is exclusive ownership of an output device. In ordinary app terms, it is a lease that prevents other processes from sharing the same output device while Pulse plays.

**Bit-perfect** means the engine and operating system do not transform sample values. The current AUHAL path is not where we should make that claim, because it deliberately crosses a float32 client boundary.

## 4. Thread Model

There are three practical execution contexts.

### 4.1 Caller Thread

The caller is `pulse-cli` today and the Tauri backend later. It can open files, update state, log, allocate, and handle errors like normal code.

Caller responsibilities:

- Pick device and file.
- Open the engine.
- Ask decode for the file's native format.
- Ask the engine to configure the device for that format.
- Start playback.
- Feed decoded PCM.
- Surface errors and status.

### 4.2 Decode And Packing Thread

Decode is normal blocking work. Symphonia reads containers and codecs, turns compressed audio into integer PCM, and passes interleaved PCM bytes to `Engine::feed`.

`Engine::feed` is also normal-thread work. It converts source PCM into interleaved native-endian float32 and pushes those bytes into a bounded `rtrb` ring buffer.

This is where allocation and format conversion belong. None of that should happen in the realtime callback.

### 4.3 AudioUnit Render Callback

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

The module split follows the runtime picture: public boundary and PCM-to-float packing in `lib.rs`, AUHAL playback callback in `auhal.rs`, Core Audio discovery in `device.rs`, HAL control plane in `hal.rs`, decode in `decode.rs`, metering placeholders in `levels.rs`, and shared errors in `error.rs`.

## 6. Public Boundary

`lib.rs` owns the small API that both `pulse-cli` and the future Tauri backend use.

```rust
pub struct PcmFormat {
    pub sample_rate: u32,
    pub bits_per_sample: u8,
    pub channels: u8,
}

pub struct Engine {
    _device: device::DeviceId,
}
```

Intended call shape:

```rust
let decoded = decode::open(path)?;
let device = device::default_output_device()?;

let mut engine = Engine::open(device.id)?;
engine.set_format(decoded.format)?;
engine.play()?;

// Decode loop feeds already-decoded interleaved source PCM.
let accepted = engine.feed(&pcm);
```

The key design point is that `Engine` does not know about playlists, UI state, album art, metadata, app windows, or Tauri commands. It owns one output device and one playback path.

## 7. Module Responsibilities

### 7.1 `device.rs`

`device.rs` is the discovery layer for Core Audio output devices.

Current public shape:

```rust
pub type DeviceId = u32;

pub struct Device {
    pub id: DeviceId,
    pub name: String,
}

pub fn list_output_devices() -> Result<Vec<Device>, EngineError>;
pub fn default_output_device() -> Result<Device, EngineError>;
```

This module should stay about identity and discovery. It should not own playback, sample-rate switching, or hog mode.

### 7.2 `decode.rs`

`decode.rs` is the file-to-PCM layer.

It uses Symphonia because Pulse's v1 format scope is PCM-only: FLAC, ALAC, AIFF, WAV.

Responsibilities:

- Open supported files.
- Inspect native sample rate, bit depth, and channels.
- Decode compressed frames into interleaved integer PCM.
- Report decode failures as `EngineError::Decode`.

### 7.3 `hal.rs`

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

### 7.4 `auhal.rs`

`auhal.rs` owns the playback callback lifecycle.

Responsibilities:

- Create a Hardware Output AudioUnit for the selected device.
- Set the AudioUnit client stream format to interleaved float32.
- Register a render callback using `coreaudio-rs`.
- Start and stop the AudioUnit.
- Drain float32 bytes from the `rtrb` consumer inside the callback.
- Fill underruns with silence.
- Track playback position with atomics.

The callback must remain tiny. Format conversion happens before bytes enter the ring.

### 7.5 `levels.rs`

`levels.rs` is the future analysis layer for RMS, peak, and spectrum data.

The important boundary is that analysis cannot slow playback. The callback may eventually copy a small preallocated tap, but FFT and UI-friendly smoothing must run elsewhere.

## 8. Playback Lifecycle

Happy path:

1. Caller opens a file with `decode::open(path)`.
2. Caller resolves an output device with `device::default_output_device()` or an explicit device ID.
3. `Engine::open(device_id)` takes hog mode.
4. `Engine::set_format(format)` switches nominal sample rate and asks HAL for a compatible physical format.
5. `Engine::set_format(format)` creates an `rtrb` ring sized for float32 output.
6. `Engine::play()` creates and starts the AUHAL sink.
7. Decode streams integer PCM chunks.
8. `Engine::feed(pcm)` converts accepted frames to float32 and pushes them into the ring.
9. Core Audio pulls float32 frames from the render callback on its own schedule.
10. `Engine::pause()` stops the AudioUnit.

The order matters. Configure the device before starting the AudioUnit, then feed the ring from normal code.

## 9. Backpressure And Underruns

Decode is producer. AUHAL callback is consumer. The buffer is bounded. That is intentional: unbounded queues hide bugs, allocate memory, and increase latency.

When the producer is faster than the device, `feed` accepts fewer source frames than requested. The decode loop can retry later. When the producer is slower than the device, the callback fills missing bytes with silence. The callback never waits for decode.

Current CLI behavior is simple: keep trying to feed until the current decoded packet is accepted, sleeping briefly on backpressure.

## 10. Format Examples

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

## 11. Testing Strategy

Unit tests can validate parsing, decode loops, PCM-to-float packing, ring-buffer behavior, and error mapping. They cannot prove playback quality.

Hardware smoke tests are required for:

- Device enumeration.
- Hog mode acquisition and release.
- Native sample-rate switching.
- Clean audible playback through the Matrix DAC.
- DAC front-panel sample-rate reporting.
- 16/44.1 and 24/96 files.

The validation language must match the backend. AUHAL can be validated for clean native-rate playback. It should not be marketed as raw bit-perfect integer output.

## 12. Current Non-Goals

- DSD, DoP, DSF, DFF.
- Video playback.
- Streaming integrations.
- libmpv, FFmpeg, or GPL audio dependencies.
- App UI owning playback state directly.
- Raw HAL integer IOProc in this stage.

## 13. Immediate Target

The immediate target is:

```sh
pulse-cli play --device <matrix-device-id> "/Users/jason/Downloads/03. Ex-Factor.flac"
```

That command should play cleanly through the Matrix DAC at the file's native sample rate. Once that is reliable, the Tauri shell can build on an engine that has earned basic playback trust.
