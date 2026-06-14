# CLI Config

> Fourth product stage: make `pulse-cli` remember local harness defaults so playback and validation do not require repeating device IDs.

## Context

Stage 3 proved clean AUHAL playback on the Matrix DAC. The next problem is ergonomics and repeatability: Core Audio device IDs are noisy to type and can change across reconnects, but the CLI still needs to be deterministic enough for human smoke tests and future agent/MCP workflows.

The first Stage 4 slice adds a small user config file for the CLI. It is intentionally separate from the future desktop app settings store.

## Goal

Make this flow work:

```sh
pulse-cli devices
pulse-cli config set-default-device <matrix-device-id>
pulse-cli play "/Users/jason/Downloads/03. Ex-Factor.flac"
```

Expected result: `play` uses the configured Matrix output device when `--device` is omitted.

## Config Location

The CLI config lives in the OS user config directory:

```text
<config-dir>/pulse/cli.toml
```

On macOS, `dirs::config_dir()` maps this under the user's Library/Application Support area. The file is not stored in the repo and should not be committed.

## Data Model

The first config schema stores one default output device:

```toml
[default_output]
uid = "AppleUSBAudioEngine:..."
name = "mini-i Series "
```

Both fields matter. Core Audio `AudioDeviceID` values are runtime object IDs, not durable config keys. They can change across reconnects, driver restarts, reboots, and device graph changes. The Core Audio device UID is the durable identity we should persist; the name is only for human display and a conservative last-resort recovery path.

## Resolution Rules

When a command accepts `--device`, explicit CLI input wins.

When `--device` is omitted:

1. If `default_output` exists and the stored UID is currently available, use that device's current runtime ID.
2. If the stored UID is missing but exactly one current device has the stored name, use the current ID for that name.
3. If the stored name is missing, fail instead of silently falling back to system output.
4. If the stored name is ambiguous, fail and ask for an explicit `--device`.
5. If no CLI default is configured, use the system default output device.

The "no silent fallback" rule prevents accidentally playing through speakers when the expected DAC is disconnected.

## Command Surface

```sh
pulse-cli config show
pulse-cli config set-default-device <device-id>
pulse-cli config clear-default-device
```

`pulse-cli devices` marks the configured Pulse default with `>` and the system default with `*` when no configured default is active for that row.

## Touched Files

- `crates/pulse-cli/src/config.rs`
- `crates/pulse-cli/src/main.rs`
- `crates/pulse-cli/Cargo.toml`
- `crates/pulse-engine/src/device.rs`
- `Cargo.toml`

## Verification

- `pulse-cli config show` works with no config file.
- `pulse-cli config set-default-device <id>` writes `cli.toml`.
- `pulse-cli devices` marks the configured default.
- `pulse-cli play <file>` uses the configured default when `--device` is omitted.
- `pulse-cli validate-format <file>` uses the configured default when `--device` is omitted.
- If the configured device is unavailable, playback fails clearly instead of using system output.
