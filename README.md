# Pulse

Local music library player for macOS with a **proven bit-perfect engine**. FLAC / ALAC / AIFF / WAV, plus DSD — DSF / DFF containers, DSD64 and DSD128, uncompressed only (no DST, no SACD ISO) — delivered as DoP; native GPUI app shell. Two Core Audio paths per device: a universal AUHAL engine that plays anywhere, and a hog-mode raw-HAL integer engine whose bit-exact delivery is hardware-proven — the DoP acceptance test lit up "DSD DoP 2.8MHz" on the DAC, which only intact bits can do.

## Stance

Open source under **GPLv3** (as of 2026-08-31; v0.1.0–v0.1.4 were MIT, and a brief closed-source period from 2026-08-28 was reversed before any proprietary release shipped). The client is and stays open; planned monetization is an optional cloud sync service, not the app. Source, releases, and bug reports all live in this repository — Sparkle's feed URL and the DMG download links point at this repo's Releases.

## Privacy

Bundled releases use Sparkle to fetch a signed `appcast.xml` from GitHub Releases when an automatic check is due or the user explicitly chooses **Check for Updates**. Sparkle's optional system profiling is explicitly disabled (`SUEnableSystemProfiling` is off), and Pulse adds no telemetry or unique identifiers. Turn off **Check for updates on launch** in Settings ▸ Update to disable scheduled requests; manual checks remain available.

## Layout

- `crates/pulse-engine` — standalone, UI-agnostic audio engine: two sinks (universal AUHAL via `coreaudio-rs`; bit-perfect raw-HAL integer IOProc), direct `objc2-core-audio` HAL device control, `symphonia` decode plus DSF/DFF-to-DoP packing, `rtrb` ring buffer. Architecture: [`docs/arch/pulse-engine.md`](docs/arch/pulse-engine.md).
- `crates/pulse-app` — native GPUI desktop app shell; observes the engine in-process ([tech stack](docs/arch/tech-stack.md)).
- `design/` — Pencil (`.pen`) design files.
- `docs/` — project markdown: architecture, product scope, reference notes.

## Status

v0.3.0 shipped 2026-09-01 — the bit-perfect release: the integer engine proven bit-exact by the DoP test, one-axis Output mode (Shared · Exclusive · Bit-perfect), volume transparency. DSD (DSF/DFF over DoP) landed on `main` right after. Work runs through feature specs in [`docs/features/`](docs/features/) with matching GitHub issues; current state lives in [`docs/roadmap.md`](docs/roadmap.md)'s `Now` section.
