# 0009 — Playback row MVP

> Remaining half of stage 7, and the first runnable Pulse app. The app shell decisions are settled and archived ([0007](archive/0007-gpui-native-ui-pivot.md) pivot, [0008](archive/0008-gpui-ce-dependency.md) GPUI-CE); the enduring rules live in [`arch/tech-stack.md`](../arch/tech-stack.md).

## Goal

One window: drag an audio file in, it plays. Play/pause works. The progress bar can be dragged to seek. Nothing else.

This is deliberately narrower than [`product/mvp.md`](../product/mvp.md), which describes the broader v0 with library scanning, SQLite, and browsing. Those stay on the roadmap; this note is the first end-to-end slice that makes Pulse a real app — file in, sound out, transport that responds.

## Progress

Implemented on `feature/stage-7-playback-row`:

- Window-wide single-file drop accepts FLAC, ALAC in M4A, AIFF, and WAV; unsupported files and engine failures produce visible messages.
- The `qKkw7` playback row is rendered with the generated theme tokens and embedded fonts/icons. Shuffle, repeat, previous, next, queue, and output-device controls remain display-only as scoped.
- Play/pause reflects real engine state, including distinct paused and ended states.
- The progress bar has a 16px drag target around its 4px visual track, updates elapsed/duration while playing, and sends one seek when the drag is released.
- macOS CI runs the full Rust `make verify` gate for pull requests and pushes to `main`. Windows and Linux support and CI are intentionally deferred.

Validation completed:

- `make verify` passes: 22 engine tests, 8 app tests, and 2 CLI tests, plus workspace check, clippy with warnings denied, and formatting.
- `make run` opens the app without a panic.
- Jason manually confirmed that dropping an audio file starts playback and that the basic playback experience feels good.
- Working-tree review found no remaining must-fix issues before the CI and progress-note additions.

Known divergences and remaining validation:

- The row uses a neutral embedded placeholder instead of the design's Blonde cover art because cover extraction is out of scope. The queue badge reports the honest single-track count rather than the design's static `7`, and unloaded time/format fields use honest placeholders.
- GPUI matched the layout geometry, icon sizing, badge offsets, 4px track, and static Rajdhani weights without fighting the design. Its offscreen Metal `render_to_image` path omitted all text, including a system-font control, so a complete automated pixel comparison was not possible; the live window renders text normally.
- Detailed hardware validation on the physical Matrix Mini-i Pro 4 remains outstanding: native-rate switching, pause/resume and seeking under real playback, clean output, and hog-mode release still need Jason's manual pass.

## Already landed

`crates/pulse-app` is a GPUI-CE binary with a window, `theme.rs` generated from the Pencil tokens, and embedded fonts/icons behind a hand-rolled `AssetSource` in `assets.rs`. `pulse-engine` ships `PlaybackController` (play/pause/resume/seek/stop plus events), proven by 22 tests and the `pulse-cli` smoke commands.

## Scope

In:

- A drop target covering the window: accept one dropped file, reject unsupported extensions with a visible message.
- The playback row from the Pencil design, rendered for real.
- Play/pause toggle driven by actual `PlaybackState`.
- Drag the progress bar to seek; elapsed/duration labels track playback.
- Errors from the engine surface visibly instead of a dead button.

Out — render if the design shows them, but do not wire:

- Shuffle, repeat, previous, next. Icons only.
- Queue badge behavior (stage 12), output-device switching from the row (stage 8).
- Sidebar, albums, tracks, playlists, storage (stage 11).
- Tag reading, cover-art extraction, visualizers, level meters.

## Design source

Component `qKkw7` ("Playback Bar") in `design/pulse-desktop.pen`. **Read it with the `pencil` MCP** (`get_variables`, then `batch_get` on `qKkw7` with `readDepth: 5`) — the table below is a convenience transcription, the file is the source of truth. Do not open `.pen` files with Read or Grep; they are encrypted.

Root: 92px tall, `bg-surface`, padding 12/20, gap 22, items centered, 1px `border` on the top edge only, full width.

| Zone | Width | Contents |
|---|---|---|
| Now Playing | 320 | 60×60 cover (`radius-sm`, 1px inner `border-strong`, clipped); title in Rajdhani 15/700 `text-primary`; secondary line in Inter 12 `text-secondary`; 4px gap |
| Transport Center | fills | Controls row (gap 12, centered): shuffle, skip-back, 28×28 `accent` play button (`radius-md`) with a 16px `bg-inset` glyph, skip-forward, repeat-2 — outer icons 17px `text-secondary`. Progress row (gap 12): elapsed in Geist Mono 11 `text-muted`, 4px `bg-inset` track (radius 2) with an `accent` fill, duration same style as elapsed |
| Output Status | 300 | Right-aligned, gap 14. Format block (132 wide, gap 3): quality line in Geist Mono 12/700 `quality` lime, device line in Inter 12 `text-secondary`. Speaker icon 17px. Queue button 38×34 with a `list-music` icon and a 16px `accent` badge offset top-right, 2px `bg-surface` outer stroke, count in Geist Mono 10/700 `bg-inset` |

