# Desktop Device Settings

> Fifth product stage: connect the Tauri shell to the proven Core Audio device layer before building library or playback UI.

## Context

Stages 1 through 4 proved the engine through `pulse-cli`: device listing, file probing, format validation, AUHAL playback, and UID-backed CLI defaults. The desktop app still only had an `engine_status` smoke command and a placeholder milestone screen.

Stage 5 starts the app integration layer with the narrowest useful backend bridge: output-device visibility and default-output selection through Tauri commands. The frontend remains the placeholder shell until the settings surface is designed in Pencil.

## Goal

Make this flow available to the Tauri frontend:

```text
Call output_device_settings
Receive current Core Audio output devices
See which device is the macOS system default
Set a Pulse app default output by stable device UID
Clear the Pulse app default back to system default
```

Expected result: the app persists a Pulse-specific output preference without relying on transient Core Audio `AudioDeviceID` values.

## Config Location

The desktop app stores settings separately from `pulse-cli`:

```text
<config-dir>/pulse/app.toml
```

The CLI remains a deterministic harness with its own `<config-dir>/pulse/cli.toml`. Sharing the same config too early would make harness experiments leak into app behavior and make smoke tests less isolated.

## Data Model

The app config stores one default output device:

```toml
[default_output]
uid = "AppleUSBAudioEngine:..."
name = "mini-i Series "
```

The UID is the durable Core Audio identity. The numeric `AudioDeviceID` is still displayed because it is useful for diagnostics, but it is not persisted as the setting key.

## Command Surface

The Tauri backend exposes:

```text
output_device_settings
set_default_output_device(uid)
clear_default_output_device
```

`output_device_settings` returns the config path, device list, macOS system-default marker, Pulse app-default marker, and whether the saved app default is currently available.

## Frontend Boundary

No frontend settings UI lands in this stage.

The next frontend step should start in a Pencil file, then the React implementation should follow that design. Until then, the existing placeholder shell stays unchanged and only the backend command surface is added.

## Touched Files

- `src-tauri/src/settings.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/Cargo.toml`

## Verification

- `cargo check --workspace`
- `pnpm build`
- `cargo test --workspace`
- command smoke: Tauri backend builds with output-device commands registered
