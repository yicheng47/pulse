# Learning Material

Pulse's unfamiliar domains are realtime audio constraints, Core Audio HAL/AUHAL, and validating playback behavior on real DACs. Rust, Tauri, and React are familiar enough; learn the audio path first.

## 1. Audio Fundamentals

- PCM only. Pulse targets multi-bit PCM, not DSD.
- A stream is defined by sample rate, bit depth, and channels.
- Native-rate playback means the device is switched to the file's sample rate instead of relying on system resampling.
- Bit-perfect means integer samples reach the DAC unchanged: no resampling, no software mixing, no volume scaling, no float round-trip. That is a future raw-HAL validation target, not the current AUHAL claim.
- The DAC's own front-panel indicator is the ground truth for sample-rate validation.

## 2. Realtime Audio Programming

The OS calls the AudioUnit render callback on a high-priority realtime thread and expects the next buffer by a hard deadline. Missing that deadline creates audible glitches.

Inside the callback: no allocation, no locks, no syscalls, no I/O, no unbounded work.

The architecture is decode thread to PCM-to-float packing to lock-free SPSC ring buffer to AUHAL render callback. The callback only drains already-packed float32 frames into the device buffer.

Primary read: Ross Bencina, "Real-time audio programming 101: time waits for nothing".

## 3. Core Audio HAL And AUHAL

The current playback path uses AUHAL / Hardware AudioUnit through `coreaudio-rs`. Pulse still uses `objc2-core-audio` and `objc2-core-audio-types` directly for the HAL control plane.

Official API reference: Apple's Core Audio documentation at https://developer.apple.com/documentation/coreaudio.

Learn these operations first:

- Device enumeration and hot-plug listeners.
- Hog mode via `kAudioDevicePropertyHogMode`.
- Per-track sample-rate switching via `kAudioDevicePropertyNominalSampleRate`.
- Physical-format probing via `kAudioStreamPropertyPhysicalFormat`.
- Hardware AudioUnit creation, `kAudioUnitProperty_StreamFormat`, and render callbacks.

## 4. Rust Audio Crates

- `objc2-core-audio` and `objc2-core-audio-types` for Core Audio bindings.
- `coreaudio-rs` for the Hardware AudioUnit playback wrapper.
- `symphonia` for deterministic PCM decode.
- `rtrb` for decode-thread to render-callback handoff.
- `realfft` for VU/spectrum analysis.
- `lofty` later for metadata read/write.
- `rusqlite` later for SQLite + FTS5.

## 5. Proving Playback Behavior

Validation must happen on hardware.

- Play 44.1, 48, 96, and 192 kHz files.
- Play 16-bit and 24-bit files.
- Confirm the Matrix Mini-i Pro 4 shows each file's native sample rate.
- Confirm hog mode excludes system audio while Pulse owns the device.
- Confirm playback is clean, not noisy, and has expected volume.

Failure modes to guard against now: silent OS resampling, underruns, float/int client-format mismatches, and system-mixer output.

Future bit-perfect validation additionally needs proof that integer samples reach the DAC unchanged.

## 6. Tauri Bridge

The engine stays in Rust and exposes a clean API. Tauri commands should call that API and publish UI events.

For meters and spectrum, compute small arrays in Rust and send them to the webview at roughly 30-60 Hz. Do not try to make UI animation part of the realtime audio path.

## 7. Reference Repos

Tier 1, Core Audio playback:

- mpv `ao_coreaudio_exclusive.c` for hog, integer, and format switching behavior.
- CamillaDSP CoreAudio backend for a Rust AUHAL float32 playback reference.
- Cog for a complete macOS audiophile player reference. Read only; it is GPL.

Tier 2, decode and playback architecture:

- Symphonia examples.
- cpal CoreAudio backend as a calling-pattern reference only.
- Local `coreaudio-rs` crate source after `cargo fetch`; it currently uses the same `objc2-core-audio` / `objc2-core-audio-types` binding family underneath.
- `rust_cli_musicplayer` for a small CLI decode/playback shape.

Tier 3, Tauri app shell references:

- Audion for a Tauri 2 offline local library shape.
- Sonus for Tauri local/NAS music library ideas.
- `arjav0703/music-app` for a small Tauri + React music app.
