# Artists Page

> Feature 11 · P2 · GitHub issue [#38](https://github.com/yicheng47/pulse-src/issues/38). Reverses `mvp.md`'s recorded "No top-level Artists page in MVP" decision — that was scope control for v0, and post-v0.1.1 real usage says the artist filter/facet alone is not how a music collection gets browsed.

## Motivation

Artist is the primary mental axis for a music library, and Pulse hides it behind a filter popover on the Tracks page. Getting "all of one artist's albums" today takes knowing the picker exists, opening it, and reading Tracks instead of Albums. The MVP treated artist as metadata-plus-facet deliberately (`docs/product/mvp.md`, Non-Goals); the library has outgrown that. The data layer is already most of the way there: `store.artists()` returns names with track counts, album queries resolve an effective artist via `COALESCE(album_artist, artist)`, and search already spans artists — this feature is surface, not plumbing.

## Scope

- **Artists entry in the sidebar's LIBRARY group** (order: Albums, Artists, Tracks, Playlists), same nav-item styling and selection state as its siblings.
- **Artists index page**: an alphabetical card grid like the Albums page — five cards per row, each a large circular avatar with the artist name and `N albums · M tracks` centered beneath — with the type-to-filter search input pattern the Tracks artist picker already established. **Avatar rule** (Jason, 2026-08-29): the avatar is the artist's photo once metadata enrichment (a separate feature: MusicBrainz identity + fanart.tv/Wikimedia images, opt-in, cached) supplies one; until then, and for any artist it cannot resolve, the avatar is one of the artist's own album covers from the library's cover cache (the first album under that artist in the page's album ordering); an artist with no photo and no cached cover gets the default empty avatar — a muted disc with a user silhouette (lucide `user`), the same for every such artist; no initials, no seeded colors (Jason, 2026-08-29: guessed tiles read as bugs). No network fetches in this feature.
- **Artist identity = effective album artist**, the same `COALESCE(NULLIF(trim(album_artist), ''), artist)` the Albums page groups by — otherwise every "feat." collaboration becomes a phantom index row. Counts come from the same grouping. A new store query (`artist_page` or extended `artists()`) returns name + album count + track count under that identity.
- **Card click opens the artist detail page** (Jason, 2026-08-29 — replaces the earlier "Albums page with a chip" route): a back link to Artists, a hero with the 190px circular avatar (same photo → cover → empty rule), an ARTIST eyebrow, the name, `N albums · M tracks · total duration · added <year>`, and **Play all** / **Shuffle** actions over the whole artist, then an **Albums** section — the Albums grid cards, restricted to this artist, in the Albums page's ordering. Album cards open Album Detail as they do today, and Album Detail's back link returns to the artist. This needs artist filtering in the album page query (`album_page`'s filter set), mirroring what Tracks already supports. No artist chip on the Albums page.
- **Design first** in `design/pulse-desktop.pen`: the Artists index grid, the artist detail page, and the sidebar entry across affected screens.

## Non-Goals

- Bio, top tracks, similar artists, or any other section on the detail page beyond the hero and the album grid — those need metadata enrichment and play history, which are separate features.
- Splitting multi-artist strings. Genres are comma-split because Qobuz writes them as lists; artist strings must not be — "Crosby, Stills & Nash" is one artist. Tag hygiene stays a library-data problem (the known Qobuz `######` album-artist garbage will be visible on this page; the fix remains a NAS retag, not code).
- External artist photos or any metadata-service lookup for artwork — the avatar is derived only from local album covers, with the default empty avatar as the fallback.
- Changes to the Tracks artist picker — it stays, they serve different moments.

## Design Source

`design/pulse-desktop.pen`, read via the `pencil` MCP only. Applied 2026-08-29 (phase 1 done):

- **Sidebar entry**: `Nav / Artists` (`IPHd0`, icon `UNR5D` lucide `mic-vocal`, label `d2Bjp`) sits second in the `Sidebar / Library` component's LIBRARY group (`IqX8X` → `lJP1a` → `H8ywB`), so every library screen carries it. Order: Albums, Artists, Tracks, Playlists. Selection follows the sibling convention — the screen's sidebar ref overrides the item fill to `$accent-soft`, icon to `$accent`, label to `$text-primary`.
- **Artists index**: screen `Library / Artists` (`ixG6j`). Page column `Artists Content`: header (`Artists Header` — title `Artists`, meta `N artists · N albums · N tracks`), the type-to-filter input (`Artist Filter Input` `ZvMKS`, the Tracks artist-picker input at 280 wide, placeholder `Filter artists`), then `Artists Scroll Region` with a header row (ARTIST / ALBUMS / TRACKS, mono 10 muted) `Artists Grid`: rows of five 204-wide cards with a 32px gap; each card is a 180px circle (`Avatar / Photo`: image fill clipped to the circle with a 1px `#403D39` inner stroke; or `Avatar / Empty`: `#1E1E1C` disc with a 1px `#312F2C` stroke and a 72px muted lucide `user` silhouette) over a centered name (Inter 14 semibold) and meta line (`N albums · M tracks`, Inter 12 muted). Rows overflow the scroll region exactly as the Albums grid does. Rows are alphabetical; the fixture includes `王菲`, `宇多田ヒカル`, and the `######` retag-debt row on purpose. Scrollbar as on Tracks.
- **Artist detail**: screen `Library / Artist Detail` (ARTISTS row, right of the index). Built on the Album Detail anatomy: `Back` (arrow + `Artists`), `Artist Header` — `Avatar / Photo` 190px circle (or `Avatar / Empty`), `Copy` with eyebrow `ARTIST`, title in Rajdhani 38, `Meta Row` (`2 albums · 26 tracks` and `1 h 52 min · added 2024`), `Actions` = the Play button plus a `Button / Secondary` Shuffle — then `Albums Section`: the Albums page's `Section Header` (`Albums` · `N albums`) and `Album Grid` cards limited to the artist.

## Implementation Phases

1. **Design** (done 2026-08-29, see Design Source): Artists index grid, artist detail page, sidebar entry, in the `.pen`.
2. **Store**: the artist index query under effective-album-artist identity (name, album count, track count, alphabetical), and artist filtering in the album page query. Unit tests beside the existing store tests, including the `album_artist`-empty fallback and a "feat."-heavy fixture proving no phantom rows.
3. **App**: sidebar route, index grid with type-to-filter, the artist detail page (hero + Play all / Shuffle + the artist's album grid), Album Detail's back link returning to the artist. View-model tests for the index filter round-trip and the Artists → Artist Detail → Album Detail route state.

## Verification

- `make verify` green; store tests for identity grouping and album artist-filtering; view-model tests for the index filter and the artist route state.
- Manual (Jason): Artists appears in the sidebar and selects correctly; every row shows an avatar — a real cover where the artist has one, the default empty avatar otherwise; the index groups reissues under one artist (王菲 test case); counts match the Albums/Tracks pages; clicking a card opens the artist detail page showing only that artist's albums, Play all / Shuffle play them, and Album Detail's back link returns to the artist; type-to-filter narrows the index; the `######` garbage row appears (expected — retag debt, not a bug).
