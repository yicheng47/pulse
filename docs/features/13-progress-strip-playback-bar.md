# Progress Strip Playback Bar

> Feature 13 · P2. Qobuz-style bar anatomy: the seek bar becomes a full-width 3px strip on the playback bar's top edge, while split elapsed/duration labels flank a larger, centered transport group. Fixes the transport compaction introduced when the volume cluster (feature 01) widened Output Status.

## Motivation

The playback bar packs three clusters into one row, and the seek bar previously lived *inside* the elastic transport column — so every widening of the side clusters (volume, dividers) squeezed the most important interaction in the bar. The approved structure gives progress the entire bar width, balances equal-width Now Playing and Output slots around the transport, and splits compact elapsed/duration labels across the centered controls. More seek travel, less crowding, and the strip doubles as the bar's top boundary.

## Design source

`design/pulse-desktop.pen`, read via the `pencil` MCP only — already applied to the Playback Bar component (`qKkw7`):

- Root is now **vertical**: a **Progress Strip** (`M9c2cc`, full-width × 3px, `$bg-elevated` track, `$accent` elapsed fill) at the very top — it replaces the bar's former top border — above a **Content Row** (`VUe6q`, the old horizontal layout: padding [12,20], gap 22). A 10px `$accent` **Hover Scrubber** (`UwAEe`) appears at the elapsed-fill endpoint only while hovering or dragging and is centered vertically on the 3px strip; the existing 12px interaction area remains its hover/drag target. Total bar height stays 92.
- **Now Playing** (`MVw5k`) is a fixed 317px slot: a 60px cover and track copy form a 300px content block, followed by a 16px gap and a 1×44px divider.
- **Transport Center** (`kRlBI`) fills the middle and stays centered: a 44px right-aligned elapsed slot, the transport controls, then a 44px left-aligned duration slot, with 28px between each timer and the controls. Both timers use `font-mono` 10 at normal weight; elapsed uses `$text-secondary`, duration uses `$text-muted`, and minute values are compact (`3:05`, not `03:05`). The controls use a 16px gap, 19px secondary buttons, and a 34px primary button with an 18px icon.
- **Output Slot** (`NyLu1`) is also fixed at 317px: a 1×44px divider, 16px gap, then the existing 300px Output Status. The equal side slots keep the primary transport button mathematically centered; the contents of Output Status (volume | dividers | format | output | queue) remain unchanged.

## Scope

- Restructure `playback_row.rs` to match: a full-width 3px seek strip at the bar's top edge carrying **all existing seek semantics** (click/drag-to-seek and position updates) with a generous invisible hit target (~12px) so a 3px line is comfortably grabbable; a 10px accent circle at the elapsed endpoint that appears on hover and remains visible throughout a drag; symmetric 317px side slots around a centered transport; split elapsed/duration labels; larger, more widely spaced controls; and no inline progress row. The strip's track color stands in for the old top border.

## Non-Goals

- Hover position previews and time-bubble tooltips — the hover circle marks the current elapsed endpoint only and does not preview or change the seek position.
- Any change to seek or transport behavior, or to the contents and interactions of the volume/output/queue cluster. Control sizing and spacing are visual changes only.

## Implementation Phases

1. Layout restructure in `playback_row.rs` per the design nodes, preserving the existing seek interaction wiring and position-tick rendering.

## Verification

- `make verify` green; existing seek/position tests stay green.
- Manual (Jason): the accent circle is hidden at rest, appears at the elapsed endpoint anywhere within the 12px hover target, and remains visible while dragging beyond it; seeking works by clicking/dragging anywhere along the full bar width, including during playback; elapsed fill tracks playback across the whole strip; split times update live; the primary transport button stays centered; the larger controls and symmetric dividers match Pencil; the bar boundary reads cleanly against the content above; volume, output, and queue interactions are unaffected.
