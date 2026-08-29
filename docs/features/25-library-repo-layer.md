# Backend Layer and Library Repo Layer

> Feature 25 · P2 · GitHub issue [#68](https://github.com/yicheng47/pulse-src/issues/68). Make the UI / domain split in `pulse-app` structural and enforced (a `backend/` module that can never import `gpui`), then regulate every SQL statement behind a Runner-style repo layer and lift the library's use cases into an ops layer. Behavior-preserving; no schema change; still one crate.

## Motivation

SQL in Pulse is confined to `library/store/`, which is good, but inside that boundary it is unregulated. Measured 2026-08-29 on `main` plus the feature 11 working tree: ~130 SQL literals across nine files (`tracks.rs` 25, `playlists.rs` 19, `schema.rs` 47 counting migrations), the effective-album-artist identity expression written out six times (feature 11 introduced the shared `EFFECTIVE_ALBUM_ARTIST_SQL` constant but only Albums and Artists use it; Tracks and Search still carry hand-written copies — issue #66), column lists repeated per query so a new column means hunting every `SELECT`, and `rusqlite` types reaching into `library/mod.rs`. `LibraryStore` is a 39-method facade that mixes row access (`playlist`, `storage_roots`), page-shaped aggregates (`album_page`, `artist_index`), and multi-step mutations (`delete_tracks` with preflight, `add_storage_root`).

Above the store there is no ops layer. The library surface orchestrates use cases itself — `surfaces/library/catalog.rs`, `lifecycle.rs`, `storage_actions.rs`, `playlist_actions.rs` (1,217 lines, 62 store call sites): load an artist detail, delete an album and reconcile the queue, run a scan and backfill covers, edit a playlist. That is business flow living in a GPUI view. Feature 19 (`pulse mcp`) needs exactly those use cases from a stdio process with no window, and today it cannot have them without copying the surface code.

Runner's backend is the template, already proven in this workspace: `repo/<table>.rs` owns row types, column lists, and plain SQL for one table; `ops/<area>.rs` composes repos into use cases; `model.rs` owns the domain types; nothing else talks to SQLite. Jason, 2026-08-29: "I want the repo layer for my pulse repo as well … I want to regulate all the SQLs."

The wider gap is that Pulse has no frontend/backend split at all — one crate, with `library/` and `surfaces/` as peers. The domain half is already `gpui`-free in practice (`library/`, `playback/`, `queue.rs`, `app_settings.rs`, `preferences.rs`, the `settings` view model — ~13.8k lines) and the UI half depends on `gpui` (`surfaces/`, `ui/`, `app_store.rs`, `main.rs`, `menu.rs`, `text_input.rs`, `theme.rs`, `assets.rs` — ~15.9k lines), with `updater.rs` straddling: its Sparkle delegate wakes the app through `WeakEntity`/`AsyncApp`. Nothing enforces the line; a store module could import `gpui` tomorrow. Jason, 2026-08-29: one crate is fine, "but we still need to separate the UI and backend logic which is the domain logic." Runner enforces it with a crate boundary (`runner-backend` has no GPUI dependency); Pulse gets the same boundary as a module first, and a crate only when something other than the app needs to link it (feature 19's `pulse mcp`).

## Scope

0. **`backend/` module boundary.** `crates/pulse-app/src/backend/` holds `library/`, `playback/`, `queue.rs`, `settings.rs` (today `app_settings.rs`), `preferences.rs`, and the updater's pure state and transitions; `app_store.rs` stays outside as the single bridge between backend and GPUI, and the updater's `WeakEntity`/`AsyncApp` wake-up moves into it (or a thin `updater_bridge` beside it) so `backend/updater.rs` is plain Rust. Surfaces import `backend::*` types and `app_store` only. Enforced, not conventional: a unit test in `backend/mod.rs` scans every file under `backend/` for `gpui` and fails on a hit, and `docs/arch/pulse-app.md` states the rule ("backend never imports gpui; `app_store` is the only bridge"). A crate split (`pulse-core`) is explicitly deferred to the feature that first needs to link the backend without GPUI.

Reference: `runner/crates/runner-backend/src/{repo,ops,model.rs,db.rs}` — `repo/project.rs` for the row-type + `COLUMNS` + `select_list` shape, `repo/mod.rs` for the shared helpers (`select_list`, `de_err`), `ops/` for use-case composition.

1. **`library/repo/`** — one module per table (`tracks`, `albums` is a view over tracks and stays a query module, `playlists`, `playlist_entries`, `storage_roots`, `scans`, `schema`), each owning its row type, a `COLUMNS` list, and every SQL statement that touches that table. Shared fragments (the identity expression, genre membership, `select_list`) live once in `repo/mod.rs` (or `repo/sql.rs`) and are interpolated, never retyped — #66 closes here. Every statement is reachable by name; no SQL string is built outside `repo/`. `rusqlite` is imported only under `library/repo/` and `library/schema` (migrations); `library/mod.rs` exposes `LibraryError` without leaking `rusqlite::Error` types into surfaces.
2. **`library/ops/`** — the use cases the surface and the future MCP server share, each a plain function over a connection (or the store handle) returning domain types: catalog paging (albums with filters + sort, tracks, artist index, artist detail = artist + albums + tracks), search, storage roots (add/rename/remove with the preflight checks), scan + cover backfill orchestration, playlist CRUD and reordering, delete album/tracks with the queue-reconciliation outcome. `surfaces/library/*_actions.rs`, `catalog.rs`, and the store-touching parts of `lifecycle.rs` shrink to "call the op, apply the result to view state"; the 62 direct store call sites in the surface go to zero — the surface depends on `library::ops` only.
3. **`library/model.rs`** — the 23 domain types (`Track`, `Album`, `Artist`, `Playlist`, `StorageRoot`, filters, sort orders, outcomes) move out of `library/mod.rs`; `library/mod.rs` becomes the module root and re-export point. `LibraryStore` keeps its name as the connection owner (open/open_in_memory/migrate) but loses its 39-method facade; callers go through `ops`.
4. **Regulation gates** (enforced by tests or greps in Verification, not by convention): SQL literals only under `library/repo/` and `library/schema.rs`; one definition of each shared fragment; every repo query has a unit test against the in-memory database; `ops` tests cover each use case's outcome type.
5. **`docs/arch/pulse-app.md`** gains a Library section describing the three layers and the rule "surfaces never see SQL or `rusqlite`".

## Non-Goals

- Schema changes, new migrations, or a bump of `SCHEMA_VERSION` — the on-disk database is untouched.
- An ORM, query builder, or `serde_rusqlite`-style row mapping if it changes semantics; Runner's `from_row` is fine to adopt where it is a pure mechanical win.
- Async or a separate thread for the store; the scan worker's threading stays as is.
- Any product or UI change; the library surfaces render exactly as before.
- Building `pulse mcp` (feature 19) — this feature only makes it possible.

## Design Source

None — no UI change.

## Implementation Phases

Each phase is its own mission and independently shippable; `make verify` green at each.

1. **Backend boundary**: `git mv` the domain modules under `backend/`, untangle the updater's gpui wake-up into the bridge, add the `gpui`-scan test and the arch-doc rule. Moves only; `make verify` green.
2. **Repo layer**: move SQL into `backend/library/repo/<table>.rs`, one definition per shared fragment (closes #66), `COLUMNS` lists, row tests. `LibraryStore` methods become thin delegations so callers are untouched.
3. **Model + ops layer**: `backend/library/model.rs`; `backend/library/ops/*` use cases; the library surface re-pointed to ops; direct store calls removed from `surfaces/`.
4. **Facade retirement + doc**: delete the delegating `LibraryStore` methods no caller uses, `rusqlite` confined, arch doc section.

## Verification

- `make verify` green after every phase; test count does not go down.
- Phase 1: the `backend/` gpui-scan test exists and passes; `grep -rln gpui crates/pulse-app/src/backend` is empty; `grep -rn "use gpui" crates/pulse-app/src/updater*.rs` shows the wake-up only in the bridge; every `surfaces/` import of domain types resolves through `backend::` or `app_store`.
- Phase 2: `grep -rn --include='*.rs' -E '"(SELECT|INSERT|UPDATE|DELETE|WITH)\b' crates/pulse-app/src | grep -v library/repo | grep -v library/schema` is empty; `grep -rn "NULLIF(trim(album_artist)" crates/pulse-app/src` matches exactly one definition; every `repo` query has a test against `open_in_memory`.
- Phase 3: `grep -rn "rusqlite" crates/pulse-app/src | grep -v library/repo | grep -v library/schema` is empty; `surfaces/library/*` contains no `LibraryStore` method calls other than `open`; each `ops` use case has a test on its outcome (paging, artist detail, delete-album outcome with missing files, playlist reorder).
- Phase 4: `LibraryStore`'s public methods are `open`, `open_with_progress`, `open_in_memory` and connection access only; `docs/arch/pulse-app.md` describes the layers.
- Manual after every phase: scan a root, browse Albums/Artists/Tracks/Playlists, search, delete an album, edit a playlist — identical to before.
