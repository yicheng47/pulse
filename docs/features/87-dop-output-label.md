# Playback Row — DoP Output Label

Feature 87 · P3 · GitHub issue [#87](https://github.com/yicheng47/pulse/issues/87). Jason, 2026-09-04, on the row reading `176.4 kHz · mini-i Series` under `DFF · DSD64`: "I think it should show DoP, instead of 176 right?"

## Motivation

The playback row's output line comes from `format_output_device(sample_rate, device_name, mode)` (`crates/pulse-app/src/backend/playback/logic.rs`), which prints the running `PcmFormat`'s sample rate. For a DSD source that format is the DoP carrier — DSD64 packs 16 one-bit samples into each 24-bit frame, so 2.8224 MHz becomes 176.4 kHz — and the line reads like a PCM rate beside a source badge that says DSD64. The badge already knows the source is DSD (`format_quality` checks the `.dsf`/`.dff` extension); the output line doesn't, so the user has to know the DoP arithmetic to see why the numbers differ.

## Scope

- **Name the carrier.** For a DSD source the output line reads `DoP 176.4 kHz · mini-i Series` (DSD128: `DoP 352.8 kHz`). The rate stays: it is what the DAC's own display shows and what a user checks when the lock fails. PCM sources are unchanged.
- **Same source rule as the badge.** DSD detection reuses `format_quality`'s extension check on the row's `source_path`; no new state. A DSD source only ever runs on the integer path (the DoP gate), so the Shared wording never needs a DoP variant.
- **Signal Path too.** The popover's output line (`surfaces/playback_popovers.rs`, the integer-path arm of `output_detail`) reads `mini-i Series · DoP 24/176.4 integer` for a DSD source.
- **Album badge container.** The album header's quality badge formats the album's quality without a file path, so a DSD album reads `PCM DSD64` (Jason's screenshot, 2026-09-04). Pass the container from the album's tracks — `DFF DSD64` / `DSF DSD64` — so the badge matches the row's `DFF · DSD64`.
- **No design pass.** Copy-only change inside existing text runs; the row and popover layouts are untouched.

## Non-Goals

- The source badge (`DFF · DSD64`), the DoP gate, the engine, the Devices page capability line, and feature 71's DSD boards.
- Detecting DoP-packed DSD inside PCM containers (a 176.4/24 FLAC carrying DoP frames is PCM to Pulse and stays labelled as such).

## Implementation Phases

1. `format_output_device` takes the source path (or a `dop: bool` derived from it) and prefixes `DoP ` for DSD sources; the Signal Path output detail does the same. Unit tests: `.dff` at 176 400 → `DoP 176.4 kHz · mini-i Series`; `.dsf` at 352 800 → `DoP 352.8 kHz · …`; `.flac` at 176 400 → `176.4 kHz · …` unchanged; the Signal Path string for a DSD source.

## Verification

- `make verify` green with the tests above.
- Manual on the Matrix in Exclusive: play a DSD64 file — the row reads `DFF · DSD64` over `DoP 176.4 kHz · mini-i Series`, Signal Path's output line says DoP, and the DAC display shows DSD. A PCM track right after shows the plain rate again.
