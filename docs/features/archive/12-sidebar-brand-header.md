# Sidebar Brand Header

> Feature 12 · P3. Post-v0.1.1 polish from real usage, benchmarked against Runner's sidebar: the header is cramped under the traffic lights and the brand mark reads as a raised tile instead of part of the surface.

## Motivation

The sidebar brand renders the app icon treatment — a 32px gradient tile with border and corner radius — which reads as a foreign object floating one layer above the sidebar, and the whole header sits tight against the traffic-light zone. Runner's header shows the target: a flat brand glyph in the surface, a larger wordmark, and generous headroom below the window controls.

## Design source

`design/pulse-desktop.pen`, read via the `pencil` MCP only — already applied across all 22 library sidebars and 3 settings sidebars (reference: `Library / Main`'s sidebar `vNEQj`, brand `OWFgQ`):

- Sidebar top padding **24 → 56** (library and settings sidebars alike), clearing the traffic-light zone.
- **Brand mark flattened**: container keeps its 32px footprint but loses gradient fill, border stroke, corner radius, and clip; the accent waveform path grows 22 → 24, centered. No tile — the glyph sits directly on `bg-surface`.
- **Wordmark** `font-display` 18 → **22**, weight 700 unchanged.
- The About/Update pages' icon-tile copies are deliberately untouched — there the app icon *is* the content.

## Scope

- Match the above in `shell.rs`'s sidebar header (and the settings sidebar's top padding for parity): padding, flat glyph rendering (drop the tile background/border), wordmark size.

## Non-Goals

- Any change to the actual app icon, dock icon, or the About/Update page identity blocks.
- Sidebar collapse behavior (feature 09) or any structural sidebar change.

## Implementation Phases

1. Single pass in `shell.rs` (+ settings sidebar padding), matched to the `.pen` values.

## Verification

- `make verify` green (no new logic — this is layout/styling; existing view tests must stay green).
- Manual (Jason): header spacing matches the design with the window controls clear of the brand row; the glyph renders flat with no tile edge; wordmark size matches; settings pages share the same top rhythm.
