# Scan Error Detail

Feature 73 · P3 · GitHub issue [#73](https://github.com/yicheng47/pulse/issues/73). Filed 2026-08-31 after a live case: the Storage page said `ISSUES · 1 file errors` and the only way to learn *which* file and *why* was to query `scan_history` and probe file signatures by hand — the culprit was a zero-byte FLAC from a dead download.

## Motivation

Per-file scan errors (`ScanFileError { path, message }`) exist only in the in-session `ScanReport`; `scan_history` persists just the counts. So the Storage page can say a root has errors but can never say which files or what went wrong — after a relaunch even the session detail is gone, while the count keeps accusing the root. An error badge that cannot be investigated trains the user to ignore it.

## Scope

- Persist per-file errors with the scan session: a `scan_errors` table (scan_history id, path, message — app-layer relationship, no foreign keys per the schema rule), replaced on each completed scan of the root, capped (e.g. 100 rows per scan) so a mass failure cannot bloat the store.
- Storage page: the `ISSUES` status opens an error detail — file path, human-readable reason, and when derivable a suggested action (e.g. "file is empty (0 bytes) — re-download or remove"). Reveal in Finder per row.
- Message quality pass at the source: map the common failure shapes to plain language (empty/truncated file, unreadable metadata, permission denied, artwork cache failure) instead of raw error chains.
- The badge clears when a rescan completes clean; the detail shows the last completed scan's errors with its timestamp.

## Non-Goals

- No auto-repair, quarantine, or deletion of failing files.
- No re-download or store integration (the music flow owns acquisition).
- No change to what counts as unsupported vs error; unknown extensions stay silently skipped until [feature 71](71-dsd-over-dop.md) claims the DSD ones.

## Implementation Phases

1. Persistence: `scan_errors` table + repo module, written with the scan-history row; retention cap; migration.
2. Design: Pencil pass for the error detail surface (small — a list under the existing Storage failure treatment, feature e2xOQ's language).
3. UI: detail view per the design; message mapping; Reveal in Finder.

## Verification

- `make verify` green; repo tests for write/replace/cap; message-mapping unit tests (zero-byte file, permission denied, corrupt header fixtures).
- Manual: a planted zero-byte FLAC shows the badge, the detail names the file with the plain-language reason, Reveal in Finder lands on it, and a clean rescan clears both badge and detail.
