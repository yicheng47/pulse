# 0012 — Library scan and store

> The headless half of stages 9 and 10: storage roots, the file scan, tag extraction, and the SQLite store. No UI. The Storage screen and the library browsing surfaces are stage 11 and stay blocked on their Pencil passes.

## Goal

Point Pulse at a folder, have it find the music, read what it can, and persist it. Everything here is testable without a window, which is the point — the GPUI window cannot be verified by agents in this sandbox, so the library gets built and proven as plain Rust first and rendered later.

## Placement

**Library code lives in `crates/pulse-app`, as a `library/` module. Not a new crate, not `pulse-engine`.**

`arch/pulse-engine.md` is explicit that SQLite, library scanning, and artwork belong outside the playback engine, so the engine is out. A separate `pulse-library` crate was considered and rejected: `pulse-app` is the only consumer, `pulse-cli` exists to prove engine and playback behavior rather than library behavior, and `pulse-app` modules are perfectly unit-testable — stage 8's preferences parser already is. If `pulse-cli scan` is ever wanted for the agent/MCP harness, extracting a module into a crate is mechanical and should happen when that second consumer actually appears.

## Dependencies

Add to the workspace and to `pulse-app`:

- `rusqlite` — **with the `bundled` feature.** Without it the binary links whatever SQLite the host ships, which varies across macOS versions and would make a packaged `.app` depend on the user's system library. Bundled is the right default for a shipping desktop app.
- `lofty` — tag and embedded-cover reading. Already named in `arch/tech-stack.md` as the intended choice.

Prefer a hand-rolled `std::fs` recursion for the directory walk over adding `walkdir`. Reach for `walkdir` only if symlink-loop detection or permission-error handling gets genuinely fiddly on real NAS mounts — and if you do, say why in the log.

## Data model

Driven by what the Storage screen (`C6IrDC`) actually displays, plus `product/mvp.md`'s scanner requirements. Read the design through the `pencil` MCP before finalizing field names; `.pen` files are encrypted, never Read or Grep them.

**Storage roots** — path, display name, when added, last scan time, and whether the root is currently reachable. A root that goes offline (unmounted NAS) keeps its rows; do not delete a library because a drive was unplugged.

**Tracks** — file path, storage root, title, artist, album, album artist, track number, disc number, duration, sample rate, bit depth, channels, file size, and modified time. Every tag field is optional: files with no tags must still be scannable and browsable, per `mvp.md`.

**Scan history** — per run: root, start and finish time, counts of added/updated/removed, and outcome. This is design-driven rather than required by `mvp.md`: the Storage screen shows both a "SCAN — 2 min ago" tile and a Recent Activity panel, and history cannot be backfilled after the fact, so recording it now is cheaper than retrofitting.

Per-root album and track counts, catalog totals, and footprint size are all derivable by query from tracks. Do not denormalize counters until a query proves too slow.

**Cover art** — extract embedded art during the scan, since the file is already open, and cache it to disk with the cache path stored on the row. Re-scanning a NAS library purely to pick up covers later is the outcome to avoid.

**Defer FTS5.** Search is not in this stage, and adding an FTS virtual table later is a purely additive migration: create the table, backfill from existing rows. Note the deferral in the schema so it is a decision rather than an oversight.

## Scan behavior

- PCM only: FLAC, ALAC, AIFF, WAV. No DSD, no video, no streaming — the standing project constraint.
- **Incremental by modified time.** A full re-read of a large NAS library is slow enough to make rescans painful; skip files whose path and mtime are unchanged.
- Handle a file that vanished between walk and read without aborting the run. Partial failure should degrade to a per-file error, not a failed scan.
- Report progress as the run proceeds — a callback or channel is fine. Stage 11 needs it for the scan-progress UI, and stage 13 for loading states. Do not build UI plumbing here, just make progress observable.
- The scan API is synchronous and blocking. The caller decides threading; the app will run it off the main thread when the UI lands.

## Testing

Follow the existing pattern: `pulse-engine`'s decode tests synthesize buffers in memory rather than committing audio files, and there are no binary fixtures anywhere in the repo. Keep it that way.

- Store logic against an in-memory SQLite (`:memory:`): insert, incremental update, removal, offline-root behavior, and the derived counts the Storage screen will ask for.
- Walk and filter logic against temp directories of empty files with the right extensions — no real audio needed to prove extension filtering, recursion, and mtime skipping.
- Tag extraction needs a real file. Generate a minimal WAV in-test (a header plus a few samples) rather than committing a fixture. If FLAC or ALAC tag paths need coverage beyond that, say so rather than quietly committing binaries.

## Verification

- `make verify` green: check, tests, clippy under `-D warnings`, fmt.
- Scan a real directory from a temporary test binary or an `#[ignore]`d test pointed at `/Volumes/Media` and report counts and timing. This is the one place a large real library tells you something a fixture cannot.
- Confirm a second scan of an unchanged directory is substantially faster than the first — that is the incremental path working.
- Confirm an offline root keeps its rows.
- **Jason only:** whether the scan is fast enough to feel right on the real NAS, and whether anything in his library is misparsed. Agents can measure; only he can judge the library is correct.

## Risks

- Scan performance on a large NAS library is the main unknown. Measure and report actual numbers rather than asserting it is fine.
- `lofty` and `symphonia` may disagree about duration or format details for the same file. The engine's report is authoritative for playback; the library's is for display. If they diverge noticeably, note it rather than silently preferring one.
- Cover-art caching can grow without bound. Keep the cache keyed so a rescan replaces rather than duplicates entries.
- Path handling on case-insensitive volumes and with unicode filenames is a classic source of duplicate rows. Decide how paths are normalized before storing them.

## Non-goals

- No Storage screen, no Albums/Tracks/Playlists UI — stage 11, blocked on design.
- No search query API and no FTS index — deferred above.
- No playlists, no metadata enrichment, no Smart Radio.
- No playback changes. The engine and the playback row are untouched by this stage.
- No `pulse-cli` scan command. The CLI stays an engine and playback harness.
