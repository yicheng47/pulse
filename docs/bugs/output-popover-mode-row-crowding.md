# Output popover mode row: Reset to Auto crowds into the segmented control

> Bug · P2 · filed 2026-09-01 from Jason's screenshot (mini-i Series card, manual Bit-perfect override). Local note only — no GitHub issue.

## Description

In the output device popover, the active device card's Mode row lays out as `Mode` label → `Reset to Auto` link → `flex_1` spacer → Shared·Exclusive·Bit-perfect segments. When a manual override is set, `Reset to Auto` (the widest of the three middle states — the AUTO and NO INTEGER PATH badges are shorter) plus the three segment labels exceed the card's width, the spacer collapses to zero, and the link's text runs flush into the segmented control's left border — visually clipped/overlapping, no breathing room. Larger interface scales (feature 30, up to 150%) make the intrinsic widths grow while the popover width stays fixed, so the crowding worsens with scale.

## Expected Behavior

The Mode row always keeps a visible minimum gap between the reset affordance and the segmented control at every interface scale, without truncating the segment labels. If the row cannot fit at the current scale, the layout adapts (e.g. the segments drop to their own line under the label/link) rather than colliding.

## Steps To Reproduce

1. Open the output popover with a device active.
2. Set a manual mode override (click any segment) so `Reset to Auto` replaces the AUTO badge.
3. Look at the gap between `Reset to Auto` and the Shared segment; raise Settings ▸ General ▸ Interface scale to make it worse.

## Relevant Code

- `crates/pulse-app/src/ui/surfaces.rs:126` — `output_mode_control`: the row with the collapsing `flex_1` spacer and no minimum gap.
- `crates/pulse-app/src/ui/surfaces.rs:114` — `output_mode_reset_link`: fixed `ml(8)`, no shrink/truncation strategy (by design — the label should not truncate).
- `crates/pulse-app/src/surfaces/playback_popovers.rs:577` — the popover call site (the constrained one; the Devices page call site at `surfaces/devices.rs:306` has more width).
- Design source: `Output Device Popover` frame `vH78z` in `design/pulse-desktop.pen` (360px) — the mock shows the AUTO-badge state; the wider Reset-to-Auto state wasn't drawn, which is how the collision slipped through. Not a regression: the row was introduced by `668e504` (2026-08-31) and never fixed since — nothing to bisect.
- Size mismatch vs the mock: `Output Mode Row` (`m14ZyM`) draws the label at 12px, segment labels at 10px with [2,8] padding; the code renders label `BODY` (13) and link/segments `SMALL` with larger padding, so the real row is intrinsically wider than the 360px design accounts for. The fix should reconcile this (adopt the mock's compact sizes, or draw the Reset-to-Auto state at the code's sizes and widen/wrap accordingly).

## Environment

- OS: macOS (Darwin 25.5)
- Device / DAC: Matrix Audio mini-i Series (mini-i Pro 4)
- Input file format: n/a (UI layout)
- Pulse version: main post-`f50e040` (v0.3.0 + DSD), 2026-09-01

## Decided Fix (2026-09-02, drawn in the Pencil pass)

The row cannot fit `Reset to Auto` plus three segments in the popover's ~332px content width at any reasonable type size, so the popover's mode row becomes a **two-line stack**: line 1 is the `Mode` label plus the state slot (AUTO tag / NO INTEGER PATH tag / `Reset to Auto` link, gap 8), line 2 is the segmented control; 8px between lines, everything left-aligned. The Devices page call site keeps today's one-line layout — it has the width — via a `stacked` flag on `ui::output_mode_control`. Design updated in `Output Mode Row` (`m14ZyM`) inside the popover frame `vH78z`, showing the Reset-to-Auto state; **pending Jason's ⌘S** to persist the `.pen`, and his visual pass at the next smoke test since this layout call was made in drive mode.

## Verification

Reproduced in Jason's screenshot 2026-09-01 (override = Bit-perfect, `Reset to Auto` flush against the Shared segment's border). Code inspected. Fix decided and drawn; implementation pending.
