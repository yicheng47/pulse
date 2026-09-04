# Album Page — Artist Link

Feature 84 · P2 · GitHub issue [#84](https://github.com/yicheng47/pulse/issues/84). Requested by Jason 2026-09-03 from the Random Access Memories album page: "Artist link to artist page."

## Motivation

The album header prints the artist name as static text beside the `year · tracks · minutes` line (`crates/pulse-app/src/surfaces/library/albums.rs`, the `album.artist` label in the header row). The Artists destination (feature 11) already has a detail page per artist with that artist's albums and tracks, but from an album the only way there is sidebar → Artists → find the name. The artist name on an album is the expected way into the artist everywhere else; here it is a dead label.

## Scope

- **The header artist name is a link.** Pointer cursor; `text_primary` at rest as today; on hover the existing text takes the accent colour (same hover language as the album-page back link). No new element, no icon, no layout change.
- **Click opens the artist's detail page.** Resolve the album's `artist` string to its catalog `Artist` row (rows are keyed by `name_key`; add a by-name lookup in `ops::catalog` if none exists), then go through the existing path: `Destination::Artists` with `ArtistRoute::Detail { artist }` via `LibraryView::open_artist` (`surfaces/library/catalog.rs`), which loads `ArtistDetail` and persists the route exactly as the Artists index does (session restore, feature 21).
- **Back behaves as it does today.** From the artist page, back returns to the Artists index (`ArtistRoute::back`); the album is one click away in the artist's album list. No new back-stack.
- **No artist row, no link.** An album artist that has no `Artist` row (a compilation's "Various Artists" that never became an artist, or a stale catalog) renders as plain text — never a click that errors.
- **Same treatment on the Artists → album view.** When the album page is reached from an artist (`ArtistRoute::Album`), the name is still a link; it opens the same detail, which is where back would go anyway.

## Non-Goals

- Artist metadata, images, or enrichment (feature 72 stays postponed).
- Per-track artist links in the track table for tracks whose artist differs from the album artist — a later item once track artists are modelled.
- Any header redesign; the link is a hover state on existing text, so no Pencil pass is proposed (token-level styling, the deviation rule from feature 74's transport-feedback fix). Jason can ask for a pass if he wants an underline or an arrow.

## Implementation Phases

1. **Catalog lookup**: `ops::catalog` gains an artist-by-name (`name_key`) lookup returning `Option<Artist>` with the counts `artist_detail` needs; unit test on the catalog fixture.
2. **Header link + navigation**: the artist label becomes a stateful element with hover + click calling `open_artist`; the click is a no-op when the lookup returns `None`. Route/session tests in `surfaces/library/session.rs` prove album → artist sets `Destination::Artists` + `ArtistRoute::Detail` and persists.

## Verification

- `make verify` green.
- Session test: from `Destination::Albums` on an album, opening its artist yields `SessionRoute::Artists { artist: Some(name), album: None }` and restores after relaunch.
- Catalog test: lookup by name finds the artist regardless of case/whitespace folding (`name_key`), returns `None` for an unknown name.
- Manual: on Random Access Memories, hover shows the accent, click lands on Daft Punk's page; back returns to the Artists index; an album whose artist has no row shows plain text with no pointer cursor.

## Log

- **2026-09-04** — Shipped in v0.3.3. Spec archived, #84 closed.
- **2026-09-04** — Merged as `704178c` (PR #92, codex-crew mission, lead review). `artist_by_name` in `ops::catalog` resolves through the writer's own `EFFECTIVE_ALBUM_ARTIST_SQL` key (whitespace folds, case does not); the album header label becomes a hover link that opens the existing artist detail and switches the destination; the artist is resolved once when the album opens and a missing row renders plain text; session restore carries it. Two GPUI tests cover the route + relaunch and the artist → album → artist round trip. In-app look is Jason's; ships in the next release.
