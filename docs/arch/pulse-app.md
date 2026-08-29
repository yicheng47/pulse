# Pulse App Architecture

> Companion to [`pulse-engine.md`](pulse-engine.md). The engine document owns the audio runtime boundary; this document describes the GPUI application's state, rendering, settings, and library boundaries.

## 1. Overview

`pulse-app` is the macOS product shell around `pulse-engine`. It owns the GPUI window, user interaction, local library, persisted app settings, update UI, device-management presentation, and the adapter that translates UI actions into playback-controller commands.

The app is organized by responsibility rather than by widget type. Rendering belongs in `surfaces/`, reusable visual primitives belong in `ui/`, observable cross-surface state belongs in `app_store.rs`, persistence types belong in `app_settings.rs`, playback adaptation belongs in `playback/`, and the SQLite-backed music catalog belongs in `library/`.

`pulse-engine` remains UI-agnostic. No GPUI entity, window, view, or surface type crosses into the engine crate.

## 2. Module Map

```text
crates/pulse-app/src/
  main.rs                    application startup and the first window
  app_store.rs               global observable app state and revision routing
  app_settings.rs            version-tolerant JSON settings model and persistence
  preferences.rs             data paths and one-time flat-file migration
  queue.rs                   app-side queue order, shuffle, repeat, and history
  settings.rs                settings page view models and section selection
  updater.rs                 update checks and updater state
  text_input.rs              shared editable text and selection state
  menu.rs                    macOS menu actions and installation
  theme.rs                   visual tokens
  assets.rs                  bundled asset loading
  playback/
    mod.rs                   playback adapter state and public app-side types
    controller.rs            controller creation, event drain, and event handling
    devices.rs               live and managed devices, capabilities, and defaults
    queue_control.rs         queue actions and track transitions
    logic.rs                 GPUI-free formatting and numeric helpers
  library/
    mod.rs                   scan API and catalog domain types
    store/                   SQLite schema and catalog queries
    metadata.rs              local audio metadata extraction
    path.rs                  path normalization
    walk.rs                  filesystem discovery
    tests.rs                 scan and deletion integration tests
  surfaces/
    shell.rs                 window chrome, titlebar drag, and body routing
    sidebar.rs               navigation groups, footer, and update hint
    sidebar_logic.rs         sidebar metrics and labels
    search.rs                search popover, keyboard flow, and IME input handler
    playback_row.rs          persistent transport row
    playback_popovers.rs     queue, volume, and device popovers
    devices.rs               Devices page
    devices_logic.rs         device timestamps and display helpers
    settings/                General, Update, and About pages
    library/                 Albums, Tracks, Playlists, Storage, filters, and menus
  ui/                        shared visual primitives and builder APIs
```

Each product area keeps its rendering and interaction code in one surface module or directory. Pure formatting, sorting, grouping, view-model, and metric functions live in sibling `*_logic.rs` files so they can be unit-tested without constructing a GPUI window.

`surfaces/library/logic.rs` contains the library view model. The other files under `surfaces/library/` divide page rendering, lifecycle, catalog loading, playback actions, playlist actions, storage actions, and their pure helpers without moving catalog persistence into the UI layer.

## 3. Domain Model

Domain objects split into three layers by where they live and who writes them:

- **Catalog** — `library.sqlite`, written by the scanner and by explicit user actions. StorageRoot, Track, Artist, Playlist, PlaylistTrack, ScanHistory. Album is *derived* from tracks and has no table.
- **Preferences** — `settings.json` (`AppSettings`, §6), written by the store on change. Saved output device, per-device exclusive-mode preferences and sightings, volume. Never joined to the catalog.
- **Runtime** — in memory in `AppStore`, gone at quit. `PlaybackSnapshot`, the queue (`QueueState`), the device table, notices. Refers to catalog rows by `TrackId` and to devices by UID; nothing here is persisted (feature 21 will persist a subset back into `settings.json`).

The key rule, from `AGENTS.md`: **relationships are managed in the application layer.** The schema declares no enforced foreign keys (`PRAGMA foreign_keys` is off; the `REFERENCES` clauses on the legacy tables are declarative only and disappear at the next rebuild). Related-id columns are plain integers, side tables join on natural keys, and insert/delete ordering and orphan cleanup are store code with tests.

### 3.1 Relationship diagram

