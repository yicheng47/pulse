# MVP Scope

> Product definition for the first usable Pulse desktop release. This is narrower than the long-term product vision.

## Goal

Pulse MVP is a macOS local music player that proves the core wedge: beautiful, fast, native-rate playback for owned PCM music libraries.

The MVP should be good enough that a local FLAC/ALAC listener can point Pulse at a NAS or local folder, browse albums/tracks/playlists, choose an output device, and play music reliably through the current AUHAL engine path.

## Primary User

The primary user owns a curated local library, often on NAS storage, and cares more about clean playback, fast browsing, and output-device control than streaming discovery.

They should not need to understand Core Audio, hog mode, device IDs, or sample-rate switching to use the app.

## MVP Must Include

### Playback

- Engine-owned `PlaybackController` inside `pulse-engine`.
- Play one local file from the app.
- Pause, resume, stop, and seek.
- Track progress and duration.
- End-of-track handling.
- Native-rate AUHAL playback through the selected Core Audio output device.
- Hog mode and sample-rate switching through the existing HAL control path.
- Clear errors when the selected device is unavailable, hogged, or incompatible.

### Output Device

- List Core Audio output devices.
- Select the Pulse output device.
- Persist selected device by UID, not transient `AudioDeviceID`.
- Show active output device in the playback row.
- Show source/output format in plain language: sample rate, bit depth, channel count where available.

### Library Storage

- Add one or more storage roots.
- Support local folders and mounted NAS folders.
- Show root online/offline status.
- Rescan a root.
- Remove or edit a root.
- Keep storage wording as `Storage`, under the `MANAGE` sidebar section.

### Library Scanner

- Scan PCM files only: FLAC, ALAC, AIFF, WAV.
- Store library metadata in SQLite.
- Extract enough tags for MVP browsing: title, artist, album, album artist, track number, disc number, duration, sample rate, bit depth, channels, file path, modified time.
- Extract or cache embedded cover art when available.
- Handle missing tags without breaking browsing.

### Browsing

- Sidebar sections from the current design direction: `LIBRARY`, `MANAGE`, `OUTPUT`.
- Primary library destinations: Albums, Tracks, Playlists.
- No top-level Artists page in MVP.
- Artist is metadata and a filter/facet for Albums and Tracks.
- Albums page with grid/list browsing.
- Tracks page with dense table browsing.
- Playlists page with manual playlists and selected-playlist detail.
- Storage page with roots, status, catalog summary, and selected-root detail.

### Search And Filters

- Search across albums, tracks, artists as metadata, and playlists.
- Search results should resolve to playable or browsable objects: albums, tracks, playlists.
- Basic filters: genre, artist, format/hi-res, recently added.
- Sort albums and tracks by title, artist, album, date added, release year, and duration where applicable.

### Playlists

- Create manual playlist.
- Rename playlist.
- Delete playlist.
- Add tracks to playlist.
- Remove tracks from playlist.
- Reorder tracks inside a playlist.
- Play playlist from the first selected item.

### Now Playing

- Bottom playback row based on the current design direction.
- Show track title as primary text.
- Show `artist - album` as secondary text.
- Show cover art.
- Show play/pause, previous, next, seek progress, output device, format, and queue count.
- Keep visualizers, lyrics, and editorial context out of the MVP unless they are trivial placeholders.

### App Shell

- Tauri desktop app shell.
- Persist window-safe app settings.
- Dark cyberpunk design system from `design/pulse-desktop.pen`.
- No landing page; first screen is the usable app.

## MVP Should Not Include

- DSD, DoP, DSF, DFF.
- Video playback or video library support.
- Streaming-service integrations.
- FFmpeg, libmpv, or GPL dependencies.
- Raw HAL integer bit-perfect engine.
- Hard bit-perfect marketing claims.
- Smart Radio.
- Metadata enrichment from MusicBrainz, Discogs, Last.fm, Wikipedia, or Cover Art Archive.
- Synced lyrics.
- Spectrum analyzer or VU meters beyond static/placeholder UI.
- EQ, DSP, replay gain, normalization, crossfade, or volume leveling.
- Cloud sync.
- Mobile apps.
- Multi-room playback.
- App Store packaging.

## Design Status

The current Pencil work is a strong foundation, not a complete product design.

Covered enough to guide MVP implementation:

- Cyberpunk design system direction.
- Sidebar structure.
- Albums page.
- Tracks page.
- Playlists page.
- Storage page.
- Playback row direction.

Still needs design detail before implementation:

- Output device settings surface.
- Search results and empty states.
- Album detail.
- Track detail or inspector behavior.
- Playlist editing flows.
- Storage add/edit/rescan states.
- Error states for missing storage, hogged device, decode failure, and scan failure.
- Loading states.
- Queue drawer or queue detail.

## Acceptance Criteria

The MVP is acceptable when these flows work end to end:

1. Add `/Volumes/Media/Music` as a storage root.
2. Scan the root and build a local SQLite library.
3. Browse Albums, Tracks, and Playlists.
4. Search for an artist and see albums/tracks, not an Artists destination.
5. Select an output device and persist it by UID.
6. Play a FLAC file through the app with clean sound.
7. Pause and resume without restarting from zero.
8. Seek within the current track.
9. Play the next/previous track in a playlist or queue.
10. Disconnect or unmount a storage root and see a clear offline/error state.
11. Attempt playback with an unavailable output device and see a clear error.

## Engineering Order

1. Add the `pulse-engine` playback controller.
2. Wire thin CLI smoke commands to the controller.
3. Add Tauri playback commands/events.
4. Implement output-device selection and persistence in the app.
5. Implement storage roots and scanner.
6. Implement SQLite library store and search.
7. Implement Albums, Tracks, Playlists, and Storage pages from the current design baseline.
8. Wire playback row and queue behavior to the controller.
9. Add missing MVP design detail passes as needed before each UI slice.

## Release Label

Call this `v0` or `MVP`, not `v1.0`.

`v1.0` should wait until the app has broader design coverage, more hardware smoke coverage, metadata polish, and packaging confidence.
