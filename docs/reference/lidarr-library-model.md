# Lidarr as a Reference for Library Management

Notes from reading Lidarr (`Lidarr/Lidarr`, `src/NzbDrone.Core`, `develop`, 2026-08-29) as the reference for Pulse's artist-centric library and the metadata-enrichment feature. Lidarr is a downloader/organizer, so only part of it transfers; the parts that do are the domain model, the metadata pipeline, and the identification pipeline. Jason, 2026-08-29: "for the library management functions, you can check how lidarr does it."

## What Lidarr is built around

Everything is keyed by MusicBrainz ids. `Music/Model` has `Artist`, `ArtistMetadata`, `Album`, `Release`, `Medium`, `Track`, plus `Links`, `Member`, `Ratings`, `PrimaryAlbumType`, `SecondaryAlbumType`, `ReleaseStatus`, and the monitoring types. The split that matters:

- **`Artist` vs `ArtistMetadata`.** `Artist` is the library row — root folder, profiles, monitored flag, tags, the user's relationship to the artist. `ArtistMetadata` is the MusicBrainz-sourced record — name, sort name, disambiguation, overview, type/status, images, links, members, ratings, aliases — refreshed on its own schedule by `RefreshArtistService` and shareable. The library row points at the metadata row; the metadata row never depends on the library.
- **`Album` (a release group) vs `Release` (one edition of it) vs `Medium`/`Track`.** A user has an album; the album has many releases (editions, countries, formats); files are matched to one release's tracks.

`MetadataSource` is an interface set — `IProvideArtistInfo`, `IProvideAlbumInfo`, `ISearchForNewArtist`, `ISearchForNewAlbum` — with one implementation, `SkyHookProxy`, Servarr's own proxy in front of MusicBrainz (and fanart.tv for images). Refresh is a pipeline of services (`RefreshArtistService` → `RefreshAlbumService` → `RefreshAlbumReleaseService` → `RefreshTrackService`) driven by a last-sync timestamp per entity.

`MediaFiles/TrackImport/Identification` is the file-matching pipeline (`IdentificationService`), a port of beets' matcher:

1. `TrackGroupingService` groups local files that look like one release (by tags and folder).
2. `CandidateService` fetches candidate releases — from tag hints (artist, album, release ids) or, when tags are useless, from AcoustID fingerprints.
3. `DistanceCalculator` scores each candidate with a weighted distance over artist, album, year, track count, track titles/durations/numbers; `Munkres` (Hungarian assignment) maps local tracks to release tracks.
4. Best candidate below a threshold wins (`NormalizedDistance() > 0.15` triggers the fingerprint fallback); fingerprinting is gated by config `AllowFingerprinting` = `Never | NewFiles | Always`.

`Profiles/Metadata` ("metadata profiles") decide which primary/secondary album types and release statuses count as part of an artist's discography. "Library Import" scans a root folder, matches files to release groups, and imports them without moving files; `Organizer` renames/moves only when asked.

## What transfers to Pulse

- **The library/metadata split.** Pulse's `artists` table (schema v5) is the library row; MusicBrainz-sourced facts belong in a separate `artist_metadata` keyed by `mbid` that the library row points at, rather than growing seams on `artists` forever. Same later for albums (`album_metadata` keyed by release-group id) if album enrichment is wanted. Refresh runs on the metadata rows by `enriched_at`, independent of scans.
- **A provider interface, one implementation.** A `MetadataProvider` trait (artist search, artist info, album info, images) with a MusicBrainz + fanart.tv implementation; no proxy service — Pulse calls MusicBrainz directly at its 1 request/second with a proper `User-Agent`, and caches everything on disk. Enrichment is opt-in and off the render path (a scan-like background job with progress), never a fetch during paint.
- **Identification as tags → candidates → distance → threshold → fingerprint.** For Pulse the first target is artist-level identity (name + the artist's album titles → MusicBrainz artist search → candidate artists → a distance over name similarity and how many of the library's album titles appear in the candidate's release groups). Album-level matching (release-group ids) is a later step and can reuse the beets/Lidarr distance idea. Manual override — "this artist is MBID X" — is part of the design from day one, as Lidarr's `lidarr:<mbid>` add flow is.
- **"Unmapped" as a first-class state.** Lidarr keeps files it could not match visible as unmapped; Pulse should treat unresolvable artists (`######`, empty tags) as *unidentified* rows with the empty avatar, not as errors and not hidden.
- **Refresh by timestamp, not by scan.** `enriched_at_ms` + an interval; a rescan never re-fetches metadata.

## What does not transfer

- Monitoring, wanted lists, import lists, indexers, download clients, quality profiles, delay profiles, history of grabs — Pulse does not acquire music.
- The `Organizer` (renaming and moving files into `Artist/Album/` folders). Pulse's NAS layout is user-owned; tag hygiene is a NAS problem by design (`mvp.md`).
- SkyHook. Pulse talks to the sources itself.
- **AcoustID fingerprinting needs Chromaprint, which is LGPL-2.1.** AGENTS.md forbids GPL dependencies; LGPL is a different license but static linking into a closed-source app is not clear-cut. Decide explicitly before any fingerprinting work — likely by shelling out to `fpcalc` as an optional external tool rather than linking, or by not fingerprinting at all and relying on tags plus manual override.
