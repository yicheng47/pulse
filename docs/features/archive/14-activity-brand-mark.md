# Activity Brand Mark

> Feature 14 · P2. Replace the hand-drawn waveform brand mark with the lucide `activity` glyph (accent-tinted) everywhere the Pulse identity appears — picked by Jason from the design canvas ("Brand Well", `mBb23`): a cleaner, more literal pulse line that stays crisp at every size.

## Motivation

The current mark is a custom-drawn waveform path that reads muddy at small sizes and was never revisited after the v0 design sprint. The lucide `activity` glyph is the canonical "pulse" symbol — simpler, instantly legible at 17px, and consistent with the icon language the entire app already uses (every other glyph in Pulse is lucide). An identity should be one mark everywhere: sidebar, settings tiles, and the dock icon must move together or not at all.

## Design source

`design/pulse-desktop.pen`, read via the `pencil` MCP only: **`Brand Well` (`mBb23`)** — lucide `activity`, `$accent` fill, on a `$bg-muted` well. The well itself is presentation for the picker; what ships is the glyph. Phase 1 updates the `.pen` surfaces (all sidebar brands, the About/Update icon tiles, and the `App Icon` screen `v06cM3`) before any code moves — this design pass has not happened yet.

## Scope

- **Sidebar brand glyph** (all sidebars): the flat accent waveform path is replaced by lucide `activity` at the same 24px size, same placement, same wordmark.
- **About/Update settings tiles**: the waveform inside the icon tiles becomes `activity`; the tile chrome (gradient, border, radius) stays — there the tile is the app icon rendered as content.
- **App/dock icon**: the `App Icon` design screen (`v06cM3`) rebuilt around the `activity` glyph on the existing gradient ground, respecting the Apple icon-grid margin already established for `app-icon/dock.png`; regenerate `dock.png` and the bundle `.icns` through the existing `make bundle` path. Bundle identifier and signing are untouched — this is artwork only.
- **In-app assets**: whatever embedded asset renders the brand glyph in `shell.rs` / the settings pages (SVG asset or drawn path) is swapped to the lucide `activity` source; the dev `setApplicationIconImage` path picks up the regenerated art.

## Non-Goals

- Any wordmark, name, or color change — `$accent` on the existing grounds, "Pulse" in Rajdhani, unchanged.
- Bundle identifier, signing, or TCC identity changes (artwork only).
- Redesigning the sidebar header layout (feature 12 owns that; this rides on top of whichever lands first).

## Implementation Phases

1. **Design pass** in the `.pen`: swap the glyph across all sidebar brands and the About/Update tiles; rebuild the `App Icon` screen around `activity`; verify instances clean.
2. **Assets + code**: regenerate `app-icon/dock.png` and the icns via `make bundle`; swap the embedded brand asset; confirm the About/Update tiles and dev dock icon render the new mark.

## Verification

- `make verify` green; `make bundle` produces a valid icns with the new art (plist and icon-representation checks already exist in the bundle script).
- Manual (Jason): sidebar glyph at 24px, About/Update tiles, and the dock icon all show the same mark; dock icon respects the Apple grid margin (compare against Runner's dock icon); no signing or launch regressions in the bundled app.
