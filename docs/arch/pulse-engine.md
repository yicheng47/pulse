# Pulse Engine Architecture

> Companion to [`tech-stack.md`](tech-stack.md). The stack doc records the technology choices; this doc explains how the engine hangs together and why those boundaries exist. It assumes you are a strong software engineer, not an audio engineer.

## 1. Overview

`pulse-engine` is the playback core. It is responsible for proving the one thing Pulse cannot fake: the PCM samples decoded from a local music file reach the DAC without resampling, software mixing, volume scaling, or format conversion.

The engine is deliberately independent from Tauri. `pulse-cli` and the future desktop app should both drive the same crate API. If bit-perfect playback only works through the app shell, the architecture is wrong.

The engine has two halves:

- **Control plane**: open devices, take exclusive ownership, pick the physical output format, switch sample rates, start and stop playback.
- **Data plane**: decode file samples on a normal thread, move PCM through a lock-free ring buffer, and let Core Audio pull buffers from an IOProc callback on a realtime thread.

The most important architectural rule: the realtime callback is not normal application code. It is closer to a kernel-adjacent hot path with a deadline. It cannot allocate, lock, perform I/O, call into the filesystem, wait on another thread, or do work whose upper bound is unclear.

## 2. Runtime Picture

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ pulse-cli or future Tauri backend                                            │
│                                                                              │
│  - chooses file                                                              │
│  - chooses output device                                                     │
│  - calls Engine::open / set_format / feed / play / pause                     │
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
│  │ - push frames             │       │ - set physical integer format       │  │
│  └────────────┬─────────────┘       │ - wait for property listeners       │  │
│               │                     └──────────────────┬──────────────────┘  │
│               │ rtrb producer                           │ Core Audio props   │
│               ▼                                         ▼                   │
│  ┌──────────────────────────┐       ┌─────────────────────────────────────┐  │
│  │ lock-free SPSC ring       │       │ macOS Core Audio HAL                │  │
│  │ bounded PCM buffer        │       │ output device                       │  │
│  └────────────┬─────────────┘       └──────────────────┬──────────────────┘  │
│               │ rtrb consumer                           │ pulls buffers      │
│               ▼                                         ▼                   │
│  ┌──────────────────────────────────────────────────────────────────────┐    │
│  │ ioproc.rs                                                            │    │
│  │ realtime thread callback                                             │    │
│  │                                                                      │    │
│  │ - drain PCM from ring                                                │    │
│  │ - copy samples into Core Audio output buffer                         │    │
│  │ - copy tiny tap data for levels                                      │    │
│  │ - return before the deadline                                         │    │
│  └──────────────────────────────┬───────────────────────────────────────┘    │
│                                 │                                            │
│                                 ▼                                            │
│  ┌──────────────────────────┐                                                │
│  │ levels.rs                 │                                                │
│  │ non-realtime analysis     │                                                │
│  │ - RMS / peak              │                                                │
│  │ - future spectrum bins    │                                                │
│  └──────────────────────────┘                                                │
└──────────────────────────────────────────────────────────────────────────────┘
```

SQLite, React, Tauri events, library scanning, artwork, lyrics, and metadata enrichment are intentionally absent from this picture. They belong outside the engine. The engine only proves playback correctness and exposes enough state for the shell to build a product around it.

## 3. Audio Terms In Software Terms

**PCM** is just an array of samples. For stereo, it is usually interleaved: left sample, right sample, left sample, right sample. A "frame" means one sample per channel, so one stereo frame contains two samples.

**Sample rate** is how many frames per second the DAC consumes: 44100, 48000, 96000, 192000, etc. If the file is 44.1 kHz and the device is set to 48 kHz, Core Audio or somebody else must resample. That breaks the bit-perfect claim.

**Bit depth** is the integer width of each sample: usually 16-bit or 24-bit for Pulse v1. If the file is integer PCM but the output path converts through float, the bits are no longer guaranteed unchanged.

**Bit-perfect** means the engine does not transform the sample values. It also means the operating system is not secretly transforming them after the engine hands them over. That is why the HAL device format matters as much as the decoder.

**Hog mode** is exclusive ownership of an output device. In ordinary app terms, it is a lease that prevents other processes and the system mixer from sharing the same output path.

**Integer mode / physical format** means the actual stream format accepted by the hardware is native integer samples, not a float format with an implicit converter.

**IOProc** is the Core Audio render callback. Core Audio calls it when the device needs more frames. It is pull-based: the hardware clock decides when the callback runs.

## 4. Thread Model

There are three practical execution contexts.

### 4.1 Caller Thread

The caller is `pulse-cli` today and the Tauri backend later. It can open files, update state, log, allocate, and handle errors like normal code.

Caller responsibilities:

- Pick device and file.
- Open the engine.
- Ask decode for the file's native format.
- Ask the engine to configure the device for that format.
- Start decode and playback.
- Surface errors and status.

### 4.2 Decode Thread

Decode is normal blocking work. Symphonia reads containers and codecs, turns compressed audio into PCM, and pushes frames into the ring buffer.

Decode can allocate. Decode can read from disk. Decode can block on file I/O. Decode can handle complex codec logic. None of that is allowed to leak into the IOProc.

If the ring buffer is full, decode applies backpressure. The current public API shape returns "frames accepted" from `feed` rather than blocking forever. That keeps the playback hot path bounded.

### 4.3 Realtime IOProc Thread

The IOProc is the hard boundary. Core Audio owns the schedule and calls Pulse when the device needs more samples.

Allowed inside IOProc:

- Read already-decoded bytes from the `rtrb` consumer.
- Copy samples into the provided output buffer.
- Update preallocated counters or atomics.
- Copy a small tap of samples into preallocated analysis storage.

Forbidden inside IOProc:

- Heap allocation.
- Locks, including mutexes and RwLocks.
- Filesystem, logging, stdout, stderr, or syscalls.
- Waiting on channels or condition variables.
- Calling Tauri, React, SQLite, HTTP, or tag parsing code.
- Format negotiation or device enumeration.
- Anything whose runtime is not tightly bounded.

This rule is not an optimization preference. Violating it creates audio dropouts.

## 5. Crate Layout

```text
crates/pulse-engine/
  Cargo.toml
  src/
    lib.rs
    device.rs
    decode.rs
    hal.rs
    ioproc.rs
    levels.rs
    error.rs
