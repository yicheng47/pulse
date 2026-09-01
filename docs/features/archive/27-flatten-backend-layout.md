# Flatten the Backend Layout

> Feature 27 · P2 · chore. Remove the `backend/library/` wrapper so `pulse-app`'s backend reads like Runner's: `backend/{model.rs, repo/, ops/, scan/}` beside `playback/`, `queue.rs`, `settings.rs`, `preferences.rs`, `updater.rs`. Moves only; no behavior, schema, or SQL change. **Merged** 2026-08-29, commit `9b8205a`. (The `library::` grep in Verification reads as intended only for `backend`: the two remaining hits are `surfaces/mod.rs` and `surfaces/search.rs` importing the `surfaces/library` view module.)

## Motivation

Feature 25 moved the old top-level `library/` module under `backend/` wholesale and then built `model.rs`, `repo/`, and `ops/` inside it. The `library` directory is now a wrapper: nothing outside `backend/` names it, and `ops/` reads as library-only when feature 55's MCP server will want playback and queue use cases beside the catalog ones. Jason, 2026-08-29: "maybe we could just remove that layer? just backend/ops, and backend/repos". Runner's `runner-backend/src/{model.rs, repo/, ops/}` is the shape.

## Scope

```
backend/
  mod.rs          module root; gpui gate + SQL/rusqlite gate (paths updated)
  model.rs        catalog domain types (was library/model.rs)
  repo/           one module per table, LibraryStore + LibraryTransaction (was library/repo/)
  ops/            catalog, delete, playlists, scan, storage (was library/ops/)
  scan/           metadata.rs, path.rs, walk.rs — the scan-pipeline helpers only ops::scan and tests use (were library/*.rs)
  playback/  queue.rs  settings.rs  preferences.rs  updater.rs   unchanged
```

- `git mv` every file; paths inside `use` statements and the boundary tests' `repo_root` follow. `system_time_ms` / `system_time_ns` (today in `library/mod.rs`) move to where their callers are (`repo/mod.rs` or `scan/`), whichever has all of them.
- `backend/mod.rs` re-exports the model types and `LibraryStore` exactly as `backend/library/mod.rs` does today, so `surfaces/` keeps importing `crate::backend::{Track, Album, …}` and `backend::ops::…` with only the `library::` segment gone.
- Test helper `metadata::write_test_wav` keeps one path (`backend::scan::metadata::write_test_wav`) for the playback and library-surface tests.
- `docs/arch/pulse-app.md`: §2 module map and §4 Library Layers renamed to the new paths; §10 boundary bullet updated. Crews may edit this file only.

## Non-Goals

- Renaming `LibraryStore`, `LibraryError`, or `LibraryTransaction` — the types keep their names.
- Moving playback / queue / settings use cases into `ops/` — that starts with feature 55.
- Splitting `backend/` into a crate.
- Any change to `surfaces/library/` — it is one surface with sub-pages and stays a directory, like `surfaces/settings/`.

## Design Source

None — no UI change.

## Verification

- `make verify` green; test count unchanged.
- `find crates/pulse-app/src/backend -maxdepth 1` lists `mod.rs model.rs repo ops scan playback queue.rs settings.rs preferences.rs updater.rs` and nothing else; `crates/pulse-app/src/backend/library` does not exist.
- `grep -rn "library::" crates/pulse-app/src --include='*.rs' | grep -v surfaces/library` is empty.
- Both boundary gates pass with `repo_root = backend/repo`.
- Manual: launch, scan a root, browse Albums / Artists / Tracks / Playlists, search, delete an album, edit a playlist — identical to before.
