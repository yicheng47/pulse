# A placeholder album artist such as `######` becomes an artist

> Bug · P3 · filed 2026-08-30 from Jason's library (Faye Wong rips tagged `ALBUMARTIST=######`). Local note only — no GitHub issue. The files were fixed at the source with `metaflac`; this note is the Pulse-side robustness rule so the next badly tagged rip cannot create a phantom artist.

## Description

Pulse groups the Artists page and the album card's artist line by album artist, falling back to the track artist only when the album artist is empty after `trim` (`EFFECTIVE_ALBUM_ARTIST_SQL` in `backend/repo/mod.rs`). Some rippers and taggers write a placeholder instead of leaving the field out — `######` here (61 tracks, six albums, all with `ARTIST=王菲`). Pulse stores the placeholder verbatim, so the Artists grid shows an artist named `######` with those albums, while the same singer's other albums sit under 王菲.

## Expected Behavior

An album artist that carries no name — no alphanumeric character in any script (`char::is_alphanumeric` is Unicode-aware, so CJK counts as a name) — is treated as unset: the track's album artist is stored as `NULL` and the existing fallback picks the track artist. `######`, `----`, `???`, `*` all fall through; `王菲`, `Various Artists`, `!!!` (the band) do not — `!!!` is the known false negative and is accepted. The same rule applies to the track artist (falls to `Unknown Artist`) and to the album title (falls to the folder-name fallback). Titles are left alone.

## Steps To Reproduce

1. `metaflac --set-tag='ALBUMARTIST=######' some.flac` on a track whose `ARTIST` is set.
2. Scan its storage root; open Artists.
3. An artist `######` appears with that album; the track's artist is correct on the album page.

## Relevant Code

- `crates/pulse-app/src/backend/scan/metadata.rs:212` — `tag_text` trims and drops empty strings only; the placeholder check belongs here (a `has_name` predicate applied to artist, album artist, and album).
- `crates/pulse-app/src/backend/repo/mod.rs:33-35` — `EFFECTIVE_ALBUM_ARTIST_SQL` / `EFFECTIVE_TRACK_ARTIST_SQL`; unchanged, the fallback already does the right thing once the value is `NULL`.
- `crates/pulse-app/src/backend/ops/scan/` — existing rows keep the placeholder until the file is rescanned; the scanner rescans on mtime change, so a one-time refresh needs either a forced rescan or a migration that re-derives the three columns from the rule. Decide in the fix; a forced rescan from Storage is acceptable for P3.

## Environment

- OS: macOS 26.5.1
- Device / DAC: n/a
- Input file format: FLAC with Vorbis comments (`ALBUMARTIST=######`)
- Pulse version: v0.2.0

## Verification

- `sqlite3 library.sqlite "select album_artist, hex(album_artist), count(*) from tracks group by 1"` showed `######` (`23×6`) on exactly 61 tracks, all under `华语/王菲/`; `metaflac --show-tag=ALBUMARTIST` confirmed the tag in the files.
- Fix verification: a `metadata.rs` test that `######`, `---`, and whitespace map to `None` for artist / album artist / album while `王菲` and `!!!`-style names are kept; rescan of a tagged fixture lands the track under the track artist.
