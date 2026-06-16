# Playback Controller

> Sixth product stage: wire Pulse's real playback behavior inside `pulse-engine`, then expose it through thin CLI and Tauri adapters.

## Context

Stages 1 through 4 proved the low-level engine path through `pulse-cli`: file probing, device selection, hog mode, native sample-rate switching, AUHAL playback, and UID-backed CLI defaults.

Stage 5 defined the desktop product direction in Pencil. The design now assumes normal player behavior: play, pause, resume, seek, stop, now-playing state, progress, output device status, queue/playlist direction, and UI feedback.

The current code does not yet have that product playback layer. `pulse-cli play` is a blocking smoke path that decodes one file, feeds the low-level engine, waits until accepted frames are pulled by AUHAL, then stops. That is good validation code, but it is not a reusable playback state machine.

## Goal

Add a UI-agnostic playback controller inside `pulse-engine`.

The controller should own playback behavior and expose a command/event API that both `pulse-cli` and the Tauri backend can drive.

Minimum user-visible behavior:

- Play one local file.
- Pause and resume without restarting from zero.
- Seek within the current file.
- Stop and release playback resources.
- Emit now-playing, state, position, ended, and error events.
- Preserve the current AUHAL/native-rate/hog-mode playback path.

## Architecture

Do not create a separate OS process yet. The playback "server" is a long-lived Rust worker inside the calling process.

```text
React UI / pulse-cli
  -> Tauri command or CLI command
    -> PlaybackController command channel
      -> controller worker thread
        -> decode worker / low-level Engine
          -> AUHAL callback
            -> Core Audio device
```

`pulse-engine` owns the controller. `src-tauri` and `pulse-cli` are adapters.

## Boundary

`pulse-engine` may know about:

- File paths.
- Playable source IDs supplied by the caller.
- Device IDs and device UID/name.
- Playback state.
- Queue/current item.
- Source duration and current position.
- PCM format and engine diagnostics.

`pulse-engine` must not know about:

- Tauri command names.
- React state shape.
- SQLite table layout.
- Album-grid UI.
- Artwork display.
- Sidebar navigation.
- Product design node IDs.

## Proposed Modules

```text
crates/pulse-engine/src/
  controller.rs
  command.rs
  event.rs
  state.rs
  queue.rs
  source.rs
```

The current low-level `Engine` can remain public for validation, but it should be treated as the backend primitive used by `PlaybackController`.

## Command Model

First slice:

```rust
pub enum PlaybackCommand {
    PlayFile { path: PathBuf },
    Pause,
    Resume,
    Seek { position_ms: u64 },
    Stop,
    SetOutputDevice { device_id: device::DeviceId },
}
```

Later queue slice:

```rust
pub enum QueueCommand {
    SetQueue { items: Vec<PlayableSource>, start_index: usize },
    PlayIndex { index: usize },
    Next,
    Previous,
    Clear,
}
```

Do not put queue behavior into the first slice unless single-file pause/resume/seek is already stable.

## Event Model

First slice:

```rust
pub enum PlaybackEvent {
    StateChanged(PlaybackState),
    NowPlaying { source: PlayableSource, format: PcmFormat },
    Position { position_ms: u64, duration_ms: Option<u64> },
    Ended,
    Error { message: String },
}
```

Events should be facts, not UI instructions. Tauri can translate them into window events; CLI can print or wait for them.

## State Model

Minimum states:

- `Idle`
- `Loading`
- `Playing`
- `Paused`
- `Stopping`
- `Ended`
- `Error`

The state model must distinguish paused from stopped. Paused has a resumable logical source position; stopped does not promise resumability.

## Pause And Resume

The conservative implementation is acceptable for v1:

1. On pause, stop AUHAL or stop feeding, record the logical source frame/time, and emit `Paused`.
2. On resume, rebuild the low-level playback path if needed, seek decode to the recorded position, and continue feeding.
3. Do not try to preserve partially buffered `rtrb` contents as product state.

This is simpler than keeping the AudioUnit alive while paused. If device stability later argues for a hot AUHAL pause mode, we can add that behind the same controller API.

## Seek

Seek should be implemented as a controlled restart at a target source frame/time:

1. Stop current feed path.
2. Clear/reset the low-level engine ring.
3. Seek the decoder to the requested timestamp.
4. Resume feeding from that point.
5. Emit an immediate position event.

For formats where Symphonia seek is imprecise, prefer nearest valid seek point plus correct state reporting over fake precision.

## Tauri Wiring

Add app state that owns one `PlaybackController`.

Initial commands:

```text
play_file(path)
pause()
resume()
seek(position_ms)
stop()
playback_state()
```

Tauri should forward controller events to the frontend. Tauri should not implement queue or playback timing logic itself.

## CLI Wiring

Keep `pulse-cli` as a harness.

Existing commands should remain:

```text
pulse-cli devices
pulse-cli probe <file>
pulse-cli validate-format <file>
pulse-cli play <file>
```

Add focused smoke commands only if useful:

```text
pulse-cli smoke-pause <file>
pulse-cli smoke-seek <file> --to 90s
```

The CLI should call `PlaybackController`, not duplicate controller behavior.

## Implementation Steps

1. Move the current low-level engine shape out of `lib.rs` if needed so `lib.rs` can export both low-level primitives and controller types cleanly.
2. Add `PlayableSource`, `PlaybackState`, `PlaybackCommand`, and `PlaybackEvent`.
3. Add `PlaybackController::spawn` with a command sender and event receiver/subscription surface.
4. Implement `PlayFile` using the existing `decode -> Engine::open -> set_format -> play -> feed` path.
5. Add position event emission from low-level `Engine::position`.
6. Implement `Stop` and verify device release.
7. Implement conservative `Pause` / `Resume` with recorded logical position.
8. Implement `Seek`.
9. Rewire `pulse-cli play` to use the controller while preserving current behavior.
10. Add Tauri commands and frontend-facing events, but keep the UI minimal until the app shell implementation stage.

## Verification

Automated:

- Unit tests for state transitions.
- Unit tests for command ordering and illegal transitions.
- Fake-backend tests for pause/resume/seek without Core Audio.
- Existing PCM packer tests still pass.
- Existing decode tests still pass.

CLI smoke:

- `pulse-cli play <file>` still plays cleanly.
- `pulse-cli smoke-pause <file>` pauses and resumes audibly from the same region.
- `pulse-cli smoke-seek <file> --to 60s` starts near the requested timestamp.
- `stop` releases hog mode.

Hardware smoke:

- 16/44.1 FLAC plays through Matrix DAC at 44.1 kHz.
- 24/96 FLAC plays through Matrix DAC at 96 kHz.
- Pause/resume does not restart from zero.
- Seek works on at least FLAC and ALAC.
- Errors are emitted when the configured output device is unavailable.

## Non-Goals

- No library scanner.
- No SQLite queue persistence.
- No album/playlist UI implementation.
- No metadata enrichment.
- No DSP, EQ, normalization, crossfade, or replay gain.
- No DSD, streaming, video playback, video library support, FFmpeg, libmpv, or GPL dependencies.
- No hard bit-perfect claim for the AUHAL path.
