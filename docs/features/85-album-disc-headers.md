# Album Page — Disc Headers

Feature 85 · P2 · GitHub issue [#85](https://github.com/yicheng47/pulse/issues/85). Requested by Jason 2026-09-03 from the Random Access Memories (10th Anniversary Edition) album page: "do we know that's two cd? so we can add a header to separate the tracks into cd1 and cd2?"

## Motivation

We do know. The scanner reads the disc tag (`backend/scan/metadata.rs`, `Accessor::disk`, for both tagged PCM and DSD files), the catalog stores it (`tracks.disc_number`), and the album track query already orders by `COALESCE(disc_number, 1), track_number` (`backend/repo/tracks.rs`). The album page then renders one flat row per track (`surfaces/library/albums.rs`, the loop over `tracks` in the album detail table), so disc 1 runs straight into disc 2 and the `#` column restarts at 1 with nothing to explain it. The boundary is known; it is just not drawn.

## Scope

- **Grouping is derived in the view.** Before rendering, the album's ordered tracks are split by `disc_number` (a missing tag counts as disc 1). When there are two or more distinct discs, a header row is inserted before each group's first track. One disc, or no disc tags at all, renders exactly as today.
- **The header row** reads `Disc N`, in the table's caption style (the mono, muted treatment of the column header from `render_album_track_header`), full width, with the table's row divider. It is not selectable, not playable, has no context menu, and takes no keyboard focus.
- **Track indices are untouched.** `activate_album_track(index, …)`, selection, the right-click menu, keyboard navigation, and queue order keep indexing the track list; headers are rendering only.
- **The `#` column keeps each track's own number**, so `1` appears again under `Disc 2` — the header is what explains it.
- **Only the album page.** The artist detail's album lists and the Tracks page keep their current tables.
- **Design deferred.** The header row is a new row style in the album track table, so it will need a Pencil pass on the album page frame before a mission — but not now (Jason, 2026-09-03: "Just record the issue for now, no need for the design."). This spec records the issue; scheduling and the design pass come later.

## Non-Goals

- Disc totals from tags (`disk_total`) — the header count comes from the tracks present.
- Scanner or schema changes; per-disc play or enqueue actions; disc subtitles such as "Disc 2 — Drumless Edition"; any change to the queue or now-playing.

## Implementation Phases

1. **Pencil pass** (when scheduled): the `Disc N` header row on the album page frame — height, spacing, caption style, divider; pinned as Design Source here once approved.
2. **View**: a pure grouping helper over `&[Track]` returning the disc boundaries (unit-tested: single disc → no headers; untagged → disc 1; headers at every boundary; discs out of order are impossible after the query, but the helper still groups by value), and the header element inserted in the album detail row loop. Indices passed to `render_album_track_row` are unchanged.

## Verification

- `make verify` green; grouping helper tests as above.
- Manual on Random Access Memories (10th Anniversary Edition): `Disc 1` above track 1, `Disc 2` above "Horizon Ouverture"; clicking, double-clicking, and right-clicking tracks under `Disc 2` act on the right track; keyboard navigation skips headers; a single-disc album shows no header.
