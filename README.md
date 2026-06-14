# Pulse

Local music library player for macOS. PCM only (FLAC / ALAC / AIFF / WAV), native-rate Core Audio playback, Tauri 2 shell.

## Stance

Open-source first under the MIT license, especially through the engine validation and early app stages. Commercial packaging can be revisited only after Pulse proves real demand from local-library and audiophile users.

## Layout

- `src/` — React + TypeScript app shell.
- `src-tauri/` — Tauri 2 backend; adapts app commands/events to the standalone engine.
- `crates/pulse-engine` — standalone, UI-agnostic audio engine: AUHAL playback through `coreaudio-rs`, direct `objc2-core-audio` HAL device control, `symphonia` decode, `rtrb` ring buffer.
- `crates/pulse-cli` — CLI harness that drives the engine and validates playback/device behavior before any UI exists.
- `design/` — Pencil (`.pen`) design files.
- `docs/` — project markdown: architecture, product scope, reference notes.

## Status

Engine playback is proven through `pulse-cli` on the Matrix Mini-i Pro 4 using AUHAL. Current goal: design the desktop product surfaces in Pencil before adding frontend or app-settings backend code.
