# Settings Surface

> Feature 06 · P1. The sidebar's Settings entry has never had a destination. Designed now, because feature 05's update controls need a home and existing preferences need somewhere to live.

## Motivation

The sidebar footer has carried a Settings item since the app shell landed in stage 7.5 with nothing behind it — with the queue button now wired, it is the last permanently inert control in the app apart from the deliberately-inert Shuffle/Repeat. Preferences that already exist are invisible or reachable only sideways: the output device is buried in a playback-bar popover, storage roots live inside the Library, and the app's own version appears nowhere at all. Feature 05 adds update controls that have no surface to land on. One page fixes all of it.

## Design source

`design/pulse-desktop.pen`, read via the `pencil` MCP only. Three screens, one per section:

| Screen | Node |
|---|---|
| `Settings / General` | `C5mRG` |
| `Settings / Update` | `D9PDB` |
| `Settings / About` | `Tnibv` |

Feature 05's status-line and button states are specified separately in **`Spec — Update States`** (`ADNMS`).

Rows are instances of the existing `Settings Row` component (`pF8bk`) with its trailing chevron replaced per row (value + chevron, toggle, or external-link glyph). Two new reusable components on the Components Board: **`Toggle / On`** (`SK7jO`) and **`Toggle / Off`** (`jkTS9`) — Pulse had no toggle before this.

## Design decisions

- **Settings is its own page, and its sidebar is the section navigation** — the pattern from `runner/design/runner-setting.pen`. The library sidebar is replaced, not reused: a "Back to library" row at the top, then a `SETTINGS` group listing General, Update, and About, using the library sidebar's exact nav-item styling (`accent-soft` fill, accent icon, `text-primary` label when selected). Navigating sections swaps the content pane; each section is a full page with its own title.
- **Settings takes the whole window — the playback row is not shown.** Playback continues, but the transport is not reachable until "Back to library". The tradeoff was considered and accepted: settings is a destination you enter and leave in one click, and a persistent player bar would make it read as a library route rather than a mode of its own. The cost is that changing the output device mid-playback means leaving settings to pause.
- **Content is grouped under mono uppercase labels** (`PLAYBACK`, `VERSION`, `PREFERENCES`, `APPLICATION`, `LINKS`) matching the sidebar's own group headers, with each group's rows in a bordered card.
- **The content column is fixed at 820px** rather than filling the pane — settings rows become unreadable when the title and its control drift to opposite ends of a wide window.

## Scope

- **Entry points.** The sidebar footer Settings item opens `Settings / General`; `Cmd+,` does the same through the existing macOS menu. "Back to library" returns to the previous library route.
- **General** — `PLAYBACK` group: *Default output device* (current device name plus chevron, opening the existing output popover) and *Exclusive mode* (toggle; see Open Questions).
- **Update** — `VERSION` group: hero row with app mark, name, version chip, status line, and action button. `PREFERENCES` group: *Check for updates on launch* toggle. Presentation is designed here; behavior belongs to [feature 05](05-update-check.md).
- **About** — `APPLICATION` group: identity row with mark, name, version chip, and tagline. `LINKS` group: *GitHub*, *License* (MIT), and *Acknowledgements*, each opening the browser via `NSWorkspace`.
- **Toggle in `components.rs`**, matching the two design components, since no toggle control exists in code yet.

## Non-Goals

- A settings search field — runner has one because it has eight sections; three do not need it. The sidebar has room for it when the count grows.
- Appearance/theme settings; Pulse has exactly one theme and no plan for a second.
- Per-device volume memory, playback-behavior preferences, or anything not already backed by real state.
- A storage row. Library ▸ Storage is already a full surface reached from the sidebar's Manage group; a settings row that only routes there is duplicate navigation, not a setting.
- A general preferences container format. The existing one-file-per-preference convention in the config directory covers the current rows; revisit when a row needs something it cannot express.

## Resolved: exclusive mode semantics

Decided 2026-08-11 (Jason): **one global preference, on by default.** On keeps today's behavior — the engine requests hog mode. Off makes it skip the hog request entirely and play in shared mode. No new failure states are introduced: stage 13's device-busy path already handles contention, and turning the toggle off simply stops Pulse from asking. Per-device memory (exclusive on the DAC, shared on AirPods) is the natural follow-up and is explicitly out of scope here; it needs a per-device preference store rather than a single flag.

## Implementation Phases

1. The settings route and its sidebar: section nav, selection state, "Back to library", `Cmd+,` menu wiring, playback row still docked. Empty content panes prove the plumbing.
2. `Toggle` in `components.rs`, plus the group/card/row primitives, matching the design's spacing exactly.
3. General and About against real state; link rows open the browser.
4. The Update page's presentation, handed to feature 05 for behavior.

## Verification

- `make verify` green, with view-model tests for row state (device name resolution), section selection, and toggle rendering.
- Manual (Jason): the sidebar entry and `Cmd+,` both open Settings; each section highlights correctly and swaps the pane; "Back to library" returns where you came from; playback keeps running while Settings is open and the transport returns intact on exit; the output row reflects the device chosen in the playback popover; every About link opens the right page; no page needs scrolling at 900px.
- Visual comparison against the three `.pen` screens.
