# Artists Page

> Feature 11 · P2. Reverses `mvp.md`'s recorded "No top-level Artists page in MVP" decision — that was scope control for v0, and post-v0.1.1 real usage says the artist filter/facet alone is not how a music collection gets browsed.

## Motivation

Artist is the primary mental axis for a music library, and Pulse hides it behind a filter popover on the Tracks page. Getting "all of one artist's albums" today takes knowing the picker exists, opening it, and reading Tracks instead of Albums. The MVP treated artist as metadata-plus-facet deliberately (`docs/product/mvp.md`, Non-Goals); the library has outgrown that. The data layer is already most of the way there: `store.artists()` returns names with track counts, album queries resolve an effective artist via `COALESCE(album_artist, artist)`, and search already spans artists — this feature is surface, not plumbing.

## Scope

- **Artists entry in the sidebar's LIBRARY group** (order: Albums, Artists, Tracks, Playlists), same nav-item styling and selection state as its siblings.
- **Artists index page**: alphabetical list rows — artist name, `N albums · M tracks` — with the type-to-filter search input pattern the Tracks artist picker already established. Plain rows, no artwork: artists have no images without external metadata enrichment (post-MVP), and placeholder tiles are decoration.
- **Artist identity = effective album artist**, the same `COALESCE(NULLIF(trim(album_artist), ''), artist)` the Albums page groups by — otherwise every "feat." collaboration becomes a phantom index row. Counts come from the same grouping. A new store query (`artist_page` or extended `artists()`) returns name + album count + track count under that identity.
- **Row click routes to the Albums page with an artist filter applied**, shown as a removable chip alongside the existing genre chips. No bespoke artist-detail surface in this feature — the Albums grid filtered to the artist *is* the detail view. This requires adding artist filtering to the album queries (`album_page`'s filter set), mirroring what Tracks already supports.
- **Design first** in `design/pulse-desktop.pen`: the Artists index screen and the Albums-page artist chip state, plus the sidebar entry across affected screens.

## Non-Goals

- A rich artist-detail page (portrait, bio, top tracks) — worth doing only after metadata enrichment (MusicBrainz et al.) exists, which is post-MVP per the roadmap parking lot.
- Splitting multi-artist strings. Genres are comma-split because Qobuz writes them as lists; artist strings must not be — "Crosby, Stills & Nash" is one artist. Tag hygiene stays a library-data problem (the known Qobuz `######` album-artist garbage will be visible on this page; the fix remains a NAS retag, not code).
- Artist artwork, initials tiles, or any placeholder art.
- Changes to the Tracks artist picker — it stays, they serve different moments.

## Implementation Phases

1. **Design**: Artists index screen, Albums artist-chip state, sidebar entry, in the `.pen`.
2. **Store**: the artist index query under effective-album-artist identity (name, album count, track count, alphabetical), and artist filtering in the album page query. Unit tests beside the existing store tests, including the `album_artist`-empty fallback and a "feat."-heavy fixture proving no phantom rows.
3. **App**: sidebar route, index page with type-to-filter, click-through to filtered Albums with a removable chip. View-model tests for filter round-trip and route state.

## Verification

- `make verify` green; store tests for identity grouping and album artist-filtering; view-model tests for the index filter and the Albums chip.
- Manual (Jason): Artists appears in the sidebar and selects correctly; the index groups reissues under one artist (王菲 test case); counts match the Albums/Tracks pages; clicking an artist lands on Albums filtered with a removable chip; type-to-filter narrows the index; the `######` garbage row appears (expected — retag debt, not a bug).