`theme.rs` already holds every token — use it, do not hardcode hex. Icons are vendored in `crates/pulse-app/assets/icons/`, including `pause`, which the design does not show but the toggle needs.

**No tags yet.** The engine reports `PcmFormat { sample_rate, bits_per_sample, channels }`, not artist/album, and `lofty` is a later stage. Derive the title from the file stem and the secondary line from something actually available (parent directory name, or the path). Do not invent "Unknown Artist" placeholders, and do not fake a codec name the engine did not report — the container can come from the file extension.

## Engine wiring

`crates/pulse-app` gains a `pulse-engine` dependency. One direction only: the engine must not learn about GPUI. That boundary is in `AGENTS.md` and [`arch/pulse-engine.md`](../arch/pulse-engine.md).

Relevant API, current as of this note:

- `device::default_output_device() -> Result<Device, EngineError>`; `Device { id, uid, name }`.
- `PlaybackController::spawn(output_device: DeviceId) -> Self`. Hold it — dropping it stops playback.
- `controller.command_sender() -> Sender<PlaybackCommand>`, `controller.subscribe() -> Receiver<PlaybackEvent>`. Both are `std::sync::mpsc`.
- `PlaybackCommand::{PlayFile { path }, Pause, Resume, Seek { position_ms }, Stop, SetOutputDevice { device_id }}`.
- `PlaybackEvent::{StateChanged(PlaybackState), NowPlaying { source, format }, Position { position_ms, duration_ms }, Ended, Error { message }}`.
- `PlaybackState::{Idle, Loading, Playing, Paused, Stopping, Ended, Error}` — note `Paused` and `Ended` are distinct from `Idle`; the play glyph should reflect that.

Derive all row state from events. `NowPlaying` fills the text, `Position` drives the fill and labels, `StateChanged` picks play vs pause, `Error` becomes visible text.

Consider resolving the device and spawning the controller lazily on first play rather than at launch, so opening the window does not reach for the DAC.

## Implementation notes

These were verified against the gpui-ce checkout at the pinned rev. Read gpui-ce for reference, never Zed's GPUI.

**File drop.** `div().on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| ...))`; `ExternalPaths::paths() -> &[PathBuf]`. `drag_over::<ExternalPaths>(|style, ..| ...)` gives hover feedback. Engine formats are FLAC, ALAC, AIFF, WAV — filter on extension and say so when rejecting.

**Event pump.** `Receiver` is blocking, so do not `recv()` on the UI thread. Drain with `try_recv()` from a foreground task:

```rust
cx.spawn(async move |this, cx| {
    loop {
        cx.background_executor().timer(POLL_INTERVAL).await;
        if this.update(cx, |this, cx| this.drain_events(cx)).is_err() {
            break;
        }
    }
})
.detach();
```

`Context::spawn` takes `AsyncFnOnce(WeakEntity<T>, &mut AsyncApp)`. The controller emits `Position` every 100ms, so a ~16ms poll is smooth; only `notify()` when something actually changed.

**Progress bar geometry.** Mouse events carry a window position, not element bounds. Capture the track's bounds during paint with `canvas(prepaint, paint)` — its prepaint closure receives `Bounds<Pixels>` — and stash them where the mouse handler can read them. `Canvas` implements `Styled`, so it can be an absolutely-positioned overlay filling the track. Fractional fill width is `gpui::relative(fraction)`.

**Seek on release, not on move.** Every `Seek` tears down the feed, resets the ring, and re-seeks the decoder. Firing one per mouse-move will stutter badly. Track the scrub position visually during the drag and send a single `Seek` on mouse up. Attach move/up handlers above the track so a drag that leaves the bar still completes.

## Verification

- `make verify` green: check, tests, clippy under `-D warnings`, fmt.
- `make run` opens the window; the row matches `qKkw7`. Compare against a `get_screenshot` of the component rather than judging from memory.
- Drag in a real FLAC: it plays. Pause and resume from the row, and confirm resume does not restart from zero. Drag the progress bar and confirm playback continues from where it was dropped.
- Drop an unsupported file and confirm a visible message rather than silence.
- **Hardware smoke needs a manual pass on the Matrix Mini-i Pro 4** — clean audible playback, native-rate switching, hog mode released on stop. Agents cannot verify sound; report it as outstanding rather than claiming it.

## Risks

- This is the visual spike for the whole GPUI pivot. If the design language fights the framework — badge offsets, the 4px track, type weights — that is the finding, and it matters more than shipping the slice. Say so explicitly.
- The row as designed is flat: solid `bg-surface`, no `background_blur`. `backdrop_filter` is not exercised here despite being a stated GPUI-CE reason.
- Inter and Geist Mono are embedded as variable fonts. If weights render flat or wrong, swapping to static cuts is the fix; Rajdhani is already static Medium + Bold.
- Pencil's `fill_container`/`fit_content` does not map one-to-one onto GPUI flex. Translate intent, not properties.
- If progress drag proves fiddly, ship click-to-seek and say that plainly rather than a bar that looks draggable and is not.
