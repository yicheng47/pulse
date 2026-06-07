# Tech Stack

Decision date: 2026-06-03. Updated HAL binding choice: 2026-06-07.

Pulse is a Tauri 2 desktop app with a standalone Rust audio engine. No Swift, no Zig, no libmpv, no FFmpeg, no GPL dependencies.

## Stack

| Layer | Choice |
|---|---|
| App shell | Tauri 2, Rust backend, WKWebView |
| UI | React + TypeScript |
| Audio engine | Standalone Rust crate, UI-agnostic |
| HAL access | `objc2-core-audio` + `objc2-core-audio-types`, wrapped by Pulse |
| Output path | Raw Core Audio HAL IOProc via `AudioDeviceCreateIOProcID` |
| Decode | `symphonia` for FLAC, ALAC, AIFF, WAV |
| Buffering | `rtrb` lock-free SPSC ring buffer |
| Levels / spectrum | `realfft` from an audio tap in the engine |
| Tags | `lofty` later for tag read/write |
| Library DB | SQLite + FTS5 later, likely `rusqlite` |
| Enrichment | `reqwest` later for MusicBrainz, Cover Art Archive, Last.fm |

## Key Decisions

- The engine is standalone and testable from `pulse-cli`.
- The app shell is not allowed to own playback correctness.
- The realtime IOProc callback must allocate nothing, lock nothing, and perform no syscalls.
- Decode happens off the realtime thread and feeds the IOProc through `rtrb`.
- The output path is raw HAL IOProc, not AUHAL, `cpal`, or `coreaudio-rs` as a dependency.
- `cpal` and `coreaudio-rs` can be read as references only.
- PCM only for v1: FLAC, ALAC, AIFF, WAV. No DSD, no DoP, no DSF/DFF.

## Why Tauri

The hard part is the audio engine, and Rust is the better fit for realtime constraints than Swift because it avoids ARC traffic and hidden allocation in the callback path. The UI wedge is a cinematic now-playing and library experience, which is faster to ship in React/CSS. Tauri also reuses the toolchain already proven in Quill and Runner.

Swift's advantages, such as native media-key polish and Now Playing integration, are shell details that can be handled later from the Tauri backend.
