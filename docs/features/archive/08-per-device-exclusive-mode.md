# Per-Device Exclusive Mode

> Feature 08 · P2 · GitHub issue [#33](https://github.com/yicheng47/pulse-src/issues/33). Exclusive mode is one global on-by-default preference, so every device pays for a choice that only fits DACs; each device should pick its own correct default and remember overrides.

## Motivation

The global toggle forces users who alternate between an exclusive-capable DAC and shared outputs such as AirPods to revisit Settings whenever the output changes. Feature 18 makes this sharper, not softer: shared mode is now the path that makes Bluetooth devices work, but with exclusive on by default, first contact with AirPods still fails with a rate error until the user manually flips the toggle. A device switch should never require a Settings visit.

The information needed to choose correctly already exists. The capability probe behind the output popover's "Up to N kHz" line (`hal.rs` `maximum_physical_format_capabilities`, surfaced via `device::output_device_capabilities`) already classifies every device by its physical format flags: signed-integer formats mean a DAC-class device where hog plus native-rate switching is meaningful; float-only formats (the entire Bluetooth class, per issue #11) mean exclusive mode buys nothing. That classification is the right default, per device.

## Scope

- **Capability-aware default.** A device with no saved preference defaults from the existing capability probe: advertises signed-integer physical formats → exclusive; float-only → shared. No new HAL queries — reuse `output_device_capabilities`.
- **Per-device persistence.** Store explicit overrides keyed by Core Audio device UID; unset means "follow the probe default".
- **The control lives in the Output Device Popover; the global toggle is deleted.** An exclusive-mode control sits next to the selected device's capability line, showing the *effective* mode — labeled Auto while unset, pinned once toggled, with an affordance to return to Auto so the probe default stays reachable after an experiment. The global Exclusive mode row in Settings → General is removed; the PLAYBACK group keeps only *Default output device*, which opens the popover where the control now lives.
- **Runtime fallback with a notice.** When exclusive-mode setup fails at backend open (hog rejected, nominal rate unsupported), retry that open in shared mode and surface a visible notice naming the device — playback continues instead of erroring. Never fall back silently: the notice is what keeps the bit-perfect promise honest on DAC-class devices.
- Changing the selected device during playback reopens the backend with the target device's effective mode while preserving the existing logical-position behavior.
- Migration from the current global marker preference without silently changing behavior on the user's saved device; the global marker is deleted afterward.

## Non-Goals

- Per-device volume, EQ, DSP, sample-rate policy, or output presets.
- Transport-type probing (`kAudioDevicePropertyTransportType`) — the format-flags signal is sufficient; revisit only if a device class misclassifies in practice.
- Changing what hog mode or native-rate switching *do* inside `pulse-engine`; this feature chooses when the existing exclusive path is attempted and how its failure degrades.
- Auto-switching the preference itself based on fallback outcomes — fallback is per-session; the stored preference changes only through the popover control.
- A Settings mirror of the per-device control — one control surface, in the popover.

## Design Source

`design/pulse-desktop.pen`, read via the `pencil` MCP only. Applied 2026-08-17: the Output Device Popover (`vH78z`) now carries the control — a Mode Row inside the Current Device card under a divider, with the "Exclusive mode" label, AUTO tag, and the existing Toggle component. The states board `Spec — Exclusive Mode Control` (`r85ht`) documents the three states: Auto on a DAC (AUTO tag, toggle on), Auto on Bluetooth (AUTO tag, toggle off), and pinned override (tag replaced by an accent "Reset to Auto" link, toggle off).

## Implementation Phases

1. Preference model: UID-keyed tri-state store (unset / exclusive / shared) replacing the global marker, with migration seeding from the existing global value.
2. Default resolution: classify the device via the existing capability probe when the stored value is unset; effective mode = override or probe default.
3. Controller fallback: exclusive open failure retries shared and emits a notice event; app renders the banner.
4. Popover UI: implement the designed Mode Row in the Output Device Popover (see Design Source), delete the global Settings row and its copy; tests.
5. Hardware validation on the Matrix DAC and AirPods Pro.

## Verification

- Preference tests: two UIDs with different overrides, unset resolving through the probe default, migration from both states of the existing marker.
- Controller tests with fake backend: a float-only device with no override opens shared and never touches the HAL rate/format setters; a hog-rejection on an exclusive device retries shared and emits the notice event exactly once; a live device change preserves position and uses the target device's effective mode.
- `make verify` is green.
- Manual, from a fresh preference state: switch to AirPods Pro mid-playback — plays immediately in shared mode, no error, no Settings visit; the Matrix DAC plays exclusive at native rate by default; forcing shared on the Matrix via the popover control persists across relaunches, and returning it to Auto restores the probe default; Settings → General shows no exclusive toggle; hog contention on the Matrix (another app holding it) falls back to shared with the banner instead of stopping playback.