```
┌─ Catalog (library.sqlite, scan-derived + user-curated) ─────────────────────────────┐
│                                                                                     │
│   StorageRoot ──1:n── Track ──────────── derives ───► Album (no table)              │
│     path_key            (storage_root_id,             group by (effective album     │
│     is_reachable         path_key) unique             artist, album title)          │
│     last_scan_at         tags · audio facts ·                │                       │
│        │                 cover_art_path                      │ aggregates            │
│        │                                                     ▼                       │
│        └──1:n── ScanHistory                          Artist (table, v5)             │
│                 (one row per scan run)                name_key = effective album    │
│                                                        artist string, unique        │
│   Playlist ──1:n── PlaylistTrack ──n:1── Track        counts · duration · cover     │
│     name             (playlist_id, position) PK       + enrichment / curation seams │
│                       track_id                                                      │
└─────────────────────────────────────────────────────────────────────────────────────┘
┌─ Preferences (settings.json) ───────┐   ┌─ Runtime (AppStore, in memory) ───────────┐
│  AppSettings                        │   │  PlaybackSnapshot · QueueState (TrackIds)  │
│   saved output UID · per-device     │   │  ManagedDevice table · notices            │
│   exclusive prefs + sightings ·     │   │  observed by surfaces via StoreRevisions  │
│   volume                            │   │                                            │
└─────────────────────────────────────┘   └────────────────────────────────────────────┘
```

Arrows mark ownership the application enforces, not database constraints.

### 3.2 StorageRoot — *a scanned folder*

A user-added directory (usually a NAS mount). Identity is `path_key`, the normalized path (`library/path.rs`), unique across roots; `path` keeps the display form. `is_case_sensitive` is probed at add time and drives how track paths under it are keyed; `is_reachable` and `last_scan_at_ms` record the last scan's view of the mount. Removing a root deletes, in one transaction and in this order, the playlist entries that pointed at its tracks, its scan history, its tracks, and the root row, then refreshes `artists` (§3.5).

### 3.3 Track — *one audio file*

The only scanned entity. Identity is `(storage_root_id, path_key)`; a rescan matches on it and updates in place, skipping files whose `modified_at_ns` is unchanged. Columns are the embedded tags (`title`, `artist`, `album`, `album_artist`, `year`, `genre`, disc/track numbers), the audio facts the quality badges need (`duration_ms`, `sample_rate_hz`, `bit_depth`, `channels`), the cover extracted into the cover cache (`cover_art_path`, `cover_art_mime_type`), and `added_at_ms` / `updated_at_ms`. Tracks are never edited by the user in Pulse; tag hygiene is a NAS problem by design (`mvp.md`).

### 3.4 Album — *derived, not stored*

An album is the group of tracks sharing `(effective album artist, album title)`, where the effective album artist is the shared SQL fragment `COALESCE(NULLIF(trim(album_artist), ''), NULLIF(trim(artist), ''), 'Unknown Artist')` defined once in `library/store/mod.rs` (`EFFECTIVE_ALBUM_ARTIST_SQL`). The Albums page, Album Detail, the genre filter, and the Artists refresh all group with that one expression; a "feat." credit on `artist` therefore never splits an album whose `album_artist` is set. Album-level facts (year, duration, quality ceiling, cover, added-at) are aggregates computed in the page query. There is no album id: routes carry `(artist, title)`.

### 3.5 Artist — *stored, refreshed by the app*

