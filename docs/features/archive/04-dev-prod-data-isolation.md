# Dev/Prod Data Isolation

> Feature 04 · P1. Discovered immediately after the v0.1.0 release: dev builds and the released app share the same data directory, so every `cargo run` opens — and can migrate — the production library.

## Motivation

The released `Pulse.app` and every development build resolve the same `~/Library/Application Support/pulse` (library.sqlite, cover cache) and the same `pulse/app-output-device.uid` preference. Before v0.1.0 that directory was dev data by definition; now it is the user's real library. The first dev branch that bumps the schema past v4 would silently migrate the production database on `cargo run`, with no path back for the release build (migrations are one-way). Scans, cover-cache rewrites, and preference writes from dev sessions land on production data the same way. Development must be unable to touch the released app's storage by construction.

## Scope

- **Profile-suffixed storage root.** Debug builds (`cfg!(debug_assertions)`) resolve `pulse-dev` instead of `pulse` for both the data directory (`library.sqlite`, `covers/`) and the config directory (`app-output-device.uid`). Release builds keep `pulse` — the shipped app's paths and TCC identity do not change.
- **Single seam.** All resolution already routes through `app_data_directory()` / `output_device_uid_path()` in `crates/pulse-app/src/preferences.rs`; the profile switch lives there and nowhere else. No call-site changes.
- **Fresh dev profile.** Do not copy the production `pulse` directory into `pulse-dev`; debug builds start with an empty library and add their own storage roots normally. A plain directory copy is unsafe because `tracks.cover_art_path` values are absolute and would continue pointing into the production cover cache.

## Non-Goals

- `pulse-cli` config separation — the CLI already keeps its own preference file, is a dev/agent harness by nature, and ships nowhere.
- An env-var override (`PULSE_DATA_DIR`-style) for arbitrary storage roots — nothing needs it yet; tests use temp directories already.
- Migrating or renaming the release directory, or any release-build behavior change. `cargo run --release` intentionally still resolves `pulse`: it is the pre-release smoke path for the exact bits that ship.
- In-app profile switching or any UI surface.

## Implementation Phases

1. Add the `cfg!(debug_assertions)`-gated directory name in `preferences.rs` and thread it through the two resolvers.
2. Unit tests: debug test builds assert both resolved paths live under `pulse-dev`; the existing filename-shape tests stay unchanged.

## Verification

- `make verify` green.
- Manual: with no `pulse-dev` profile present, `cargo run` creates an empty dev library under `pulse-dev`; after adding a disposable storage root, confirm zero `tracks.cover_art_path` rows reference `/Application Support/pulse/covers/` and all writes stay under `pulse-dev` (verify with `fsevents`/`ls -lT` timestamps on both directories). The bundled release `Pulse.app` still opens the untouched `pulse` library.
