# Pulse Engine Architecture

> Companion to [`tech-stack.md`](tech-stack.md). The stack doc records technology choices; this doc explains how playback hangs together, which layer owns which responsibility, and why the boundaries exist. Rewritten 2026-09-01 after the M3 bit-perfect milestone shipped (v0.3.0); the design record behind it is archived in [`impls/archive/bit-perfect/`](../impls/archive/bit-perfect/README.md).

## 1. Overview

`pulse-engine` is the UI-agnostic playback engine. It owns the complete playback behavior: device discovery, decode (including DSD-to-DoP packing), format negotiation, the output sinks, transport state, queue-adjacent source handling, and progress/events. The GPUI app (`pulse-app`) drives the engine in-process as an adapter, not a playback owner.

There are **two engines** behind one controller, selected per device through the one-axis Output mode (Shared · Exclusive · Bit-perfect, AUTO-resolved from a capability probe):

- The **universal engine** (`engine.rs` + `auhal.rs`) plays through Core Audio's Hardware AudioUnit (AUHAL). Pulse feeds an interleaved float32 client stream and Core Audio converts to the device's physical format. It works on every output — Bluetooth, AirPods, float-only devices — and runs shared (polite, no device-wide rate switching) or exclusive (hog + native rate). Its honest claim is native-rate playback with no Pulse-side DSP. It is **not** bit-perfect: the float32 client boundary is a deliberate transform.
- The **integer engine** (`integer_engine.rs` + `raw_sink.rs`) is a raw HAL sink: `AudioDeviceCreateIOProcID` with hog mode, mixing disabled, an integer physical format, and — the decisive part — the **virtual format set equal to the integer physical format**, so the IOProc buffer takes source integers directly. Samples are never converted to float and the engine structurally cannot multiply them; volume is hardware or fixed. **This path is bit-perfect, and since 2026-09-01 the claim is proven, not aspirational**: a DoP-packed DSD64 stream played through the full path and the Matrix Mini-i Pro 4 displayed "DSD DoP 2.8MHz" — DoP markers survive only bit-exact delivery, so a single flipped bit would have broken the DSD lock.

The two-engine shape is the resolution of an old failure. Stage 0003's raw-HAL attempt produced heavy noise and forced the AUHAL pivot; the root cause, understood only during M3, was integer bytes written into a buffer whose *virtual* format was still float32. The integer engine fixes that cause instead of avoiding it, and the universal engine remains the compatibility path rather than a fallback wearing the bit-perfect label.

## 2. Runtime Picture

The runtime is a long-lived controller inside the Rust process, not a separate OS server.

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ pulse-app UI                                                                 │
│  play / pause / seek / device selection · receives state, position, errors   │
└──────────────────────────────┬───────────────────────────────────────────────┘
                               │ PlaybackCommand in · PlaybackEvent out
                               ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ pulse-engine::PlaybackController (controller thread, normal Rust code)       │
│  transport state · current source · device · decode pump · event emission    │
│                                                                              │
│  SourceDecoder ──── DecoderFactory        PlaybackBackend ── BackendFactory  │
│   PcmDecoder (symphonia)                   EngineBackend  → universal engine │
│   DsdDopDecoder (.dsf/.dff → DoP)          IntegerBackend → integer engine   │
└───────────────┬───────────────────────────────────┬──────────────────────────┘
                │ interleaved PCM bytes             │ engine API
                ▼                                   ▼
