# Device Management Page

> Feature 22 · P1 · GitHub issue [#58](https://github.com/yicheng47/pulse/issues/58). The sidebar already ships an OUTPUT → Devices destination, and it still renders the legacy file-drop surface. That page becomes the device manager: every known device and its stored state.

## Motivation

The library sidebar has carried an OUTPUT → Devices destination since the shell landed, and clicking it still renders the legacy drop-a-file playback surface (`shell.rs` `render_body`) — a shipping nav entry backed by a leftover page, the same class of gap that made the Settings surface (feature 06) a priority. Meanwhile feature 08 gave every device stored identity — an exclusive-mode override keyed by UID, a probe-classified default, a saved-output marker — with no surface beyond the output popover, which shows connected devices and the control for the selected one. There is no way to see what Pulse remembers, edit the override of a device that isn't plugged in, or forget a device that's gone for good. The dead destination and the invisible store solve each other.

This deliberately refines feature 08's "no Settings mirror" non-goal: the popover remains the only *quick control* for the active device; the page manages the *stored table*, which the popover structurally cannot show.

## Scope

- **The existing sidebar OUTPUT → Devices destination** replaces its legacy drop-surface body with the device manager. Only the body retires: window-level drag-and-drop playback (`shell.rs` `ExternalPaths` → `handle_drop`) is untouched. No new nav entry anywhere.
- **Device table**: the union of currently connected outputs and every device with stored state. Per row: name, class chip from the capability probe (DAC / Bluetooth-class by format flags), capability ceiling ("Up to 24-bit / 192 kHz"), connection status, and markers for the active output and the saved default.
- **Per-row exclusive-mode control** with the popover's exact semantics (Auto tag, pinned override, Reset to Auto) — editable for offline devices too; edits write the stored override.
- **Forget device**: remove a non-active device's stored state entirely; it reappears with probe defaults if it ever reconnects. Confirm before forgetting.
- **Empty state** for a fresh install (only connected devices, no stored strays).
- Pencil design first: page layout, row anatomy, chips, empty state.

## Non-Goals

- Selecting the playback output from this page — switching stays in the popover, one click from the transport; two switching surfaces would drift.
- Per-device volume, EQ, DSP, or sample-rate policy (unchanged from feature 08).
- Input devices, AirPlay/network device management, or device renaming.
- Per-device diagnostics or dropout stats — feature 17 surfaces playback health elsewhere.

## Implementation Phases

1. Pencil design: the Devices page in the Settings frame — table anatomy, class/status chips, offline-row treatment, forget confirmation, empty state.
2. Preference store: enumerate stored device entries (UID, override, last-known name/capabilities — persist name and ceiling at first sight so offline rows render), plus forget.
3. Page implementation at the Devices destination, retiring the legacy drop-surface body; reuse the popover's mode-control component.
4. Tests and manual validation.

## Verification

- Unit tests: the table merges connected and stored-only devices without duplicates; an override edit on an offline device persists and applies when it reconnects; forget removes the row and a reconnect re-probes defaults; the active device cannot be forgotten.
- `make verify` is green.
- Manual: with the Matrix DAC unplugged, its row still shows name, ceiling, and pinned state, and its override is editable; forgetting a stale Bluetooth device removes it; plugging the DAC back applies the edited override on first open; the popover and page never disagree about the same device.
