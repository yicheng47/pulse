# Learning Material

Pulse's unfamiliar domains are realtime audio constraints, Core Audio HAL, and proving bit-perfect playback. Rust, Tauri, and React are familiar enough; learn the audio path first.

## 1. Audio Fundamentals

- PCM only. Pulse targets multi-bit PCM, not DSD.
- A stream is defined by sample rate, bit depth, and channels.
- Bit-perfect means integer samples reach the DAC unchanged: no resampling, no software mixing, no volume scaling, no float round-trip.
- The DAC's own front-panel indicator is the ground truth.

## 2. Realtime Audio Programming

The OS calls the IOProc callback on a high-priority realtime thread and expects the next buffer by a hard deadline. Missing that deadline creates audible glitches.

Inside the callback: no allocation, no locks, no syscalls, no I/O, no unbounded work.

The architecture is decode thread to lock-free SPSC ring buffer to IOProc. The IOProc only drains already-decoded PCM into the device buffer.

Primary read: Ross Bencina, "Real-time audio programming 101: time waits for nothing".

## 3. Core Audio HAL

The bit-perfect macOS path goes through Core Audio HAL. Pulse uses `objc2-core-audio` and `objc2-core-audio-types` for bindings, then wraps the property API itself.

Official API reference: Apple's Core Audio documentation at https://developer.apple.com/documentation/coreaudio.

Learn these HAL operations first:

- Device enumeration and hot-plug listeners.
- Hog mode via `kAudioDevicePropertyHogMode`.
- Integer mode via `kAudioStreamPropertyPhysicalFormat`.
- Per-track sample-rate switching via `kAudioDevicePropertyNominalSampleRate`.
- Raw IOProc creation via `AudioDeviceCreateIOProcID` and `AudioDeviceStart`.

## 4. Rust Audio Crates

- `objc2-core-audio` and `objc2-core-audio-types` for Core Audio bindings.
- `symphonia` for deterministic PCM decode.
- `rtrb` for decode-thread to IOProc handoff.
- `realfft` for VU/spectrum analysis.
- `lofty` later for metadata read/write.
- `rusqlite` later for SQLite + FTS5.

## 5. Proving Bit-Perfect

Validation must happen on hardware.

- Play 44.1, 48, 96, and 192 kHz files.
- Play 16-bit and 24-bit files.
- Confirm the Matrix Mini-i Pro 4 shows each file's native rate/depth.
- Confirm hog mode excludes system audio while Pulse owns the device.
- Add a null-test style check for decoded samples where practical.

Failure modes to guard against: silent OS resampling, float conversion, and system-mixer output.

## 6. Tauri Bridge

The engine stays in Rust and exposes a clean API. Tauri commands should call that API and publish UI events.

For meters and spectrum, compute small arrays in Rust and send them to the webview at roughly 30-60 Hz. Do not try to make UI animation part of the realtime audio path.

## 7. Reference Repos

Tier 1, bit-perfect core:

- mpv `ao_coreaudio_exclusive.c` for hog, integer, and format switching behavior.
- CamillaDSP CoreAudio backend for a Rust exclusive-output reference.
- Cog for a complete macOS audiophile player reference. Read only; it is GPL.

Tier 2, decode and playback architecture:

- Symphonia examples.
- cpal CoreAudio backend as a calling-pattern reference only.
- `rust_cli_musicplayer` for a small CLI decode/playback shape.

Tier 3, Tauri app shell references:

- Audion for a Tauri 2 offline local library shape.
- Sonus for Tauri local/NAS music library ideas.
- `arjav0703/music-app` for a small Tauri + React music app.
