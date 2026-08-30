# App Shell Overhaul

> Feature 28 · P1. Rebuild the window shell to the approved design: a full-width header (brand │ search … update · settings), the library sidebar as a floating island, the page flat beside it, the playback row spanning the whole window, and Settings restructured into sections with the Devices page folded in as Settings ▸ Output. Design approved by Jason on 2026-08-30; design commit `06f83d9`.

## Motivation

Today the sidebar runs the full window height with the brand and the Settings row inside it, the top bar and the playback row live only over the content column, and the device manager is a library destination. Jason, 2026-08-30: move Settings to a gear at the top right so the player can span the bottom; make the header full width too; make the sidebar island-like; put Output into Settings; give the Settings sidebar sections. The result centers the transport on the window axis, frees the sidebar to be navigation only, and gives Settings a home that scales.

## Design Source

`design/pulse-desktop.pen`:

- `Library / Main` — `JPObg` (the reference frame; every `Library / *`, `Playback — *`, and `App — *` frame is rebuilt on the same shell).
- `Top Bar / App` component — `Y0hrAW`: 74 px, `bg-page`, padding `0 28 0 24`, gap 16: Brand Zone (Traffic Lights ref `QQFSz` · 2 px gap · app mark 24 px accent in a 32 px box · "Pulse" 22 px display bold) │ Divider (1 × 28, `border`) │ Search input (`VoSds`, 420 × 36) │ fill spacer │ Actions Zone (gap 12): Update Hint `l0QhO` (22 px `accent-soft` circle, 13 px accent `arrow-down`, visible only when an update is ready) · Settings Button `Q7kyI` (32 px, `settings` 18 px `text-secondary`; active on Settings pages: `bg-elevated` fill, accent icon).
- `Sidebar / Island` component — `IqX8X`: 236 wide, radius 12, `bg-surface`, 1 px `border`, padding `20 12 16 12`; sections 32 px apart, header → items 10, rows 39 px (`padding 10 12`, icon–label gap 12), items gap 4; LIBRARY (Albums · Artists · Tracks · Playlists) · MANAGE (Storage); no brand, traffic lights, footer, or OUTPUT group. Active row: `bg-elevated` fill, accent icon, `text-primary` label; idle rows transparent, `text-muted` icon, `text-secondary` label.
- Shell layout (in `JPObg`): root vertical — header · `Upper` (horizontal: `Sidebar Island Slot` with padding `12 0 12 12` holding the island at fill height · `Main` flat on `bg-page`, fill) · `Bottom Player`. The island is inset 12 px from the header, the window edge, and the player; the page is flush with header and player and starts at x = 248.
- `Bottom Player` component — `l6NrTs`: 1440 × 92 full width; Now Playing (317) · Transport Center (fill, centered on the window axis) · Output Slot (317, right-aligned); same content as today's row.
- `Sidebar / Settings` component — `qPd6E`: same island styling; Back to library · SETTINGS (General · Output) · APP (Update · About), 32 px between sections.
- `Settings / General` `C5mRG`, `Settings / Output` `XmoeK` (the former Library / Devices page content), `Settings / Update` `D9PDB`, `Settings / About` `Tnibv` — Settings pages on the same shell with the settings island in the slot and the gear active in the header.
- Overlays keep their anchors: search popover under the header's search input (x 223), page popovers/menus/banners shifted with the page (+12 px).

## Scope

1. **Shell layout.** `Shell` renders header · body row · player as three full-width bands; the body row is the sidebar island slot plus the page. The titlebar drag area becomes the header (traffic lights move into it). The playback row keeps all behavior; only its width and centering change.
2. **Header.** Brand, divider, the existing search input and popover, the update hint (feature 23's "release ready" signal, relocated from the sidebar's Settings row), and the settings gear. Gear click opens Settings; `Cmd-,` and the app menu keep working (feature: menu bug fix). The gear shows the active state while Settings is open.
3. **Sidebar island.** Kit-level: the library sidebar renders as the island with the approved spacing and the neutral active-row style; brand, traffic lights, Settings footer, and the OUTPUT group are removed. The active-row style is shared by the Settings sidebar.
4. **Page.** The library and settings pages render flat on `bg-page`, flush with header and player, with today's 28 px inner padding; the Albums page shows the genre-filter control as designed (already the case in the app).
5. **Settings.** Settings becomes a page in the body row with its own island sidebar: Back to library · SETTINGS (General, Output) · APP (Update, About). `Output` is the device-management page (feature 22) moved from the library sidebar; its `Destination::Devices` route and any sidebar entry are removed; deep links (output popover "Manage devices"-style entries, if any) open Settings ▸ Output.
6. **Update hint.** Rendered in the header's Actions Zone when an update is available; the sidebar hint is removed.

## Non-Goals

- Any change to page content beyond placement (no album grid, tracks, or settings-row redesign).
- Window resizing rules beyond what exists; minimum window size stays.
- A collapsible sidebar (feature 09) — the island is the same width as today.
- Queue drawer or anything new in the island's empty lower area.

## Implementation Phases

1. **Shell + header + player width** — done, `3edc358` (2026-08-30): layout bands, `surfaces/header.rs` + `header_logic.rs`, the header as the titlebar drag area, gear + update hint as kit `IconButton`s, player full width with the transport centered on the window. Settings render in the body band with the player still mounted; the header search field is shown but inert on Settings pages (as designed). Traffic lights: macOS only hit-tests the standard window buttons inside the native title-bar band (32 px on macOS 26; the buttons are 14 px, not the design's 12), and gpui positions them relative to that band, so centering them in a 74 px header put them outside it and made them unclickable. `main.rs` installs an empty unified `NSToolbar` on the window, which makes the band 66 px while every click in its empty area still reaches gpui; the toolbar is hidden while fullscreen (a window-bounds observer in `Shell::new`) because macOS would otherwise paint it over the header. `traffic_light_position` is `(24, 30)`.
2. **Sidebar island + Settings restructure**: island styling and spacing, active-row style, remove the OUTPUT group; Settings page with its island sidebar and sections; Devices page becomes Settings ▸ Output; routes and deep links updated, including the persisted `Devices` session route (feature 21), which restores to Settings ▸ Output. Also drops the dead `icons/circle-arrow-down.svg` asset left by phase 1.

## Verification

- `make verify` green after each phase; test count does not go down.
- `grep -rn "Destination::Devices" crates/pulse-app/src` is empty after phase 2; the Settings section enum lists General, Output, Update, About.
- Manual against the design frames: header brand/search/gear positions and the 74 px height; island inset 12 px on three sides with the page flush; transport cluster centered on the window (not the column); Settings sections; Devices content reachable only via Settings ▸ Output; update hint appears in the header with `PULSE_DEV_UPDATE_AVAILABLE`; `Cmd-,`, the app menu, and the gear all open Settings; traffic lights draggable in the header.
