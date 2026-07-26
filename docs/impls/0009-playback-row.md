# 0009 — Playback row and controller wiring

> Remaining half of stage 7. The app shell decisions are settled and archived ([0007](archive/0007-gpui-native-ui-pivot.md) pivot, [0008](archive/0008-gpui-ce-dependency.md) GPUI-CE); the enduring rules live in [`arch/tech-stack.md`](../arch/tech-stack.md). This note covers only what is still unbuilt.

## Already landed

`crates/pulse-app` exists as a GPUI-CE binary: window, theme-as-data generated from the Pencil tokens (`theme.rs`), and an embedded font/icon asset layer behind a hand-rolled `AssetSource` (`assets.rs`). The Tauri/React scaffold is gone. `pulse-engine` ships `PlaybackController` with play/pause/resume/seek/stop plus events, proven by 22 tests and the `pulse-cli` smoke commands.

## Goal

Build the playback row from the Pencil design and drive it with the real engine. Two slices, in this order, because the first answers a risk and the second is plumbing.

**Slice 1 — static surface.** The row rendered from hardcoded content, no engine. This is the visual spike: it settles whether the cyberpunk design language survives as GPUI paint code instead of CSS, which stages 11–13 all assume.

**Slice 2 — controller wiring.** App-owned `PlaybackController`, in-process event subscription, transport driven from the row.

## Design source

Component `qKkw7` ("Playback Bar") in `design/pulse-desktop.pen`. Read it with the `pencil` MCP rather than trusting this summary — it is the source, this is a convenience transcription.

Root: 92px tall, `bg-surface`, padding 12/20, gap 22, items centered, 1px `border` on the top edge only, full width.

| Zone | Width | Contents |
|---|---|---|
| Now Playing | 320 | 60×60 cover (`radius-sm`, 1px inner `border-strong`, clipped, image fill); title in Rajdhani 15/700 `text-primary`; `artist - album` in Inter 12 `text-secondary`; 4px gap |
| Transport Center | fills | Controls row (gap 12, centered): shuffle, skip-back, 28×28 `accent` play button (`radius-md`) with a 16px `bg-inset` glyph, skip-forward, repeat-2 — outer icons 17px `text-secondary`. Progress row (gap 12): elapsed in Geist Mono 11 `text-muted`, 4px `bg-inset` track (radius 2) with an `accent` fill, duration same as elapsed |
| Output Status | 300 | Right-aligned, gap 14. Format block (132 wide, gap 3): quality in Geist Mono 12/700 `quality` lime, device line in Inter 12 `text-secondary`. Speaker icon 17px. Queue button 38×34 with a `list-music` icon and a 16px `accent` badge offset to the top-right, 2px `bg-surface` outer stroke, count in Geist Mono 10/700 `bg-inset` |

Icons are already vendored in `crates/pulse-app/assets/icons/`, including `pause` for the toggle the design does not show.

## Slice 2 details

`crates/pulse-app` gains a `pulse-engine` dependency — one direction only. The engine must not learn about GPUI; that boundary is in `AGENTS.md` and `arch/pulse-engine.md`.

The app owns one `PlaybackController` and derives all row state from `PlaybackEvent`: `NowPlaying` fills the title/artist/format text, `Position` drives the progress fill and elapsed/duration labels, `StateChanged` selects the play vs pause glyph, `Error` surfaces a visible failure rather than a silent dead button. Row interactions send `PlaybackCommand` — play/pause toggle, seek from the progress bar.

The open design question is bridging `controller.subscribe()`, a blocking channel receiver, into GPUI's runtime without stalling the main thread. Expected shape: a task on the background executor draining the receiver and updating the entity through an async app handle, so events arrive as normal entity updates. Settle this in slice 2, not slice 1.

## Verification

- `make verify` green: check, tests, clippy under `-D warnings`, fmt.
- `make run` opens the window and the row visually matches `qKkw7`. Compare against a Pencil screenshot of the component rather than judging from memory.
- Slice 2: play a real FLAC from the app, then pause, resume, and seek using the row's own controls; confirm the format and device text reflect what the engine actually opened.
- **Needs a manual pass on the Matrix Mini-i Pro 4** — clean audible playback, native-rate switching, pause/resume not restarting from zero, hog mode released on stop. No agent can sign this off. Stage 6's hardware smoke is also still outstanding and covers the same ground from the CLI side.

## Risks

- The design language in paint code is the whole point of slice 1. If gradients, glow, or the badge offsets fight the framework, we learn it here rather than at stage 11. Note that the row as designed is flat — solid `bg-surface`, no `background_blur` — so `backdrop_filter` is not exercised yet despite being a stated reason for GPUI-CE.
- Inter and Geist Mono are embedded as variable fonts. If weights render flat or wrong, swap in static cuts; Rajdhani is already static Medium + Bold.
- Pencil's `fill_container`/`fit_content` sizing does not map one-to-one onto GPUI flex. Expect to translate intent, not properties.
- Album art is one 60px image here. Grid-scale decode and caching stays a stage 11 question.
- Progress-bar seek needs drag handling, not just a click target. Keep it a click-to-seek in the first pass if drag proves fiddly, and say so rather than shipping a bar that looks draggable and is not.

## Non-goals

- No sidebar, albums, tracks, playlists, or storage surfaces — those are stage 11.
- No queue panel or next/previous behavior; the badge is display-only until stage 12.
- No shuffle or repeat behavior — the icons render, they do not wire.
- No output-device switching from the row; that is stage 8.
- No visualizers, lyrics, or level meters.
