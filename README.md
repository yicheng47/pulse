# Pulse

Bit-perfect local music library player for macOS. PCM only (FLAC / ALAC / AIFF / WAV), Core Audio HAL exclusive + integer mode, Tauri 2 shell.

## Stance

Open-source first under the MIT license, especially through the engine validation and early app stages. Commercial packaging can be revisited only after Pulse proves real demand from local-library and audiophile users.

## Layout

- `src/` — React + TypeScript app shell.
- `src-tauri/` — Tauri 2 backend; adapts app commands/events to the standalone engine.
- `crates/pulse-engine` — standalone, UI-agnostic audio engine: `objc2-core-audio` HAL wrapper, raw IOProc, `symphonia` decode, `rtrb` ring buffer.
- `crates/pulse-cli` — CLI harness that drives the engine; validates bit-perfect output against the DAC before any UI exists.
- `design/` — Pencil (`.pen`) design files.
- `docs/` — project markdown: architecture, product scope, reference notes.

## Status

Engine and app-shell skeleton only. Current goal: `pulse-cli play <file.flac>` driving the Matrix Mini-i Pro 4 at the source's native rate/depth in hog + integer mode, confirmed on the DAC's own indicator.
