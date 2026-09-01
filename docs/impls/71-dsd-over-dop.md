# 71 — DSD playback over DoP

> Impl note for [feature 71](../features/71-dsd-over-dop.md) (issue [#71](https://github.com/yicheng47/pulse/issues/71)). First impl note on the issue-number scheme. The engine does not change: DSD enters as `.dsf`/`.dff` files and leaves the decode layer as ordinary DoP-marked 24-bit PCM; the v0.3.0 integer engine already delivers that bit-exactly (stage-5 pass, Matrix read "DSD DoP 2.8MHz").

## Container coverage

DSF and DFF/DSDIFF are the DSD market: every download store (NativeDSD, Blue Coast, 2L, Qobuz DSD) ships one of the two. Supporting both, uncompressed only, covers "most major DSD containers"; the deliberate refusals — DST-compressed DFF, SACD ISO, WavPack DSD — are rips and niches, not retail formats, and stay non-goals per the spec.

## Where it plugs in

- `crates/pulse-engine/src/controller.rs:28` — `DecoderFactory` currently hardwires `PcmDecoder::open` for every path. Route by extension: `.dsf`/`.dff` → the new `DsdDopDecoder`; everything else stays symphonia. The private `SourceDecoder` trait (`format` / `duration_ms` / `seek` / `next_pcm`, controller.rs:255) is the whole contract — the decoder is a sibling of `decode.rs`, no controller changes beyond the factory.
- New `crates/pulse-engine/src/decode_dsd.rs` — container parsing + DoP packing. Reports `PcmFormat` at the DoP rate (DSD64 → 176.4kHz/24, DSD128 → 352.8kHz/24); `next_pcm` emits frames with the alternating `0x05`/`0xFA` marker byte; `seek` maps ms → DSD byte offset aligned to a whole DoP frame and restarts the marker phase. DSF: 4096-byte per-channel blocks, LSB-first → 256-entry bit-reversal LUT. DFF: FRM8/PROP(FS/CHNL/CMPR)/DSD chunks, MSB-first, byte-interleaved; `CMPR` ≠ `DSD ` → refuse with a clear `EngineError::Decode`.
- `script/dop_pack.py` — the stage-5 DFF packer is the reference implementation; the Rust packer must match it byte-exactly and fixtures are generated with it (small hand-built DSF/DFF headers + known bit patterns, committed as tiny fixture files, no real music in the repo).
- `crates/pulse-app/src/backend/scan/walk.rs:202` — add `"dsf" | "dff"` to the supported-extension match (the `ignore.dsf` walk test flips meaning and moves to the positive set).
- `crates/pulse-app/src/backend/scan/metadata.rs` — symphonia cannot probe these containers: hand-read the header for rate/channels/duration; DSF metadata from the trailing ID3v2 chunk the header points at; tagless DFF falls back to filename parsing, per the spec.
- `crates/pulse-app/src/backend/playback/logic.rs:141` — `format_quality` would print "DSF · 24-bit"; DSD paths need a branch that prints `DSD64` / `DSD128`.
- `crates/pulse-app/src/backend/playback/` (`devices.rs` + `controller.rs`) — the refusal gate: a DSD track is playable only when the resolved output mode is the integer engine and the device rate ceiling carries the DoP rate; otherwise a clear designed error, never audio. Pencil pass decides the unplayable-track treatment (dimming, badge, copy).

## Decisions (proposed — confirm or amend during build)

- The library stores the **true DSD sample rate** (2 822 400 / 5 644 800) on the track row; DoP rate is a decode-time mapping. Honest metadata, and the badge derives from it.
- Marker phase always starts at `0x05` after open and after every seek; both DoP start bytes are legal, a fixed choice keeps fixtures deterministic.
- DSD64→DSD64 gapless works unchanged (`SetNext` sees identical `PcmFormat`); a DSD↔PCM or DSD64↔DSD128 boundary takes the existing format-change rebuild path. No special casing.
- DSD128 ships in the same pass but stays untested-on-hardware until a device with a ≥352.8kHz integer ceiling confirms it; the gate already refuses where the ceiling is lower.
- Phases 1–2 deliberately leave separate DSF/DFF header parsers in `pulse-engine` and `pulse-app`; follow-up consolidation is required because the duplicated rate table, bits-per-sample rules, and `DSF_BLOCK_SIZE` versus the app's hardcoded `4_096` can drift until scanning accepts a track the decoder refuses, or the reverse.

## Phases

1. **Parsers + packer** (engine only): `decode_dsd.rs` with byte-exact fixtures (DSF bit-reversal, DFF interleave, marker alternation, seek alignment, DST refusal); factory routing. `make verify` green, no app change.
2. **Library**: walk + metadata + repo rate fields + quality badge; a scanned DSD album browses like any other.
3. **Gating + UI**: the refusal path with designed copy; Pencil pass first (standing rule: no frontend surface before a design exists).
4. **Hardware acceptance**: `~/qobuz/04 - 暧昧.dff` on the Matrix Mini-i Pro 4 through the integer engine — display flips to DSD64, pause holds the lock (v0.3.0 hold-on-pause), seek re-locks after a brief drop; refusal verified on a Universal-engine device (AirPods) with the designed error and zero audible noise.

## Verification

Spec's list verbatim: fixture byte-exactness, Matrix DSD64 lock across pause/seek, refusal paths silent, scan behavior (DST skipped with note, DSF tags land, tagless DFF named from filename). Plus: `script/dop_pack.py` output for the same DFF matches the Rust packer's stream byte-for-byte over the first N frames.
