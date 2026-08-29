# Centralize effective album artist SQL across catalog queries

> Bug · P3 · filed 2026-08-29 as GitHub issue #66 (closed when tracking moved to docs, 2026-08-29). **Fixed** in feature 25 phase 2, commit `aa211ec` — one shared fragment in `backend/library/repo/mod.rs`.

## Context

Feature #38 introduced `EFFECTIVE_ALBUM_ARTIST_SQL` so the Albums and Artists groupings use the exact same `COALESCE(NULLIF(trim(album_artist), ''), artist)` identity. Five hand-written copies remain in `crates/pulse-app/src/library/store/tracks.rs` (four, including `for_album`) and `crates/pulse-app/src/library/store/search.rs` (one), leaving future identity changes vulnerable to drift.

## Scope

- Reuse one shared effective-album-artist SQL definition in Albums, Artists, Tracks, and Search catalog queries.
- Preserve each query's existing parameter numbering and behavior.
- Keep multi-artist strings intact; do not split names.
- Add or adjust focused store tests proving the shared identity behavior.

## Verification

- `make verify` passes.
- No hand-written effective-album-artist SQL copies remain in catalog store queries.
