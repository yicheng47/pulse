# DSD Playback over DoP

Feature 71 · P2 · GitHub issue [#71](https://github.com/yicheng47/pulse/issues/71). Decided by Jason, 2026-08-31: with the integer engine delivering bits exactly, DSD support becomes cheap — a container parser and a bit-packer, not a DSD engine. Gated on feature 32's phase-4 DoP acceptance pass.

## Motivation

Feature 32's acceptance test already proves the pipeline can carry DSD-in-DoP intact; the only thing between that proof and playing a DSD library is decode-side support for the DSD containers. DoP-packing at decode time turns a `.dsf`/`.dff` into an ordinary 176.4kHz/24-bit PCM stream — 16 DSD bits per channel per sample, alternating `0x05`/`0xFA` markers in the top byte — and the DAC does the unwrapping. No DSP, no rate conversion, no engine changes. The PCM-only engine constraint holds: DSD never exists as DSD inside Pulse; it is PCM frames whose payload happens to be DoP.

## Scope

- **Decode**: `.dsf` and `.dff` parsers behind the existing `SourceDecoder` seam, uncompressed DSD only. DSF stores 4096-byte per-channel blocks with LSB-first bit order (per-byte bit reversal required); DFF is FRM8/PROP/DSD chunks, MSB-first. Output is DoP frames: DSD64 → 176.4kHz/24, DSD128 → 352.8kHz/24 where the device's rate ceiling allows. The DoP packer is shared with (graduates from) the [stage 5 payload script](../impls/archive/bit-perfect/stage-5-acceptance.md).
- **Library**: scan indexes `.dsf`/`.dff`; metadata from DSF's trailing ID3v2 tag, filename fallback for tagless DFF; the quality badge reads DSD64/DSD128.
- **Gating — a safety rule, not polish**: DoP survives only the bit-perfect path. A corrupted DoP stream plays as loud hiss, so a DSD track refuses to play — clear error, never noise — when the active device's engine is Universal, or its rate ceiling cannot carry the DoP rate. UI treatment decided by the 2026-09-01 Pencil pass (design source: `Spec — DSD Gate & Toasts` board `GGd35` and screen `Library / Tracks — Unplayable DSD` `V5k1T` in `design/pulse-desktop.pen`), riding on [feature 74](archive/74-toast-notifications.md)'s toast primitive:
  - **Refusal is a toast**, not the banner. Wrong output mode gets `Toast / Error + Action` — "DSD needs Bit-perfect output" with a **Switch to Bit-perfect** button that switches the mode and retries the same track. The other three gate errors are plain error toasts: "This device can't carry DSD128" (rate ceiling, names the device and its ceiling; DSD64 still plays), "Output device not verified yet" (capabilities unknown), "Couldn't read this DSD file" (metadata unreadable). These strings replace the four placeholders in `playback/logic.rs`.
  - **Library treatment**: rows of DSD tracks unplayable on the active output dim to 55% opacity — except the FORMAT value, which stays full-strength and flips to `$warning` (it is the explanation, not decoration). Re-evaluated live when the output mode or device changes. Rows stay clickable; clicking play fires the matching toast.
  - **Queue advance skips, not stops**: when auto-advance reaches an unplayable DSD track it skips forward with a warning toast, mirroring the missing-file skip — replacing the current behavior where `sync_next_source` clears the next track and playback stops dead.
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
3. **Gating + UI**: engine/rate gate with the clear-error path; Pencil pass for the unplayable-track treatment (done 2026-09-01 — decisions in Scope above; implementation depends on [feature 74](archive/74-toast-notifications.md) phase 1).
4. **Hardware validation**: `~/qobuz/04 - 暧昧.dff` through the integer engine on the Matrix — display flips to DSD64; pause holds the lock; seek re-locks; refusal verified on a Universal-engine device.

## Verification

- `make verify` green; parser and packer fixtures are byte-exact (known DSD bytes in, exact DoP frames out).
- Matrix: DSD64 on the display during playback; pause/resume keeps the DSD lock (feature 32's hold-on-pause); seek re-locks after a brief drop.
- Refusal paths: Universal engine and rate-limited devices show the clear error, never audible noise.
- Scan: DST-compressed DFF is skipped with the note; DSF metadata lands; tagless DFF gets the filename fallback.
