# Metadata Enrichment

> Feature 26 · P2. Identify each library artist against MusicBrainz, fetch an artist photo from fanart.tv, and show the photo on the Artists grid and detail pages — opt-in, cached on disk, refreshed by age, never on the render path. A new **Settings ▸ Metadata** page owns the switch, the API key, the triggers, and the status. Design drafted 2026-08-30, awaiting Jason's approval. Reference: [`docs/reference/lidarr-library-model.md`](../reference/lidarr-library-model.md).

## Motivation

The Artists page (feature 11) shows an album cover where an artist photo belongs, because Pulse knows nothing about an artist beyond the tags in the files. Lidarr's model transfers cleanly: a library row (`artists`, ours since schema v5) that points at a MusicBrainz-keyed metadata record, one provider interface with one implementation, identification from tags with an explicit "unmapped" state, and refresh by timestamp rather than by rescan. Jason, 2026-08-30: "we at least need a setting page for this, and then we need to describe when we would trigger the metadata grip" — both are the core of this spec.

## Design Source

`design/pulse-desktop.pen`:

- `Settings / Metadata` — `s5Q1ti`. Same shell as the other Settings pages; the settings island (`qPd6E`) gains `Nav / Metadata` (`sU7Ok`, lucide `globe`) as the third SETTINGS item after General and Output.
- Page title "Metadata"; `Column` `ssaSe` (820 wide, gap 22):
  - **ONLINE SOURCES** (`bctYx`, card `sWbc9`): `Row / Enrich artists` `Y801zQ` — "Enrich artists from MusicBrainz" / "Looks up each artist's identity and photo online. Off by default — nothing leaves this Mac until you turn it on." · Toggle. `Row / fanart.tv API key` `qCkzS` — "fanart.tv API key" / "Needed for artist photos. Get a personal key at fanart.tv/get-an-api-key; identity still works without one." · `Key Field` `P5Yp8G` (300 × 36, `bg-inset`, 1 px `border`, radius 6, `key-round` icon, masked mono value).
  - **ENRICHMENT** (`R5WLp`, card `hPcm4`): `Row / After scans` `h6XeQ2` — "Run after library scans" / "Identifies artists added or changed by a scan as soon as the scan finishes." · Toggle. `Row / Refresh` `VPwVF` — "Refresh cached facts" / "Identity and photos are checked again after 30 days; a rescan never re-fetches metadata." · value "30 days" (informational, no control). `Row / Status` `waqpX` — "Status" / "142 of 160 artists identified · 12 unmapped · 6 waiting · last run 2 h ago" · `Button / Secondary` "Clear cache" (`trash-2`) · `Button / Primary` "Enrich now" (`sparkles`).
  - `Attribution` `sB8dt`: "Artist data from MusicBrainz (CC0). Photos from fanart.tv, used under its API terms." (11 px `text-muted`).
- Artists grid (`ixG6j`) and detail (`rA7SD`) are unchanged: the avatar slot already draws `photo → album cover → empty`; this feature fills the photo.

## Scope

### 1. Data model (schema v6)

