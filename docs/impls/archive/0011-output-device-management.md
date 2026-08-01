# 0011 — Output device management

> Stage 8. Depends on [`0010-app-shell.md`](0010-app-shell.md) for the chrome. Makes the playback row's device line and speaker icon real.

## Goal

Let the user see the available Core Audio output devices, pick one, and have that choice persist across launches by device UID. The playback row shows the active device and its format for real instead of placeholder text.

## Design source

Read via the `pencil` MCP; `.pen` files are encrypted, never Read or Grep them.

- `vH78z` — Output Device Popover. **This is the surface this stage builds.**
- `pv9Av` — Device Row (520 wide, richer: capability meta plus a status pill) and `Y8Ojv` — Status Pill. These belong to the unbuilt Devices page, not the popover. Leave them for later.

**Design gap: there is no Devices page.** The sidebar has an OUTPUT → Devices item (`speaker` icon), but no screen frame for it exists in the file — only the popover. So this stage delivers device management through the popover, reached from the playback row's speaker icon. The Devices nav destination keeps the placeholder body 0010 gives it until the page is designed. Do not invent that page.

### Popover spec (`vH78z`)

360 wide, `bg-surface`, 1px `border`, radius-lg, padding 14, vertical, gap 11.

- Header, `space_between`: title "Choose audio output" Rajdhani 17/700 `text-primary`; `settings` icon 16px `text-secondary`.
- Current device block: `bg-inset`, radius-md, **1px `accent` border**, padding 12, gap 12, items centered — `speaker` icon 22px `accent`; a vertical copy block (gap 3) with name Rajdhani 17/700 `text-primary`, driver line "CoreAudio · Exclusive mode" Inter 12 `text-secondary`, and a capability line "Up to 24-bit / 192 kHz" Geist Mono 11/700 in `quality` lime; then a `check` icon 18px `accent`.
- Section label "DIRECT DEVICES", Geist Mono 10/700 `text-muted`, letterSpacing 0.8.
- Device list, vertical, gap 2. Each row radius-md, padding [9,10], gap 12, items centered, name Rajdhani 14/600.
  - Selected: fill `accent-soft`, 1px `accent` border, icon 18px `accent`, label `text-primary`, trailing `check` icon 16px `accent`.
  - Unselected: transparent fill, no border, icon 18px `text-muted`, label `text-secondary`, no check.
- Section label "NETWORK DEVICES", then "No network devices found" Inter 12 `text-muted`. Pulse has no network device support, so this section is honest as a permanent empty state.
- Footer, `space_between`, padding [12,2,2,2], 1px `border` on top: "Can't find your device?" Rajdhani 14/500 `text-secondary` and a `log-in` icon 16px `text-muted`. Render it; there is no designed destination, so do not wire it.

## Engine work required

The popover's capability line needs data `pulse-engine` does not currently expose. Today's public surface is:

- `device::list_output_devices() -> Result<Vec<Device>, EngineError>`, `Device { id, uid, name }`.
- `device::default_output_device() -> Result<Device, EngineError>`.
- `device::validate_output_format(..) -> FormatValidation { device, requested, nominal_sample_rate, physical_format }` — validates one *requested* format; it does not report a device's range.

`hal.rs` is a private module whose helpers are all `pub(crate)`, so supported-format enumeration and hog-mode state are unreachable from `pulse-app`.

Add a **narrow** public capability query to `pulse-engine` — enough to answer "up to N-bit / M kHz" for a device, derived from its available physical formats. Do not make `hal` public wholesale; the engine's job is to hand the UI a plain data struct, and the module boundary in [`arch/pulse-engine.md`](../arch/pulse-engine.md) still applies. Add unit coverage where the logic is pure (picking the max from a format list), consistent with how `hal.rs` already tests format matching.

**Be honest in that line.** Report only what the device actually advertises. The driver line says "Exclusive mode" — do not claim exclusivity for a device where hog mode is unavailable; if hog-mode state is not cheaply knowable, word the line so it describes Pulse's playback path rather than asserting a per-device guarantee. Do not make bit-perfect claims for the AUHAL path.

## Persistence

Persist the selected device by **Core Audio UID**, never by `AudioDeviceID` — IDs are transient across reboots and replugs. That rule is in the roadmap's stage rules.

`pulse-cli` already persists a device preference through its own `config` module. The app needs its own store; do not share the CLI's file, since the CLI is a deterministic harness and its config is part of that contract.

Keep this minimal — a single persisted preference, not a settings framework. The roadmap says not to harden app-shell settings backends before the settings UX and storage model are designed, and no Settings surface exists yet. One small app-owned config file with the selected output UID is the whole scope.

On launch: resolve the persisted UID against the current device list. If it is gone, fall back to the system default and say so in the UI rather than silently switching.

## Scope

In:

- Popover opens from the playback row's speaker icon; lists real devices; shows which is active.
- Selecting a device sends `PlaybackCommand::SetOutputDevice` and persists the UID. The controller already restarts active playback on device change — there is a test for it.
- The playback row's device line shows the real active device name and the real format from `NowPlaying`.
- Unavailable, hogged, or vanished device surfaces a visible message.

Out:

- The Devices page (undesigned, see above).
- Network/AirPlay devices — no engine support; the empty state is permanent for now.
- A general settings surface, per-device format overrides, or forcing a physical format.
- The header settings icon and the footer help affordance: render, do not wire.

## Verification

- `make verify` green: check, tests, clippy under `-D warnings`, fmt.
- `make run`: popover matches `vH78z`. Compare against a `get_screenshot` and report divergences.
- The device list matches `pulse-cli devices` output on the same machine — that CLI path is already trusted, so it is the cross-check.
- Select a different device, quit, relaunch: the choice survives, and the row reflects it.
- Selecting a device mid-playback restarts cleanly on the new device.
- **Manual hardware pass on the Matrix Mini-i Pro 4, Jason only:** switching to the Matrix plays cleanly at native rate, the capability line matches what the DAC actually supports, hog mode is released when switching away, and unplugging the device produces a clear error rather than silence. Agents cannot verify sound — report this as outstanding, never as done.

## Risks

- Error states for unavailable and hogged devices are not designed (`product/mvp.md` lists them as outstanding). Surface them as plain honest text in the popover and row for now, and flag that the real states still need a design pass rather than inventing a treatment.
- Device hot-plug is not handled anywhere yet. The popover should at least re-list on open; a full `kAudioHardwarePropertyDevices` listener is a larger change and can wait unless it turns out to be trivial.
- The capability query touches HAL property reads on the main thread. Keep it off the realtime path entirely — it is a control-plane call, and the render callback rules in `AGENTS.md` are unchanged.
