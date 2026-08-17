# Genre Filter Picker

> Feature 10 · P2. Raw embedded genre tags remain useful filter data, but arbitrary provider metadata should not control the width and composition of the Albums and Tracks filter bars.

## Motivation

Pulse currently promotes every distinct comma-separated genre member into a top-level chip. Real libraries contain provider-specific languages, spellings, hierarchies, and high-cardinality values, so the row becomes unpredictable and visually noisy. Preserve the source metadata without treating it as a trusted product taxonomy.

## Scope

- Keep All, Hi-Res, and Recently Added as fixed top-level filters on Albums and Tracks.
- Replace dynamic genre chips with one Genre trigger on both surfaces.
- Open a searchable popover using the existing artist-picker interaction and visual language.
- List the raw normalized genre members alphabetically with album counts; highlight the selected genre and expose Clear filter only while a genre is active.
- Selecting a genre continues to use the existing exact case-insensitive member query. Genre and the fixed filters remain mutually exclusive, matching the current `FilterChip` model.

## Non-Goals

- A canonical genre taxonomy, alias mapping, provider enrichment, automatic translation, metadata editing, nested genres, or user-pinned genres.
- Combining Genre with Hi-Res or Recently Added in one query.
- Changing the embedded genre value stored for a track.

## Design

The authoritative open state is `Library / Albums — Genre Filter` (node `I1OSIN`) in `design/pulse-desktop.pen`, derived from the existing artist-filter popover language.

## Implementation Phases

1. Add the Albums Genre picker state to `design/pulse-desktop.pen`, derived from the existing artist picker.
2. Add album-count summaries for normalized genre members and unit-test comma splitting, case-insensitive deduplication, and per-album counting.
3. Wire the shared picker state and searchable rows into Albums and Tracks, removing dynamic top-level genre chips.

## Verification

- `make verify` is green with store aggregation, search filtering, and picker-state tests.
- Manual: open and dismiss the picker from Albums and Tracks, search mixed-language genres, select and clear a genre, switch to every fixed filter, and confirm long lists scroll without changing the filter-row layout.
- Compare the open Albums picker against the Pencil state.
