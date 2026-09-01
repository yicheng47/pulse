# Now Playing Page

> Feature 56 · P2 · GitHub issue [#56](https://github.com/yicheng47/pulse/issues/56). The playback row is the only surface for the current track; clicking it should open a full now-playing view built around enlarged album art.

## Motivation

Pulse has no destination for "what's playing" — the current track lives only in the compact playback row, and the album art never renders larger than a thumbnail. A now-playing page is table stakes for a music player and the natural home for everything current-track-centric that comes later (lyrics once the library supports them, richer format detail). v1 is deliberately minimal: the enlarged art, the track identity, and clean entry/exit.

## Scope

- **Entry**: clicking the playback row's track-info cluster (art thumbnail, title/artist text) opens the page; the transport controls, seek strip, and popover buttons keep their existing behavior. The cluster is inert while nothing is loaded.
- **Surface**: the page takes the content area as an immersive mode — sidebar hidden, playback row stays visible and fully functional (it is the page's transport). A back affordance top-left and Escape both exit; navigating via any other route (queue popover jump, etc.) keeps the page open and updates it.
- **Content v1**: large centered album art (falling back to the existing placeholder treatment), track title, artist, album name, and the format/quality badge reusing the album-header badge component. Background treatment (flat vs. art-derived) is decided in the Pencil pass.
- **Live**: the page tracks `NowPlaying` events — track changes flip art and metadata; stop/idle exits the page.
- Cover art renders from the original file at page size, not the grid thumbnail cache.

## Non-Goals

- Lyrics display — future feature once the library models lyrics (synced `.lrc` or embedded tags); this page is where it will land, but nothing in v1 depends on it.
- Queue display on the page — the queue popover already covers it.
- Visualizations, animations, or audio-reactive artwork.
- A separate mini-player or detachable window.

## Implementation Phases

1. Pencil design: the Now Playing screen (art scale, identity block, badge placement, background treatment, back affordance), plus the playback row's hover affordance on the track-info cluster.
2. Implement the surface and routing: click target, immersive mode with sidebar hidden, back/Escape exit, `NowPlaying`-driven updates, full-size art decode.
3. Manual validation across track changes, stop, device switches, and empty state.

## Verification

- Unit tests: entry is inert with no loaded track; `NowPlaying` updates swap the page's track identity; stop exits the page state.
- `make verify` is green.
- Manual: click the row mid-playback — page opens with sharp full-size art (not an upscaled thumbnail); track advance flips the page contents; Escape and the back affordance both return to the prior library view; playback row remains fully operable while the page is open.
