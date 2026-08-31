# Product Vision

Pulse is a high-quality local music library player for macOS. It is music-only: no video playback, no video library support, no streaming integration, no DSD.

The target is the local FLAC listener who wants Audirvana-level output care without Audirvana's pricing and UI. The product wedge is simple: beautiful, fast, native-rate playback for owned music libraries.

Bit-perfect integer output remains a possible future differentiator, but it should not be claimed until a lower-level path is rebuilt and validated on hardware.

The first release scope is defined separately in [`mvp.md`](mvp.md). This vision doc is intentionally broader than the MVP.

## Commercialization Stance

Pulse is a commercial, closed-source product as of 2026-08-28. It was open-source under MIT through engine validation and the first app releases (v0.1.0–v0.1.4), which is what that phase was for: credibility, inspectable engineering, and fast feedback while the playback path was unproven.

Distribution plan: direct sales first (Developer ID + notarization + Sparkle already ship this way; a merchant-of-record handles tax), one-time purchase with paid major versions rather than a subscription. The Mac App Store is a possible second channel later, but it would require dropping Sparkle and proving exclusive-mode Core Audio under the App Sandbox, so it is not on the path to v1.0.

No GPL dependencies remains a constraint; every current dependency is MIT/Apache/MPL-2.0 and compatible with closed distribution. `symphonia` is MPL-2.0, which only obliges publishing modifications to symphonia's own files — do not vendor-patch it.

## Scope

- Local and NAS music libraries.
- PCM formats only: FLAC, ALAC, AIFF, WAV.
- Native-rate playback through Core Audio AUHAL, with direct HAL control for hog mode, sample-rate switching, and physical-format diagnostics.
- Fast scanning and sub-100ms search across large libraries.
- Grid/list browsing, collections, smart collections, and metadata editing.
- Cinematic now-playing with album-art color extraction, synced LRC lyrics, real VU/spectrum, artist context, and next-track prefetching.
- Editorial enrichment from MusicBrainz, Cover Art Archive, Last.fm, Discogs, or Wikipedia.
- Smart Radio from the user's own library.

## Non-Goals

- No DSD.
- No video playback or video library support.
- No streaming-service integration.
- No libmpv.
- No FFmpeg.
- No GPL dependencies.
- No iOS/tvOS app for v1.

## Build Order

1. Prove the audio engine with a validation CLI and the Matrix Mini-i Pro 4 showing the file's native sample rate with clean sound.
2. Design the library, now-playing, search, and settings surfaces in Pencil.
3. Scaffold the native GPUI app shell, `crates/pulse-app` ([tech stack](../arch/tech-stack.md)).
4. Build the library scanner, SQLite store, and FTS search.
5. Add metadata enrichment.
6. Build now-playing.
7. Add Smart Radio v1.
8. Polish and package.
