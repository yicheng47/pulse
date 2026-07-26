# Pulse

Local music library player for macOS. PCM only (FLAC / ALAC / AIFF / WAV), native-rate Core Audio playback, native GPUI app shell.

## Stance

Open-source first under the MIT license, especially through the engine validation and early app stages. Commercial packaging can be revisited only after Pulse proves real demand from local-library and audiophile users.

## Layout

- `crates/pulse-engine` — standalone, UI-agnostic audio engine: AUHAL playback through `coreaudio-rs`, direct `objc2-core-audio` HAL device control, `symphonia` decode, `rtrb` ring buffer.
- `crates/pulse-cli` — CLI harness that drives the engine and validates playback/device behavior before any UI exists.
- `crates/pulse-app` — native GPUI desktop app shell; observes the engine in-process ([tech stack](docs/arch/tech-stack.md)).
- `design/` — Pencil (`.pen`) design files.
- `docs/` — project markdown: architecture, product scope, reference notes.

## Status

Engine playback is proven through `pulse-cli` on the Matrix Mini-i Pro 4 using AUHAL, the engine-owned playback controller has landed, and the desktop product surfaces are designed in Pencil. Current goal: the playback row surface and controller wiring in `pulse-app` (stage 7).