```

The module split follows the runtime picture: public boundary in `lib.rs`, Core Audio discovery in `device.rs`, HAL control plane in `hal.rs`, data-plane callback in `ioproc.rs`, decode in `decode.rs`, metering in `levels.rs`, and shared errors in `error.rs`.

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

// Decode loop feeds already-decoded interleaved PCM.
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

This module should stay about identity and discovery. It should not configure sample rates, own hog mode, or manage IOProcs. Those belong to `hal.rs` and `ioproc.rs`.

### 7.2 `decode.rs`

`decode.rs` is the file-to-PCM layer.

It uses Symphonia because Pulse's v1 format scope is PCM-only: FLAC, ALAC, AIFF, WAV. The output should preserve the file's native integer format whenever possible, because converting everything into float would defeat the point of the engine.

Responsibilities:

- Open supported files.
- Inspect native sample rate, bit depth, and channels.
- Decode compressed frames into interleaved PCM.
- Feed the engine from a normal thread.
- Report decode failures as `EngineError::Decode`.

Non-responsibilities:

- Picking output devices.
- Switching hardware sample rate.
- Owning realtime state.
- Reading tags for library metadata. That belongs to later library code, probably with `lofty`.

### 7.3 `hal.rs`

`hal.rs` is the Core Audio control plane.

Core Audio HAL is mostly a property API: get or set a property on an audio object using an address made of selector, scope, and element. That is awkward, but it maps cleanly to a small wrapper layer.

Responsibilities:

- Take and release hog mode.
- Query supported physical formats.
- Set the device's nominal sample rate.
- Set the stream's physical integer format.
- Install property listeners for async changes.
- Wait until Core Audio confirms the requested rate or format is actually active.
- Convert `OSStatus` failures into `EngineError::Os`.

The async part matters. Sample-rate and physical-format changes are not trustworthy just because `AudioObjectSetPropertyData` returned success. The device may take time to settle, and some devices can reject, adjust, or invalidate formats. The wrapper should wait for the relevant property notification and then re-read the property.

### 7.4 `ioproc.rs`

`ioproc.rs` owns the raw output callback lifecycle.

Responsibilities:

- Create the IOProc with `AudioDeviceCreateIOProcID`.
- Start the device with `AudioDeviceStart`.
- Stop with `AudioDeviceStop`.
- Destroy or release callback state safely.
- Keep the callback code tiny and bounded.
- Drain the `rtrb` consumer into Core Audio's output buffer.

This module is intentionally not AUHAL. AUHAL is AudioUnit-shaped and can insert an implicit converter. Pulse's bit-perfect path uses raw HAL IOProc because it gives direct control over the device path.

The callback should have all memory it needs before `AudioDeviceStart`. If a future implementation needs scratch buffers, allocate them before starting playback and pass raw/preallocated state into the callback.

### 7.5 `levels.rs`

`levels.rs` exposes metering data without contaminating the realtime path.

Current shape:

```rust
pub struct Levels {
    pub rms: [f32; 2],
    pub peak: [f32; 2],
}
```

The now-playing UI wants VU meters and a spectrum. The safe structure is to tap a small copy of the audio buffer and do analysis off the realtime callback. The UI does not need sample-accurate timing; 30-60 Hz payloads are enough.

Responsibilities:

- Track RMS and peak.
- Later, compute FFT bins with `realfft`.
- Expose tiny snapshots to the app shell.

Non-responsibilities:

- Driving playback.
- Doing heavy FFT work inside IOProc.
- Sending Tauri events directly from the engine.

### 7.6 `error.rs`

`error.rs` keeps failure modes explicit.

Current errors:

- `Os { call, status }`: a Core Audio call failed.
- `Hogged(pid)`: another process owns the device.
- `NoMatchingFormat(PcmFormat)`: the DAC cannot expose the requested physical format.
- `Decode(String)`: the decoder failed.

The important product rule: errors should preserve enough detail to tell whether Pulse failed, Core Audio rejected the request, or the device simply does not support the file's native format.

## 8. Playback Lifecycle

The happy path is:

1. Caller selects a file.
2. `decode::open(path)` reads the file's native `PcmFormat`.
3. Caller selects a device from `device::list_output_devices()` or `device::default_output_device()`.
4. `Engine::open(device)` takes hog mode and prepares engine state.
5. `Engine::set_format(format)` switches nominal sample rate and physical integer format.
6. Decode thread starts pushing PCM into the ring buffer.
7. `Engine::play()` starts the IOProc.
8. Core Audio pulls frames from the IOProc on its own schedule.
9. `Engine::pause()` or track end stops playback.
10. Engine release drops hog mode and device state.

Two sequencing details are load-bearing:

- Configure the device before starting IOProc.
- Confirm the device actually switched format before sending audio.

If those are wrong, the app can appear to play music while silently losing the bit-perfect guarantee.

## 9. Backpressure And Buffering

The ring buffer is the boundary between normal code and realtime code.

Decode is producer. IOProc is consumer. The buffer is bounded. That is intentional: unbounded queues hide bugs, allocate memory, and increase latency.

When the producer is faster than the device, `feed` accepts fewer frames than requested. The decode loop can retry later or pause reading. When the producer is slower than the device, IOProc under-runs. The callback should fill silence rather than block, and the engine should expose enough state for the caller to diagnose underruns later.

This makes playback behavior explicit:

- Full buffer means decode is ahead.
- Empty buffer means decode or I/O cannot keep up.
- Neither condition should cause IOProc to wait.

## 10. Format Handling

The source file's `PcmFormat` drives device configuration.

```text
FLAC 44.1 kHz / 16-bit / 2ch
  -> device nominal sample rate: 44100
  -> physical format: integer, 16-bit, 2ch