┌───────────────────────────────┐   ┌──────────────────────────────────────────┐
│ universal engine              │   │ integer engine                           │
│ engine.rs + auhal.rs          │   │ integer_engine.rs + raw_sink.rs          │
│ PCM → f32 → rtrb ring         │   │ PCM → IntPacker → rtrb ring              │
│ AUHAL render callback drains  │   │ raw IOProc drains integer bytes          │
│ float32 client stream;        │   │ virtual = physical integer format;       │
│ Core Audio converts           │   │ hog + mixing off; nothing converts       │
└───────────────────────────────┘   └──────────────────────────────────────────┘
```

SQLite, app view state, library scanning, artwork, and metadata belong outside the engine. The engine may receive a `PathBuf`, a source ID, or a device ID; it must not know about album pages or GPUI types.

## 3. Audio Terms In Software Terms

**PCM** is an array of samples, interleaved for stereo. A "frame" is one sample per channel.

**Sample rate** is frames per second the DAC consumes. If file and device rates differ, something must resample; Pulse treats native-rate switching as core behavior (exclusive/bit-perfect modes switch the device; shared mode leaves the device clock alone and lets AUHAL resample).

**Bit depth** is the integer width of each source sample — 16 or 24 for the PCM library formats.

**Virtual vs physical format**: the physical format is what the hardware side of the stream runs; the virtual format is what the process-facing buffer carries. They can differ — that gap is exactly where 0003's noise came from, and closing it (virtual = physical integer) is the integer engine's premise.

**Hog mode** is exclusive device ownership — a lease preventing other processes from sharing the output while Pulse plays. It dies with the process.

**DoP** packs the DSD bitstream into 24-bit PCM frames — 16 DSD bits per channel per frame under an alternating `0x05`/`0xFA` marker byte. The DAC recognizes the markers and unpacks the original DSD stream. Because any sample-value change destroys the markers, DoP doubles as a bit-exactness proof and is why DSD playback is gated to the integer engine.

**Bit-perfect** means no layer transforms sample values between decode and the DAC. In Pulse this claim belongs to the integer engine only; the universal engine does not make it.

## 4. Thread Model

Four execution contexts.

### 4.1 Adapter Thread

`pulse-app` translates UI intent into `PlaybackCommand` and events back into display state. It owns no playback state beyond display state.

### 4.2 Playback Controller Thread

Normal Rust code: owns the state machine (idle / loading / playing / paused / ended / error), the current source and decoder, the selected device and backend, the decode pump, and event emission. It decides when to open, hold, or rebuild an engine.

### 4.3 Decode And Packing Work

Blocking, allocating work: symphonia decodes containers to interleaved integer PCM; `DsdDopDecoder` parses DSF/DFF and emits DoP frames. Feeding converts to the sink's wire format — float32 for the universal engine, packed integers via `IntPacker` (driven by the probed format flags, not assumptions) for the integer engine — and pushes into a bounded `rtrb` ring.

### 4.4 Realtime Callback

Both sinks end in a realtime callback — the AUHAL render callback or the raw IOProc. The rules are identical and non-negotiable: read pre-packed bytes from the ring consumer, copy into the output buffer, zero-fill underruns, update atomics, return. No allocation, no locks, no syscalls, no waiting, no unbounded work. Violations are audible.

## 5. Crate Layout

```text
crates/pulse-engine/src/
  lib.rs            public surface, PcmFormat
  controller.rs     PlaybackController: state machine, decode pump, backend/decoder seams
  command.rs        PlaybackCommand, EngineKind
  event.rs          PlaybackEvent, PlaybackErrorKind, VolumeDomain/VolumeState
  state.rs          PlaybackState
  source.rs         PlayableSource
  decode.rs         symphonia decode: FLAC/ALAC/AIFF/WAV → integer PCM
  decode_dsd.rs     DSF/DFF parsers + DoP packer (feature 71)
  device.rs         output-device discovery and identity
  hal.rs            all unsafe Core Audio property FFI: hog, formats, rates, listeners
  engine.rs         universal engine: format negotiation + AUHAL lifecycle
  auhal.rs          AudioUnit render-callback sink (float32 client stream)
  integer_engine.rs bit-perfect engine: probe-gated open, IntPacker, IOProc lifecycle
  raw_sink.rs       raw IOProc callback + ring consumer
  gain.rs           software volume for the universal path (unity default)
  levels.rs         playback analysis tap
  error.rs          EngineError
