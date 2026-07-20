# Tech Stack

Decision date: 2026-06-03. Updated playback backend: 2026-06-14. Updated app shell to GPUI: 2026-07-20 ([impl 0007](../impls/0007-gpui-native-ui-pivot.md)).

Pulse is a native GPUI desktop app with a standalone Rust audio engine. One Rust binary — no webview, no JS toolchain, no Swift, no Zig, no libmpv, no FFmpeg, no GPL dependencies.

## Stack

| Layer | Choice |
|---|---|
| App shell + UI | GPUI (Zed's framework, Apache-2.0, crates.io), native macOS, Metal renderer |
| Design source | Pencil (`design/pulse-desktop.pen`); theme carried into Rust as data |
| Audio engine | Standalone Rust crate, UI-agnostic |
| HAL access | `objc2-core-audio` + `objc2-core-audio-types`, wrapped by Pulse |
| Output path | AUHAL / Hardware AudioUnit via `coreaudio-rs` |
| Decode | `symphonia` for FLAC, ALAC, AIFF, WAV |
| Buffering | `rtrb` lock-free SPSC ring buffer |
| Levels / spectrum | `realfft` from an audio tap in the engine |
| Media keys / Now Playing | `MPRemoteCommandCenter` / `MPNowPlayingInfoCenter` via `objc2`, from the app shell |
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
- The UI observes the engine in-process: controller state, playback events, and level/spectrum taps reach render code with no IPC or serialization boundary.
- `gpui` is a dependency (Apache-2.0); Zed's app crates are GPL — architectural reference only, never copy code.

## Why GPUI

The 2026-06-03 decision picked Tauri because the UI wedge looked faster in React/CSS and the toolchain was proven in Quill and Runner. Pivoted 2026-07-20, before any UI was built: Runner's cancelled GPUI rewrite de-risked the framework end-to-end (production rendering, IME text input, .app packaging + codesign — postmortem and salvaged code in the memory repo, `projects/runner/gpui-rewrite/`), and with zero shipped Pulse UI there is no parity cost — this is a greenfield choice.

Rust end to end fits this product: the hard part is the realtime engine, the app is macOS-only by nature (Core Audio), and GPUI's macOS-first Metal posture matches. Media keys and Now Playing are `objc2` calls from the native shell — the same ecosystem the HAL layer already uses. The open risk is the cinematic design language in paint code instead of CSS; impl 0007 fronts that risk in the first UI slice.