ALAC 96 kHz / 24-bit / 2ch
  -> device nominal sample rate: 96000
  -> physical format: integer, 24-bit, 2ch
```

Some DACs expose 24-bit samples inside 32-bit containers. That can still be bit-perfect if the 24 meaningful bits are placed correctly and no scaling/conversion happens. The HAL wrapper needs to model the actual physical format rather than assume file bit depth equals byte width.

The first implementation should be conservative: support the formats the Matrix DAC exposes clearly, reject unsupported combinations with `NoMatchingFormat`, and only broaden behavior after hardware validation.

## 11. Validation Strategy

Unit tests can validate parsing, decode loops, ring-buffer behavior, and error mapping. They cannot prove bit-perfect playback.

Hardware validation is required:

- Play 44.1, 48, 96, and 192 kHz files.
- Play 16-bit and 24-bit files.
- Confirm the Matrix Mini-i Pro 4 front panel shows the file's native rate/depth.
- Confirm system audio cannot mix while Pulse owns the device in hog mode.
- Check that switching between rates does not leave the device in a stale or invalid state.

A useful software-side check is a null-test style decode comparison: decode with Pulse, decode independently, and confirm the sample stream matches before it enters the HAL path. That proves the decoder path but not the OS/device path.

## 12. What Belongs Outside The Engine

Keep these out of `pulse-engine`:

- Tauri commands and event types.
- React state or UI models.
- SQLite library schema.
- Playlist, queue, and collection semantics.
- Album art, lyrics, artist bios, and enrichment APIs.
- Tag editing.
- App settings persistence.
- Commercial licensing or update logic.

Some of those features will need engine data, especially position and levels. They should consume the engine through a small adapter in the Tauri backend, not by adding app concepts to the engine crate.

## 13. Invariants

- `pulse-engine` is UI-agnostic.
- `pulse-cli` remains a first-class validation harness.
- Core Audio HAL is the output authority.
- Raw IOProc is the playback path, not AUHAL.
- Decode never runs inside the realtime callback.
- The realtime callback never allocates, locks, blocks, logs, or performs I/O.
- PCM only: FLAC, ALAC, AIFF, WAV.
- No DSD, DoP, DSF/DFF, FFmpeg, libmpv, or GPL dependencies.
- Bit-perfect claims require hardware validation.

## 14. Current Status

The crate is still a skeleton. The module boundaries are in place, but most functions are `todo!()`.

The next meaningful milestone is not a UI milestone. It is:

```text
pulse-cli play <file.flac>
```

That command should drive the Matrix Mini-i Pro 4 at the file's native rate/depth in hog mode and integer mode. Once that works reliably, the Tauri shell can build on an engine that has earned trust.
