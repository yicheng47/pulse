# DSD Playback over DoP

P2. Decided by Jason, 2026-08-31: with the integer engine delivering bits exactly, DSD support becomes cheap — a container parser and a bit-packer, not a DSD engine. Gated on feature 32's phase-4 DoP acceptance pass.

## Motivation

Feature 32's acceptance test already proves the pipeline can carry DSD-in-DoP intact; the only thing between that proof and playing a DSD library is decode-side support for the DSD containers. DoP-packing at decode time turns a `.dsf`/`.dff` into an ordinary 176.4kHz/24-bit PCM stream — 16 DSD bits per channel per sample, alternating `0x05`/`0xFA` markers in the top byte — and the DAC does the unwrapping. No DSP, no rate conversion, no engine changes. The PCM-only engine constraint holds: DSD never exists as DSD inside Pulse; it is PCM frames whose payload happens to be DoP.

## Scope

- **Decode**: `.dsf` and `.dff` parsers behind the existing `SourceDecoder` seam, uncompressed DSD only. DSF stores 4096-byte per-channel blocks with LSB-first bit order (per-byte bit reversal required); DFF is FRM8/PROP/DSD chunks, MSB-first. Output is DoP frames: DSD64 → 176.4kHz/24, DSD128 → 352.8kHz/24 where the device's rate ceiling allows. The DoP packer is shared with (graduates from) impl 0016's stage-5 payload script.
- **Library**: scan indexes `.dsf`/`.dff`; metadata from DSF's trailing ID3v2 tag, filename fallback for tagless DFF; the quality badge reads DSD64/DSD128.
- **Gating — a safety rule, not polish**: DoP survives only the bit-perfect path. A corrupted DoP stream plays as loud hiss, so a DSD track refuses to play — clear error, never noise — when the active device's engine is Universal, or its rate ceiling cannot carry the DoP rate. UI treatment of unplayable DSD tracks (dimming, badge, error copy) is a Pencil-pass decision.
- **Purity**: feature 32's rules apply unchanged — hardware volume or fixed at 100%; the integer engine structurally cannot multiply samples, which is exactly what DoP requires.
- **Underruns**: zero-filled frames carry no markers, so the DAC drops out of DSD and re-locks when data resumes — a brief mute/click, accepted under the engine's existing click policy.

## Non-Goals

- No DST-compressed DFF (refused at scan with a clear note), no SACD ISO, no DSD256+ until a probe proves the rates.
- No native/raw DSD output — DoP only.
- No DSD→PCM conversion fallback for non-bit-perfect devices; the refusal is the feature.
- No change to feature 32's engine; this is decode and library work only.

## Implementation Phases

1. **Parsers + packer**: DSF/DFF chunk parsing and the DoP packer in the decode layer, fixture-tested byte-exactly (including DSF bit-reversal fixtures), behind the `SourceDecoder` factory.
2. **Library**: scan extensions, metadata, quality badge.
3. **Gating + UI**: engine/rate gate with the clear-error path; Pencil pass for the unplayable-track treatment.
4. **Hardware validation**: `~/qobuz/04 - 暧昧.dff` through the integer engine on the Matrix — display flips to DSD64; pause holds the lock; seek re-locks; refusal verified on a Universal-engine device.

## Verification

- `make verify` green; parser and packer fixtures are byte-exact (known DSD bytes in, exact DoP frames out).
- Matrix: DSD64 on the display during playback; pause/resume keeps the DSD lock (feature 32's hold-on-pause); seek re-locks after a brief drop.
- Refusal paths: Universal engine and rate-limited devices show the clear error, never audible noise.
- Scan: DST-compressed DFF is skipped with the note; DSF metadata lands; tagless DFF gets the filename fallback.
