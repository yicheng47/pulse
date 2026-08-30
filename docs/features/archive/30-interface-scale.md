# Interface Scale

> Feature 30 · P2. A Settings ▸ General control that scales the whole window (80–150%), replacing the redundant "Default output device" row now that Settings ▸ Output exists. Requested by Jason 2026-08-30; design approved the same day and pulled into M2. **Merged** 2026-08-30 in two phases — `05b9d08` (`px` → `rems` through `theme::rpx`, `theme::text` type scale, `check-units` gate; feature 29) and `56a120e` (`interface_scale` setting snapped to 80–150%, the General row stepper, View menu with `Cmd-=`/`Cmd--`/`Cmd-0`, Default output row and the settings output picker removed); smoke-tested by Jason. Depends on moving the app from `px` to `rems` units, which also delivers feature 29.

## Motivation

The General page has one row, "Default output device", which duplicates Settings ▸ Output since feature 28. Jason: "Could you put the UI zoom percentage there? The default output is redundant." A 1440-px design on a 13" display or an external 4K panel wants different densities, and macOS's display scaling is a blunt, system-wide tool. gpui supports this natively: every length written in `rems` scales with `Window::set_rem_size`, which is how Zed implements its UI font size. Pulse writes every length in `px` today (1,158 call sites, zero `rems`), so the control is cheap but its prerequisite is not.

## Design Source

`design/pulse-desktop.pen`, `Settings / General` `C5mRG`: the Playback group is now **INTERFACE** (`Not8k`) with one row `VARZq` in the card `SP83o` — title "Interface scale", description "Scales every part of the window. 100% is the default.", and a stepper `FyX8A` on the right: `Button / Icon` minus (`YwF3J`) · value pill `FiKXE` (56 × 34, `bg-inset`, radius 6, mono 13 semibold "100%") · `Button / Icon` plus (`wI2k3`). The "Default output device" and "Exclusive mode" rows are removed from the page (the second never existed in the app; per-device exclusive mode lives in the output popover).

## Scope

1. **Units.** Replace `px(N)` with `rems(N / 16)` across `pulse-app` for every layout length, text size, radius, and gap that should scale — a mechanical pass with the rem base at 16 px so nothing moves at 100%. Lengths that must stay physical stay in `px`: the native traffic-light position, window minimum size, hairline borders (1 px), the album cover image decode sizes. `theme.rs` gains the type scale from feature 29 expressed in rems, so 29 closes with this feature.
2. **Setting.** `interface_scale: f32` in `settings.json` (default 1.0; clamped to 0.8–1.5 on load; missing key → 1.0, no migration). Applied with `window.set_rem_size(px(16. * scale))` at window open and whenever the value changes; the window re-lays out immediately.
3. **Control.** The General page row per the design: minus / value / plus stepping through 80 · 90 · 100 · 110 · 125 · 150%; buttons disabled at the ends; the value pill shows the current step. `Cmd-=` / `Cmd--` / `Cmd-0` (reset) in a View menu do the same.
4. **Remove the Default output row** and the empty PLAYBACK group from General.

## Non-Goals

- Per-page or per-element zoom; text-only scaling.
- Scaling artwork resolution or the album grid's column rule beyond what rems give for free (the grid measures the viewport in px and card widths in rems, so columns naturally drop as scale rises).
- Any change to Settings ▸ Output.

## Implementation Phases

1. **Rem migration + type scale** (behavior-preserving at 100%; feature 29 folds in): units pass, `theme.rs` scale, a test that `set_rem_size(16)` renders identical layout to today for the album grid math, and the grep gate for both `text_size(px(` and bare `px(` outside the allow-list.
2. **Setting + control + shortcuts**: persistence, `set_rem_size` plumbing, the General row, the View menu items, the Default output row removal.

## Verification

- `make verify` green after each phase; test count does not go down.
- Phase 1: screenshots of Albums, an album detail, Tracks, Settings pages, and the playback row at 100% match today's build pixel-for-pixel apart from the type-scale drift collapses listed in the handoff.
- Phase 2: step 80 → 150% and back; every surface scales including popovers, menus, tooltips, the header, and the player; the traffic lights stay put; the album grid drops columns as the scale rises; quit and relaunch keeps the scale; `Cmd-0` returns to 100%.
