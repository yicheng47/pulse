# Product Vision

Pulse is a bit-perfect local music library player for macOS. It is music-only: no video, no streaming integration, no DSD.

The target is the local FLAC listener who wants Audirvana-level output correctness without Audirvana's pricing and UI. The product wedge is simple: beautiful, fast, bit-perfect playback for owned music libraries.

## Commercialization Stance

Pulse is open-source first, especially during the initial engine, design, and product-validation stage.

The project may still become commercial later, but only after the bit-perfect proof and early UI attract real demand from local-library and audiophile users. Until then, optimize for credibility, inspectable engineering, and fast feedback rather than licensing complexity or paid packaging.

No GPL dependencies remains a constraint. Open-source does not mean giving up the option for commercial distribution later.

## Scope

- Local and NAS music libraries.
- PCM formats only: FLAC, ALAC, AIFF, WAV.
- Bit-perfect playback through Core Audio HAL exclusive mode, integer mode, and per-track sample-rate switching.
- Fast scanning and sub-100ms search across large libraries.
- Grid/list browsing, collections, smart collections, and metadata editing.
- Cinematic now-playing with album-art color extraction, synced LRC lyrics, real VU/spectrum, artist context, and next-track prefetching.
- Editorial enrichment from MusicBrainz, Cover Art Archive, Last.fm, Discogs, or Wikipedia.
- Smart Radio from the user's own library.

## Agent Harness Direction

`pulse-cli` should remain a deterministic, scriptable engine harness even after the desktop app exists. It is the place to prove device visibility, file probing, hog mode, format switching, playback validation, and diagnostics without a UI in the way.

That also makes `pulse-cli` the natural future boundary for agent and MCP workflows. The command surface should support stable JSON output, stable exit codes, bounded playback tests, and machine-readable error types so an agent can list devices, probe files, run format validation, play a short test clip, and collect diagnostics safely.

A TUI can be added later if it helps human debugging, but it should not replace the boring command surface. The CLI's first job is to be a reliable harness for engineers and agents.

## Non-Goals

- No DSD.
- No video.
- No streaming-service integration.
- No libmpv.
- No FFmpeg.
- No GPL dependencies.
- No iOS/tvOS app for v1.

## Build Order

1. Prove the audio engine with `pulse-cli play <file>` and the Matrix Mini-i Pro 4 showing the file's native rate/depth.
2. Design the library, now-playing, search, and settings surfaces in Pencil.
3. Scaffold the Tauri app shell following the Runner-style layout.
4. Build the library scanner, SQLite store, and FTS search.
5. Add metadata enrichment.
6. Build now-playing.
7. Add Smart Radio v1.
8. Polish and package.