`artists` (schema v5, feature 11) stores each derived artist once. `name_key` is the effective-album-artist string exactly as the fragment produces it, so the join back to track groups is exact; `name` is the display form. The row carries the aggregates the Artists index reads directly — `album_count`, `track_count`, `total_duration_ms`, `earliest_added_ms`, `cover_art_path` (the first album's cover in the Albums ordering) — and nullable seams reserved for later features: `display_name` and `hidden` (curation), `mbid`, `photo_path`, `photo_source`, `enriched_at_ms` (metadata enrichment). Tags stay the source of truth for *which* artists exist: the store refreshes the table — recompute from `tracks`, upsert by `name_key` preserving `id` and the seams, prune rows whose group is gone — at the end of every scan and after every mutation that changes track groups (delete album or tracks, remove root), inside the same transaction as the mutation. The v4→v5 migration created the table and backfilled it from the existing tracks.

### 3.6 Playlist and PlaylistTrack — *user-curated order*

A playlist is a name plus an ordered list of track ids. `playlist_tracks` is keyed by `(playlist_id, position)`; reordering rewrites positions in one transaction. Entries point at tracks by id only; when a track disappears (rescan removal, album delete) the store removes the dangling entries explicitly, and the UI marks entries whose file is missing on disk without deleting them (`missing_track_ids` in the playback snapshot). Deleting a playlist deletes its entries first.

### 3.7 ScanHistory — *one row per scan run*

Per root, per run: counts of added/updated/removed/unsupported/errored files, whether removals were suppressed (an unreachable mount must not read as "everything deleted"), and an `outcome` of `completed`, `completed_with_errors`, `offline`, or `failed`. The Storage page reads the latest rows; nothing else depends on this table.

### 3.8 Runtime objects

`QueueState` (`queue.rs`) is the app-side play order — entries, index, shuffle/repeat, history, skip and failure accounting — owned by `Playback` inside the store and exposed through `PlaybackSnapshot` behind an `Arc`. `ManagedDevice` rows (§5) merge the live Core Audio device list with the stored device preferences; the Devices page and the output popover render the same rows. Neither is written to the catalog.

## 4. Startup And Ownership

`main.rs` loads or migrates `AppSettings`, creates one `Entity<AppStore>`, installs it as `GlobalAppStore`, registers playback shutdown for application quit, and opens the initial `Shell` window.

`AppStore` owns the app-side `Playback` adapter. `Playback` owns the `pulse_engine::PlaybackController`, the controller event receiver, the queue, device state, display state, and the cached snapshot collections exposed to GPUI surfaces.

`AppStore::new` starts one GPUI task that wakes every 16 milliseconds. Each tick drains pending controller events; every 125 ticks the same task also checks whether the active Core Audio device is still present. The task stops when the weak store entity can no longer be updated.

Application quit calls `AppStore::shutdown`, which forwards shutdown to the playback adapter and its controller before the process exits.

## 5. Store And Observe Contract

Surfaces access the shared entity through `global_app_store` and observe it through GPUI. A surface does not own a second playback adapter or poll the engine independently.

`StoreRevisions` contains separate monotonic counters for settings, devices, playback, and queue state. A surface saves the last revisions it consumed and calls `reactions_since` after a store notification to determine which dependent view state must be refreshed.

Every store command and every event-drain cycle finishes by taking a new internal revision snapshot. `AppStore` compares that snapshot with the previous one, increments exactly the changed domain counters, and calls `cx.notify()` only when at least one domain changed. A no-op command or poll neither bumps a revision nor notifies observers.

`playback_changed` destructures every field of `PlaybackSnapshot`. Adding a snapshot field therefore requires an explicit decision about whether it participates in the playback revision instead of silently inheriting an incomplete comparison.

`PlaybackSnapshot` stores queue state, the visible device list, and missing-track IDs behind `Arc`. Position events can therefore clone a snapshot with pointer copies for those collections. Queue, device, and missing-track revision checks use `Arc::ptr_eq`; the playback adapter replaces the corresponding `Arc` whenever that collection changes.

Active-device comparison includes the transient Core Audio device ID as well as the stable UID and display name. Managed-device sightings and persisted preferences use UID and name because a Core Audio ID can change, while the live active row remains ID-aware because the same UID can be rebound to a different current device instance.

The library catalog remains a separate `LibraryStore` owned by the library surface. Library loading and mutation are explicit surface tasks; playback, device, settings, and queue observation use `GlobalAppStore` revisions.

## 6. Settings And Migration

Current app settings live in `settings.json` under Pulse's platform data directory: `pulse` for release builds and `pulse-dev` for debug builds. `AppSettings` stores the saved output-device UID, per-device exclusive-mode preferences and sightings, volume level, and mute state.

Saving writes pretty JSON to a newly created temporary file, synchronizes it, renames it over `settings.json`, and synchronizes the parent directory on Unix. Loading applies serde defaults for missing fields and normalizes the device UID and volume range. Invalid JSON is moved to a unique `.corrupt` sibling and replaced with defaults.

When `settings.json` does not exist, `preferences.rs` reads the legacy flat files for output device, exclusive mode, volume, and mute from Pulse's platform configuration directory. It writes the complete JSON file before removing the successfully migrated legacy files. Corrupt legacy values are archived individually and replaced with that field's default so the other fields can still migrate.

Once `settings.json` exists it is authoritative and legacy app-setting files are left untouched. The updater's legacy `check-updates.disabled` preference is handled separately by the updater migration because update settings are not fields of `AppSettings`.

## 7. Surface Rules

`Shell` owns only window-level concerns: custom titlebar rendering and drag behavior, sidebar composition, route selection, search and settings overlays, and placement of the persistent playback row. Product behavior stays in the corresponding surface module.

Rendering surfaces may depend on GPUI, `AppStore`, library APIs, playback action types, and the shared `ui/` kit. Their sibling logic modules must remain free of GPUI imports and receive ordinary Rust values.

Search owns both the popover and its `EntityInputHandler` implementation because IME state is part of the search interaction. Settings owns the General, Update, and About pages. Library owns Albums, Tracks, Playlists, Storage, genre filtering, context menus, and page-local lifecycle and action code.

## 8. UI Kit Rules

Reusable controls live under `ui/` and expose builder-style APIs. Callers construct a control with `new`, add optional state or behavior through methods such as `variant`, `size`, `selected`, `disabled`, or event handlers, and then compose it into a surface.

The kit owns shared buttons, icon buttons, badges, toggles, tooltips, menus, overlays, scrollbars, settings rows and cards, empty states, and recurring playback/device visual treatments. Theme tokens remain in `theme.rs` and assets remain in `assets.rs`.

Do not create a page-local replacement for a component already provided by `ui/`. A visual primitive used by more than one surface moves into the kit; product-specific layout and behavior stay with the surface that owns them.

## 9. Boundary Summary

`surfaces/` renders product areas and converts interaction into app actions. `ui/` supplies reusable visual grammar. `app_store.rs` owns observable cross-surface playback state and notifications. `app_settings.rs` owns the persisted settings shape. `playback/` adapts the engine to app semantics. `library/` owns the local catalog and scan pipeline. `pulse-engine` owns audio playback and contains no GPUI code.
