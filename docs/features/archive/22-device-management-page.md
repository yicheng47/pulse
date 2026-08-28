# Device Management Page

> Feature 22 · P1 · GitHub issue [#58](https://github.com/yicheng47/pulse-src/issues/58). The sidebar already ships an OUTPUT → Devices destination, and it still renders the legacy file-drop surface. That page becomes the device manager: every known device and its stored state.

## Motivation

The library sidebar has carried an OUTPUT → Devices destination since the shell landed, and clicking it still renders the legacy drop-a-file playback surface (`shell.rs` `render_body`) — a shipping nav entry backed by a leftover page, the same class of gap that made the Settings surface (feature 06) a priority. Meanwhile feature 08 gave every device stored identity — an exclusive-mode override keyed by UID, a probe-classified default, a saved-output marker — with no surface beyond the output popover, which shows connected devices and the control for the selected one. There is no way to see what Pulse remembers, edit the override of a device that isn't plugged in, or forget a device that's gone for good. The dead destination and the invisible store solve each other.

This deliberately refines feature 08's "no Settings mirror" non-goal: the popover remains the only *quick control* for the active device; the page manages the *stored table*, which the popover structurally cannot show.

## Scope

- **The existing sidebar OUTPUT → Devices destination** replaces its legacy drop-surface body with the device manager. Only the body retires: window-level drag-and-drop playback (`shell.rs` `ExternalPaths` → `handle_drop`) is untouched. No new nav entry anywhere.
- **Device table**: the union of currently connected outputs and every device with stored state, as flat rows with inline controls in two groups — **CONNECTED** (active device first, then alphabetical) and **NOT CONNECTED** (alphabetical; each row carries its last-seen time and the Forget affordance). The grouping mirrors the memory model — present devices vs remembered ones — and the page subtitle states that settings stick to each device regardless of connection, so a come-and-go device moving between groups reads as status, not data loss. Per row: name, class chip from the capability probe (DAC / Bluetooth-class by format flags), capability ceiling ("Up to 24-bit / 192 kHz"), and markers for the active output and the saved default. The saved default is editable here: every connected row that is not the default carries a **Set as default** button that rewrites the saved-output marker (the device Pulse opens with on launch) without switching what is playing; the default row shows the Default pill in its place. Choosing an output in the popover keeps updating the default exactly as it does today, so the popover and the page never disagree about which device is the default.
- **Per-row exclusive-mode control** with the popover's exact semantics (Auto tag, pinned override, Reset to Auto) — editable for offline devices too; edits write the stored override.
- **Forget device**: remove a device's stored state entirely; it reappears with probe defaults if it ever reconnects. Confirm before forgetting, reusing the app's existing destructive-confirm dialog pattern (as designed for Delete Album) — no new dialog design. Shown only on rows not currently connected — a connected device would re-register instantly, and Reset to Auto already covers clearing its override. Forgetting the saved-default device also clears the saved-output marker, so the next launch resolves the system default instead of a dangling UID.
- **Layout and scrolling**: the page column is centered in the body with equal left and right padding at every window width — never left-aligned against the sidebar. The body scrolls vertically as one column and shows a scrollbar whenever the table overflows the viewport, using the same scrollbar treatment as the library pages; a hidden overflow is a bug.
- **Empty state**: with nothing remembered offline, the NOT CONNECTED group is absent entirely — never an empty group header. A fresh install simply shows connected devices.

## Non-Goals

- Switching the *active* playback output from this page — switching stays in the popover, one click from the transport; two switching surfaces would drift. (Editing the saved default is in scope; it changes what Pulse opens with, not what is playing.)
- Per-device volume, EQ, DSP, or sample-rate policy (unchanged from feature 08).
- Input devices, AirPlay/network device management, or device renaming.
- Per-device diagnostics or dropout stats — feature 17 surfaces playback health elsewhere.

## Design Source

`design/pulse-desktop.pen`, read via the `pencil` MCP only. Applied 2026-08-17: screen `Library / Devices` (`XmoeK`) — full library chrome with Devices lit in the sidebar's OUTPUT group. A 720-wide page column, centered in the body with equal side padding (corrected 2026-08-28 — the first draft left-aligned it against the sidebar), carries the title, the "settings stick to each device" subtitle, then the CONNECTED group (active row with accent stroke and Active pill: mini-i on Auto/exclusive; AirPods Pro on Auto/shared) and the NOT CONNECTED group (DELL on Auto, Scarlett pinned with Reset to Auto; last-seen lines and Forget buttons). Connected non-default rows carry a Set as default button using the Forget button's anatomy (added 2026-08-28). Cards reuse the output popover's card anatomy — divider, Mode Row, AUTO tag, and the shared Toggle components.

## Implementation Phases

1. Preference store: enumerate stored device entries (UID, override, last-known name/capabilities/seen-at — persist name, ceiling, and a last-seen timestamp at every sighting so offline rows render with "last seen"), plus forget.
2. Page implementation at the Devices destination per the Design Source, retiring the legacy drop-surface body; reuse the popover's mode-control component and the Delete Album confirm pattern for Forget.
3. Tests and manual validation.

## Verification

- Unit tests: the table merges connected and stored-only devices without duplicates; a device moving between groups on connect/disconnect keeps its stored override untouched; within each group ordering is deterministic (active first in CONNECTED, alphabetical otherwise); an override edit on an offline device persists and applies when it reconnects; forget removes the row and a reconnect re-probes defaults; connected rows never offer forget; forgetting the saved-default device clears the saved-output marker; Set as default on a connected non-default row rewrites the saved-output marker and moves the Default pill without changing the active output, and the default row never offers it.
- `make verify` is green.
- Manual: with the Matrix DAC unplugged, its row still shows name, ceiling, and pinned state, and its override is editable; forgetting a stale Bluetooth device removes it; plugging the DAC back applies the edited override on first open; the popover and page never disagree about the same device; with more devices than fit the window, a scrollbar is visible and the column stays centered as the window is resized.
