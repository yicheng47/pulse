# Round Album card images at the image layer in GPUI

> Bug · P3 · filed 2026-08-29 as GitHub issue #67 (closed when tracking moved to docs, 2026-08-29). **Fixed** 2026-08-29, commit `84cf129` — `render_cover` takes the radius and applies it to the `img`; the Album Detail hero owns its ring at `RADIUS_MD`.

## Problem

`crates/pulse-app/src/surfaces/library/albums.rs` puts `overflow_hidden()` and `rounded(RADIUS_SM)` on the Album cover container, but GPUI clips overflow to rectangular bounds rather than the rounded shape. The image itself remains square at the 4px corners; this is subtle but is the same rendering behavior exposed by the larger circular Artist avatars in feature #38.

## Scope

- Apply the card corner radius to the Album cover `img` itself while preserving `ObjectFit::Cover`.
- Keep the existing rounded container and border for the card ring.
- Verify with a non-square cover that it remains center-cropped and no square image corners escape the 4px radius.

## Verification

- Check Album cards with contrasting non-square artwork in the real GPUI window.
- `make verify` passes.

This is pre-existing Album-page behavior and was intentionally left out of the feature #38 working-tree delta.
