# Tech Stack

Decision date: 2026-06-03. Updated playback backend: 2026-06-14.

Pulse is a Tauri 2 desktop app with a standalone Rust audio engine. No Swift, no Zig, no libmpv, no FFmpeg, no GPL dependencies.

## Stack

| Layer | Choice |
|---|---|
| App shell | Tauri 2, Rust backend, WKWebView |
| UI | React + TypeScript |
| Audio engine | Standalone Rust crate, UI-agnostic |
| HAL access | `objc2-core-audio` + `objc2-core-audio-types`, wrapped by Pulse |
| Output path | AUHAL / Hardware AudioUnit via `coreaudio-rs` |
| Decode | `symphonia` for FLAC, ALAC, AIFF, WAV |
| Buffering | `rtrb` lock-free SPSC ring buffer |
| Levels / spectrum | `realfft` from an audio tap in the engine |
| Tags | `lofty` later for tag read/write |
| Library DB | SQLite + FTS5 later, likely `rusqlite` |
| Enrichment | `reqwest` later for MusicBrainz, Cover Art Archive, Last.fm |

## Key Decisions

- The engine is standalone and testable from `pulse-cli`.
- The app shell is not allowed to own playback correctness.
- The realtime AudioUnit render callback must allocate nothing, lock nothing, and perform no syscalls.
- Decode happens off the realtime thread and feeds the AUHAL callback through `rtrb`.
- Direct HAL remains the control plane for output-device listing, hog mode, nominal sample-rate switching, and physical-format diagnostics.
- The default playback path is AUHAL float32 because raw IOProc integer playback hit device/client-format behavior that produced noisy output on Matrix hardware.
- PCM only for v1: FLAC, ALAC, AIFF, WAV. No DSD, no DoP, no DSF/DFF.

## Why Tauri

The hard part is the audio engine, and Rust is the better fit for realtime constraints than Swift because it avoids ARC traffic and hidden allocation in the callback path. The UI wedge is a cinematic now-playing and library experience, which is faster to ship in React/CSS. Tauri also reuses the toolchain already proven in Quill and Runner.

Swift's advantages, such as native media-key polish and Now Playing integration, are shell details that can be handled later from the Tauri backend.