```

## 6. Public Boundary

The product API is controller-oriented: `PlaybackController::spawn`, commands in, events out. Command, event, and state shapes live in `command.rs`, `event.rs`, and `state.rs` — this doc deliberately doesn't duplicate them. The behavior contract is the stable part: commands are imperative, events are observable facts, and UI state is derived from events.

`EngineKind` is part of the command surface: the app resolves each device's stored Output mode to a kind, and the controller opens the matching backend.

## 7. The Two Seams Inside The Controller

The controller abstracts both of its variable ends behind private traits with factories, each with a fake implementation for tests:

- **`SourceDecoder`** (`format` / `duration_ms` / `seek` / `next_pcm`) behind a `DecoderFactory` that routes by extension: `.dsf`/`.dff` → `DsdDopDecoder`, everything else → symphonia's `PcmDecoder`.
- **`PlaybackBackend`** (`start` / `feed` / `position` / `underrun_frames` / volume surface / `stop` / `retains_device_when_paused` / `release`) behind a `BackendFactory` keyed by `EngineKind`. `EngineBackend` adapts the universal engine; `IntegerBackend` adapts the integer engine. The wrappers keep controller vocabulary out of the engine modules.

Both traits are deliberately private: the controller is their only consumer, and the pattern stays consistent across the crate. Promote one only when a second consumer or a third implementation is real.

## 8. Module Responsibilities

- **`controller.rs`** — owns transport behavior: command receiver, event sender, state machine, decode pump, backend lifecycle (open / hold / rebuild on device- or format-change), end-of-track and gapless advance (`SetNext`), position and dropout reporting.
- **`state.rs` / `source.rs`** — state snapshots and the engine-level playable input; they know paths and durations, never SQLite rows or UI types.
- **`device.rs`** — discovery and identity (id, uid, name) only; no playback state.
- **`decode.rs`** — symphonia: open, probe native format, decode to interleaved integer PCM, accurate seek, `EngineError::Decode` on failure.
- **`decode_dsd.rs`** — DSF (planar blocks, LSB-first bit reversal) and DFF (chunked, MSB-first) to DoP frames; refuses DST and MSB-first DSF with clear errors rather than risking noise.
- **`hal.rs`** — the entire unsafe Core Audio property surface behind typed `Result` helpers: hog acquire/release, nominal rate, physical/virtual formats, mixing, listeners. Rate and format switches are async; the wrappers wait on property listeners before trusting new state.
- **`engine.rs` / `auhal.rs`** — the universal engine: nominal-rate handling per mode, float32 packing, AUHAL sink lifecycle, software gain hook.
- **`integer_engine.rs` / `raw_sink.rs`** — the bit-perfect engine; see §9.
- **`gain.rs`** — software volume for the universal path only; the integer path has no gain stage by construction.
- **`levels.rs`** — analysis tap; must never slow playback.

## 9. The Bit-Perfect Design

Design-level summary of the M3 milestone; the staged record with hardware findings lives in [`impls/archive/bit-perfect/`](../impls/archive/bit-perfect/README.md) and its `IMPL_LOG.md`.

- **Premise**: set the stream's virtual format equal to an integer physical format, so the IOProc buffer carries source integers untouched. This is the direct fix for 0003's root cause.
- **Probe gate**: not every device accepts an integer virtual format. A capability probe answers per device; devices that refuse resolve to the universal engine under AUTO. The milestone's stage 1 was explicitly a gate — "not possible on this OS/device" would have been recorded as the honest outcome rather than shipping a float engine with the label.
- **Device state discipline**: hog, mixing, and both formats are mutated under RAII guards (`HogGuard`, `FormatRestoreGuard`) that restore prior state on release. Hog dies with the process; formats can persist across a crash — a documented, accepted risk.
- **Purity**: the integer path has no multiply. Volume is the device's hardware control or fixed at 100% (feature 31's volume domains and Signal Path verdicts surface this honestly in the app).
- **Pause holds the device** (`retains_device_when_paused`): pausing keeps hog and the negotiated format instead of tearing the sink down. Whether a DAC holds its DSD lock through the data gap of a pause is the DAC's business, not the engine's — feature 71 phase 4 still owes that observation on the Matrix.
- **Proof**: the DoP acceptance. A DSD64 stream DoP-packed into ordinary PCM frames played end to end and the Matrix reported the DSD stream intact. DSD playback (feature 71) now rides this property in production: DSD tracks are refused on any path that cannot guarantee bit-exact delivery, because corrupted DoP is loud hiss.

## 10. Backpressure And Underruns

Decode is the producer; the realtime callback is the consumer; the ring is bounded on purpose. When the producer outruns the device, `feed` accepts fewer frames and the pump retries. When it falls behind, the callback zero-fills and counts the underrun — it never waits. The controller reports dropouts only while data is owed; UI sees state, position, and dropout events, never the feeding mechanics.

## 11. Pause, Resume, And Seek Semantics

Pause preserves the logical position. On the universal path the sink may be rebuilt on resume; on the integer path the device is held (§9) and resume continues without renegotiation. Seek re-seeks the decoder (accurately, decoding forward to the target), resets the ring, and resumes per prior state; the DoP packer restarts its marker phase on every seek. Format changes between tracks take an engine rebuild; same-format boundaries advance gaplessly via `SetNext`.

## 12. Format Examples

Universal engine, 24/96 FLAC: decode to i24, sign-extend and scale each sample to f32, AUHAL converts the float32 client stream to the device's physical format.

Integer engine, 24/96 FLAC: decode to i24, `IntPacker` lays samples into the device's probed integer physical format, the IOProc copies bytes — no float, no scaling, no gain.

Integer engine, DSD64 DFF: `DsdDopDecoder` emits 176.4kHz/24-bit DoP frames (16 DSD bits + marker per channel per frame); to the engine it is ordinary PCM; the DAC unpacks the DSD stream.

## 13. Testing Strategy

Unit tests cover parsing, state transitions, packing, ring behavior, and error mapping — byte-exact fixtures for the DoP packer, generated by `script/generate_dsd_fixtures.py`. Controller behavior is tested against `FakeBackend`/`FakeDecoder` through the §7 seams; hardware is never required for transport logic.

Hardware validation matches the claim being made. The universal engine validates clean native-rate playback and device etiquette. The integer engine validates bit-exactness — the DoP test is the standing proof, re-runnable whenever the path changes: if the Matrix shows the DSD rate, delivery is bit-exact; if it shows PCM or noise, it is not.

## 14. Non-Goals

- DST decompression, SACD ISO, native/raw DSD output — DSD exists only as DoP-packed PCM (feature 71).
- DSD→PCM conversion: unplayable-on-this-device DSD refuses with a clear error instead.
- Video, streaming integrations, libmpv/FFmpeg/GPL audio dependencies.
- App UI owning playback state; GPUI or `pulse-app` types inside `pulse-engine`; SQLite inside the engine.

Current build order lives in [`docs/roadmap.md`](../roadmap.md) — start at its `## Now` section.
