# Pulse

Local music library player for macOS. PCM only (FLAC / ALAC / AIFF / WAV), native-rate Core Audio playback, native GPUI app shell.

## Stance

Open source under **GPLv3** (as of 2026-08-31; v0.1.0–v0.1.4 were MIT, and a brief closed-source period from 2026-08-28 was reversed before any proprietary release shipped). The client is and stays open; planned monetization is an optional cloud sync service, not the app. Source, releases, and bug reports all live in this repository — Sparkle's feed URL and the DMG download links point at this repo's Releases.

## Privacy

Bundled releases use Sparkle to fetch a signed `appcast.xml` from GitHub Releases when an automatic check is due or the user explicitly chooses **Check for Updates**. Sparkle's optional system profiling is explicitly disabled (`SUEnableSystemProfiling` is off), and Pulse adds no telemetry or unique identifiers. Turn off **Check for updates on launch** in Settings ▸ Update to disable scheduled requests; manual checks remain available.

## Layout

- `crates/pulse-engine` — standalone, UI-agnostic audio engine: AUHAL playback through `coreaudio-rs`, direct `objc2-core-audio` HAL device control, `symphonia` decode, `rtrb` ring buffer.
- `crates/pulse-app` — native GPUI desktop app shell; observes the engine in-process ([tech stack](docs/arch/tech-stack.md)).
- `design/` — Pencil (`.pen`) design files.
- `docs/` — project markdown: architecture, product scope, reference notes.

## Status

All MVP stages (1–13) are built and v0.1.4 is shipped. Work now runs through feature specs in [`docs/features/`](docs/features/) and GitHub issues.