- New table `artist_metadata` keyed by `mbid TEXT PRIMARY KEY`: `name`, `sort_name`, `disambiguation`, `artist_type`, `country`, `begin_year`, `end_year`, `photo_path`, `photo_source` (`fanart`), `fetched_at_ms`. MusicBrainz facts only; no foreign keys (app-layer relation, per the project rule).
- `artists` keeps its existing seams — `mbid` (the pointer), `photo_path` (denormalized display copy the job writes so the grid's read path stays as it is), `enriched_at_ms` (the refresh clock) — and gains `enrichment_state TEXT` (`unidentified` default · `identified` · `unmapped` · `failed`), `enrichment_attempts INTEGER`, `enrichment_retry_after_ms INTEGER`, `enrichment_error TEXT`.
- Photos live in `<app data>/artists/<mbid>.jpg` next to `covers/`; `Clear cache` deletes the directory and the `artist_metadata` rows and resets every artist to `unidentified` with its photo cleared (album-cover fallback returns at once).
- `settings.json` gains `metadata: { enabled: false, fanart_api_key: null, enrich_after_scan: true }`. The key is stored in the settings file like every other preference (user-level file, mode 0600 on write); Keychain is a non-goal.

### 2. Provider

- A `MetadataProvider` trait in `backend/metadata/`: `search_artists(name) -> Vec<ArtistCandidate>`, `artist_release_groups(mbid) -> Vec<String>`, `artist_photo(mbid) -> Option<Bytes>`. One implementation: MusicBrainz WS/2 (`https://musicbrainz.org/ws/2/artist?query=…&fmt=json`, `…/release-group?artist=<mbid>&limit=100`) plus fanart.tv (`https://webservice.fanart.tv/v3/music/<mbid>?api_key=…`, highest-liked `artistthumb`; skipped when no key). HTTP through `ureq` with rustls (MIT/Apache; no GPL), one blocking call at a time on the job thread, `User-Agent: Pulse/<version> (https://github.com/yicheng47/pulse)`, MusicBrainz's 1 request/second honoured with a token bucket, 10 s timeouts, and 503/429 backoff.
- Tests use recorded JSON fixtures behind the trait; nothing in `make verify` touches the network.

### 3. Identification

For one artist row (name = the library's effective album artist, albums = its album titles):

1. Skip names with no alphanumeric character in any script (the [placeholder bug](../bugs/placeholder-album-artist.md) rule) → `unmapped` without a request.
2. `search_artists(name)`; keep the top 5 candidates.
3. Score each candidate: name similarity after Unicode NFKC + case folding + diacritic stripping (exact = 1.0, else normalized Levenshtein), plus alias matches from the search response; candidates below 0.85 are dropped.
4. If exactly one candidate remains → identified. If several remain, fetch each one's release-group titles (at most 3 extra requests) and pick the candidate with the most library album titles matched (same normalization); ties or zero matches → `unmapped`.
5. On `identified`: upsert `artist_metadata`, set `artists.mbid`, then fetch the photo if a key is set and write `artists.photo_path`; `enriched_at_ms = now`, `enrichment_state = identified`.
6. Network or provider errors → `failed`, `enrichment_attempts += 1`, `retry_after = now + 24 h × 2^(attempts − 1)` capped at 7 days; the artist keeps whatever it had.

`unmapped` is a first-class, visible state: the artist stays in the grid with the album-cover avatar. Manual override (set the mbid by hand) is parking-lot scope for the curation feature.

### 4. Triggers — when the job runs

All triggers are no-ops while `metadata.enabled` is off, and the job never runs on the render path or the audio thread. One job at a time; a running scan takes precedence (the job waits for the scan to finish).

| Trigger | Selects | Notes |
|---|---|---|
| **Enrich now** (Settings ▸ Metadata) | every artist that is `unidentified`, or `identified`/`unmapped` with `enriched_at_ms` older than 30 days, or `failed` past its `retry_after` | The full pass; the status row shows progress (`n of m`) and the button becomes Cancel. |
| **After a library scan** (when "Run after library scans" is on) | artists whose row was created or whose `album_count` changed in that scan, plus `unidentified` rows | Runs on `ScanOutcome::Completed` / `CompletedWithErrors` from `ops/scan`; skipped for cancelled scans. |
| **Launch refresh** | `identified`/`unmapped` rows with `enriched_at_ms` older than 30 days, and `failed` rows past `retry_after` | Once per launch, 30 s after the library opens, at most 50 artists per launch so a stale library trickles in under the rate limit instead of saturating it. |
| **Turning the switch on** | same as Enrich now | The first pass starts immediately after enabling; turning the switch off cancels a running job and leaves cached data in place. |
| **Pasting a key when artists are already identified** | `identified` rows with no photo | Photos only, no identity requests. |

A rescan alone never re-fetches metadata (Lidarr's rule): the clock is `enriched_at_ms`, not scan time.

### 5. Settings page and status

The page from the Design Source, backed by a gpui-free `MetadataSettingsModel` (`enabled`, `has_key`, `after_scan`, counts by state, last run, job progress). The status row's text is built from the counts; while a job runs it reads "Identifying 12 of 48 · 王菲…" and the primary button is Cancel. Errors from the last run surface in the description (e.g. "Offline — 6 artists will be retried"). Toggling `enabled` off cancels; `Clear cache` asks for confirmation through the existing modal kit.

### 6. Surfaces

The Artists grid and detail read `artists.photo_path` first, as they already do; the artist detail page adds nothing in this feature (a "Refresh metadata" action and the mbid override belong to curation).

## Non-Goals

- Audio fingerprinting (Chromaprint/AcoustID) — a licensing decision (LGPL-2.1) before any work, and unnecessary for a tagged library.
- Album-level metadata (release-group ids, album art from the network), biographies, links, or members.
- Manual override / merge / hide (parking lot: curation on the `artists` seams).
- A proxy service; Pulse talks to MusicBrainz and fanart.tv directly.
- Keychain storage for the API key.

## Implementation Phases

1. **Backend**: schema v6, `artist_metadata` repo, the provider trait + MusicBrainz/fanart implementation with fixture tests, the identification scorer with its normalization tests, the job runner with cancellation and rate limiting, `settings.json` fields, and a `pulse-cli enrich --dry-run <library>` subcommand that prints candidate scores per artist so matching can be validated on Jason's library before any UI exists. No visible change in the app.
2. **Settings page + triggers**: Settings ▸ Metadata per the design, the Metadata nav row, the five triggers, status/progress, Clear cache, photos on the Artists grid and detail via the existing `photo_path` read.

## Verification

- `make verify` green after each phase; the gpui gate and SQL gate still hold (`backend/metadata` is gpui-free; SQL stays in `repo/`).
- `grep -rn "ureq\|musicbrainz" crates/pulse-app/src` shows the client only under `backend/metadata/`.
- Phase 1 on Jason's library: `pulse enrich --dry-run` identifies the well-tagged artists (王菲, Western artists with Latin names) with confidence and reports `unmapped` for `######`-style rows; no writes.
- Phase 2 manual: enable the switch → the first pass runs and the Artists grid fills with photos as it goes; disable → the job stops; paste a key later → photos arrive without identity requests; rescan → no requests; relaunch after 30 days (or with a clock override for testing) → the launch refresh trickles; Clear cache → covers return immediately.
