# Centralize the track-artist identity SQL in the tracks repo

> Bug · P3 · filed 2026-08-29 from the feature 25 phase 4 review. **Fixed** 2026-08-29, commit `84cf129` — `EFFECTIVE_TRACK_ARTIST_SQL` in `backend/repo/mod.rs`, pinned to one definition by the boundary gate.

## Description

The Tracks page's artist identity — `COALESCE(NULLIF(trim(artist), ''), 'Unknown Artist')` — is written three times inside `crates/pulse-app/src/backend/repo/tracks.rs` and never hoisted: the artist-facet aggregate (`tracks::artists`, ~line 291, with the fallback bound as `?1`), the `TrackSortOrder::Artist` order-by (~line 614), and the artist filter clause in `track_filter_clause` (~line 666). Feature 25 phase 2 centralized the *album*-artist identity as `EFFECTIVE_ALBUM_ARTIST_SQL` in `repo/mod.rs` and the phase 4 boundary test pins that fragment to one definition; the track-artist fragment has no such guard.

## Expected

One `EFFECTIVE_TRACK_ARTIST_SQL` fragment in `repo/mod.rs`, interpolated by all three sites, with the same "exactly one definition" assertion the album-artist fragment has.

## Why it matters

`tracks::artists` groups on the expression and the filter clause matches on it; if one copy drifts, the artist popover offers a chip whose filter returns nothing — silent and user-visible. A doc comment on `tracks::artists` names the invariant today; a shared constant would enforce it.

## Relevant code

- `crates/pulse-app/src/backend/repo/tracks.rs` — the three copies.
- `crates/pulse-app/src/backend/repo/mod.rs` — `EFFECTIVE_ALBUM_ARTIST_SQL`, the pattern to follow.
- `crates/pulse-app/src/backend/mod.rs` — the SQL boundary gate that asserts the album-artist fragment has one definition; extend it to the track-artist fragment.

## Verification

- `make verify` passes.
- `grep -rn "NULLIF(trim(artist)" crates/pulse-app/src` matches exactly one definition (the `EFFECTIVE_ALBUM_ARTIST_SQL` constant contains its own `NULLIF(trim(artist), '')` sub-expression; the gate should count the full track-artist fragment, not the substring).
- The Tracks-page artist facet and filter keep track-artist identity (existing regression test from feature 25 phase 2).
