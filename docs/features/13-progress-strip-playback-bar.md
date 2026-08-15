# Progress Strip Playback Bar

> Feature 13 · P2. Qobuz-style bar anatomy: the seek bar becomes a full-width 3px strip on the playback bar's top edge instead of an inline row inside the transport cluster. Fixes the transport compaction introduced when the volume cluster (feature 01) widened Output Status.

## Motivation

The playback bar packs three fixed clusters plus an elastic transport into one row, and the seek bar lives *inside* the transport column — so every widening of the side clusters (volume, dividers) squeezes the most important interaction in the bar. Qobuz's answer is structural: the progress bar spans the entire bar width as a thin strip at the top edge, the transport keeps only its buttons, and times become a compact stacked block. More seek travel, less crowding, and the strip doubles as the bar's top boundary.

## Design source

`design/pulse-desktop.pen`, read via the `pencil` MCP only — already applied to the Playback Bar component (`qKkw7`):

- Root is now **vertical**: a **Progress Strip** (`M9c2cc`, full-width × 3px, `$bg-elevated` track, `$accent` elapsed fill) at the very top — it replaces the bar's former top border — above a **Content Row** (`VUe6q`, the old horizontal layout: padding [12,20], gap 22). Total bar height stays 92.
- **Transport Center** (`kRlBI`) is now horizontal and centered: a **Times** stack (`x9CAO` — elapsed `$text-secondary` over duration `$text-muted`, mono 11, right-aligned) then the transport controls, gap 18. The old inline Progress Row is deleted.
- Output Status (volume | dividers | format | output | queue, feature 01's cluster) is unchanged.

## Scope

- Restructure `playback_row.rs` to match: full-width 3px seek strip at the bar's top edge carrying **all existing seek semantics** (drag-to-seek, position updates) with a generous invisible hit target (~12px) so a 3px line is comfortably grabbable; stacked times block left of the transport buttons; inline progress row removed; the strip's track color stands in for the old top border.

## Non-Goals

- Hover scrubbing previews, a drag knob, or time-bubble tooltips — future polish, not designed.
- Any change to transport buttons, the volume cluster, or seek behavior itself.

## Implementation Phases

1. Layout restructure in `playback_row.rs` per the design nodes, preserving the existing seek interaction wiring and position-tick rendering.

## Verification

- `make verify` green; existing seek/position tests stay green.
- Manual (Jason): seeking by clicking/dragging anywhere along the full bar width; elapsed fill tracks playback across the whole strip; times stack updates live; bar boundary still reads cleanly against the content above; volume cluster and queue button unaffected.
