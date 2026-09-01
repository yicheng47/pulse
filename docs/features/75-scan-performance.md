# Scan Performance

Feature 75 · P2 · GitHub issue [#75](https://github.com/yicheng47/pulse/issues/75). Decided by Jason, 2026-09-01: parallel extraction, the folder-art memo, and the batched writer all land; M4.

## Motivation

The scan is one thread doing everything in sequence (`ops/scan.rs`): walk, then per file a lofty metadata read, a folder-artwork `read_dir`, and one SQLite transaction. Sequential per-file IO is latency-bound — on an SMB/NAS root every file pays a network round-trip, so wall time is dominated by waiting, not work. Two artwork behaviors also do repeated work: `discover_folder_artwork` lists the parent directory once **per track** (a 20-track album without embedded art pays 20 listings), and unchanged tracks with no cover re-probe folder art on **every rescan, forever** (`scan.rs` unchanged-track branch) — an artless library never stops paying it.

Change detection itself is already right (unchanged `path_key` + `modified_at_ns` skips the metadata read) and stays as is.

## Current Concurrency Model (unchanged by this feature)

One `rusqlite::Connection` inside `LibraryStore`; no lock, no pool. The library surface owns it as `Option<ops::Store>`; a scan `take()`s it and moves it into the `pulse-library-scan` thread, and UI actions that need the DB fail fast with the store-busy message until `ScanFinished` hands it back. WAL + `synchronous=NORMAL` + `busy_timeout(5s)` are set (WAL matters for feature 55's cross-process readers). The scan thread therefore already has exclusive write access — parallelism goes into the pure-filesystem work, never into SQLite.

## Scope

- **Folder-art memo (per scan)**: `discover_folder_artwork` results memoized per parent directory in a scan-lifetime map, negative results included — one `read_dir` per directory per scan, and the unchanged-artless-track re-probe collapses to a map hit after the first track of the directory.
- **Batched writer transactions**: the per-file transaction becomes one transaction per chunk (~200 files or on a time boundary), committed at chunk edges; cancellation checks stay per file and a cancel commits the finished chunk rather than losing it. Scan-history/report semantics unchanged.
- **Parallel extraction**: a bounded pool (capped at CPU count, small fixed queue so embedded-artwork buffers can't pile up in RAM) runs `extract_metadata` + folder-art discovery; the scan thread stays the single writer, applying results and owning the store exclusively as today. Exact add/update/skip/error counts, per-file error attribution, `removals_suppressed`, and per-file progress events are preserved; results may apply out of walk order (nothing in the schema depends on insertion order).

## Non-Goals

- No directory-mtime pruning — APFS and SMB mtime semantics differ enough to silently miss changes; the honest full walk stays.
- No FSEvents/watcher-driven incremental scan — a separate feature if ever wanted.
- No second read-only connection to lift the store-busy UX during scans — separate concern, and WAL already permits it later.
- No schema changes, no change to `LibraryStore` ownership or the ops-layer API.

## Implementation Phases

1. **Folder-art memo**: the per-scan directory map in `apply_reachable_scan`, covering both the new-track and unchanged-artless paths.
2. **Batched transactions**: reshape the writer loop to chunked transactions with per-file cancel checks and chunk-edge commits.
3. **Extractor pool**: bounded worker pool feeding the writer over a channel; the writer applies results, counts, and reports progress.

Each phase ships on its own; 1 and 2 are small and de-risk the loop reshape before 3.

## Verification

- `make verify` green; scan fixture tests keep byte-identical reports (counts, errors, removals) against the sequential baseline on the same tree.
- Memo: a fixture album with N artless tracks performs one directory listing (observable via a counting shim or temp-dir instrumentation); a rescan of unchanged artless tracks does no per-track listings beyond the memo fill.
- Batching: cancel mid-scan keeps completed chunks (tracks visible after cancel), loses at most the open chunk; no partial file ever commits.
- Parallel: same fixture tree scanned sequential vs parallel yields identical library contents; cancellation stops within a bounded number of in-flight files.
- Manual: before/after wall time on Jason's real NAS root, recorded in the impl note.
