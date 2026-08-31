# Pulse

Local music library player for macOS. PCM only (FLAC / ALAC / AIFF / WAV), native-rate Core Audio playback, native GPUI app shell.

## Stance

Proprietary, commercial. Pulse started open-source under MIT through engine validation and the first app releases (v0.1.0–v0.1.4); as of 2026-08-28 the source is private and the product is being prepared for paid distribution. Commits before that date remain MIT-licensed for anyone who obtained them; everything after is closed.

## Repositories

- `yicheng47/pulse-src` (private, this repo) — all source, design files, engineering issues, PRs, and CI.
- `yicheng47/pulse` (public) — releases, changelog, and customer bug reports. No code. Sparkle's feed URL and the DMG download links point here.

## Privacy

Bundled releases use Sparkle to fetch a signed `appcast.xml` from GitHub Releases when an automatic check is due or the user explicitly chooses **Check for Updates**. Sparkle's optional system profiling is explicitly disabled (`SUEnableSystemProfiling` is off), and Pulse adds no telemetry or unique identifiers. Turn off **Check for updates on launch** in Settings ▸ Update to disable scheduled requests; manual checks remain available.

## Layout

- `crates/pulse-engine` — standalone, UI-agnostic audio engine: AUHAL playback through `coreaudio-rs`, direct `objc2-core-audio` HAL device control, `symphonia` decode, `rtrb` ring buffer.
- `crates/pulse-app` — native GPUI desktop app shell; observes the engine in-process ([tech stack](docs/arch/tech-stack.md)).
- `design/` — Pencil (`.pen`) design files.
- `docs/` — project markdown: architecture, product scope, reference notes.

## Status

All MVP stages (1–13) are built and v0.1.4 is shipped. Work now runs through feature specs in [`docs/features/`](docs/features/) and GitHub issues.
