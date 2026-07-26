# Pulse impl log

Running progress record. Newest entries at the bottom. Keep entries short: what happened, what's next, blockers.

Purpose is context preservation across sessions — decisions, gotchas, and dead ends that the code and `git log` do not explain on their own. Stage plans live in the numbered impl notes; stage order lives in [`ROADMAP.md`](ROADMAP.md). This file records what actually happened.

Entries before 2026-07-26 were reconstructed from `git log` and the archived impl notes rather than written live, so they are thinner than later ones.

## 2026-06-07 — Scaffold

- Initial repo: Tauri 2 + React shell alongside a standalone Rust engine crate. MIT, open-source first.

## 2026-06-13 — Stages 1–2: engine validation and HAL format checks

- `pulse-cli` created as the engine harness (#1): file probing, device listing, format inspection.
- HAL format validation (#2): hog mode via `kAudioDevicePropertyHogMode`, nominal sample-rate switching, physical-format probing through `objc2-core-audio` directly.

## 2026-06-14 — Stages 3–5: AUHAL pivot, CLI config, design-first reorder

- **Playback moved from raw HAL IOProc to AUHAL.** Raw IOProc hit a client-format mismatch on the Matrix hardware — the physical stream could be integer while the callback buffer stayed float32, and writing integer bytes into it produced heavy noise. AUHAL makes the contract explicit: Pulse feeds interleaved float32 and Core Audio converts.
- Consequence carried forward: **no hard bit-perfect claims for this path.** The honest claim is native-rate exclusive playback with no Pulse-side DSP. Raw integer HAL is parked as a future experiment.
- Stage 4: UID-backed CLI default output device config, so harness runs are repeatable. Device identity is persisted by UID, never by transient `AudioDeviceID`.
- Stage 5 reordered mid-flight: a backend device-settings stage was replaced by design-first (#5), on the rule that no frontend surface gets built before its Pencil design exists.
- First Pencil mockup: cyberpunk library screen.

## 2026-06-15 — Design: tracks and playlists

- Added Tracks and Playlists library pages to `design/pulse-desktop.pen`.

## 2026-06-16 — Stage 6 scoped

- `0006-playback-controller.md` written: UI-agnostic controller inside `pulse-engine`, command/event API, conservative pause/resume, seek as controlled restart. Queue commands explicitly deferred until single-file transport is stable.

## 2026-07-20 — GPUI pivot (impl 0007)

- **Tauri 2 + React dropped for a native GPUI app.** Trigger: Runner's cancelled GPUI rewrite had de-risked the framework end to end (production rendering, Chinese IME via `EntityInputHandler` with no platform code, hand-rolled `.app` assembly + Developer ID codesign). That rewrite died to the parity treadmill of replacing 28k LOC of shipped UI; Pulse had zero shipped UI, so the killer risk did not exist here.
- Rationale that stuck: Pulse is macOS-only by nature (Core Audio) and already deep in `objc2-*`, so GPUI's macOS-first Metal posture costs nothing, and the UI can observe the engine in-process — no IPC, no serialization, no webview frame pacing.
- Deleted `src/`, `src-tauri/`, and the whole JS toolchain; added `crates/pulse-app`. Workspace lockfile shed ~4,500 lines of webview dependency tree.
- Accepted risk, still open at the time: the cinematic design language means paint code, not stylesheets.

## 2026-07-21 — Stage 6 implemented

- `PlaybackController` landed in `pulse-engine`: `PlayFile`/`Pause`/`Resume`/`Seek`/`Stop`/`SetOutputDevice`, five events, fake-backend tests for pause/resume/seek and device restart, plus `smoke-pause` and `smoke-seek` CLI commands.

## 2026-07-22 — Makefile

- `make build/run/check/test/clippy/fmt/verify`. `make verify` is the gate: check, test, clippy with `-D warnings`, fmt check.

## 2026-07-26 — Controller hardened

- Codex crew review pass on the stage 6 diff. 17 → 22 tests. New coverage: drop-stops-playback with a live sender clone, play-while-playing backend reuse, paused seek not compounding error, output-device failure stopping playback, end-of-track stop-failure ordering, backend stop failure emitting error instead of paused, pause releasing the backend while seek reuses a resumed one.
- **Still outstanding:** hardware smoke on the Matrix Mini-i Pro 4. Agents cannot verify sound; this needs Jason's ears and has been carried forward since.

## 2026-07-26 — GPUI-CE dependency swap (impl 0008)

- Moved off crates.io `gpui` to rev-pinned [GPUI-CE](https://github.com/gpui-ce/gpui-ce) (`gpui` + `gpui_platform` at `6c799b8e99`).
- Two blockers forced it: crates.io `gpui 0.2.2` (~Nov 2025) is the final publish and predates the platform split, so there is no upgrade path; and `backdrop_filter` exists only in gpui-ce.
- Entry point changed: `Application::new()` no longer exists, construction is `gpui_platform::application()`.
- This reversed an earlier stance ("stay on crates.io until a concrete blocker, then a minimal patch-fork"). The blocker appeared and the whole-fork route won. Enduring rules now live in [`arch/tech-stack.md`](../arch/tech-stack.md); the decision notes are archived.
- Later finding: the `backdrop_filter` justification is **currently unexercised** — the designed surfaces are flat `bg-surface` with borders, no `background_blur` anywhere. The abandoned-crate reason is what carries the swap.

## 2026-07-26 — Asset layer and a palette correction

- **`theme.rs` had the wrong accent for a session.** It was written from a 36-day-old palette memory saying amber `#F5A624`; the actual design file is "Design System · Cyberpunk Neon" with magenta `#FF2D7E`. Caught by reading the `.pen` before building UI on top of it. Lesson recorded: re-read `get_variables` before writing theme code, the `.pen` is ground truth.
- Regenerated all tokens from the live design: 22 colors, 3 radii, 3 font families.
- None of the design's font families (Rajdhani, Inter, Geist Mono) are macOS system fonts, so they are embedded — `crates/pulse-app/assets/` with the four TTFs (SIL OFL) plus the lucide icons the row needs (ISC), behind a hand-rolled `AssetSource` over `include_bytes!`. No `rust-embed` dependency.

## 2026-07-26 — Docs restructured

- Adopted Runner's convention: an `archive/` subdir per docs section; notes move there once their stage ships or their decision is applied, keeping their number. The active listing answers "what is being worked on now".
- Archived stages 1–4, 6, and the applied decision notes 0007/0008. Rule added and immediately exercised: **fold enduring decisions into `docs/arch/` before archiving**, so an archive never buries live guidance.
- Swept stale content: `tech-stack.md` still called the shell crates.io `gpui`, two docs pointed at Zed's GPUI source instead of the gpui-ce checkout (different renderer — reading Zed's would describe a binary we do not run).

## 2026-07-26 — Stage 7: playback row MVP shipped (impl 0009, PR #7)

- Scope narrowed to one runnable slice on Jason's call: drag a file in, it plays, play/pause, drag-to-seek. Built by the codex crew from `0009`.
- Landed: window-wide single-file drop (FLAC, ALAC in M4A, AIFF, WAV) with visible rejection messages, the `qKkw7` row rendered from theme tokens, play/pause on real `PlaybackState`, a 16px drag target around the 4px visual track sending one `Seek` on release, and macOS CI running `make verify`.
- `make verify` green at 32 tests (22 engine, 8 app, 2 CLI). Jason confirmed a dropped file plays and the feel is good.
- **The pivot's headline risk resolved favorably: GPUI matched the layout geometry, icon sizing, badge offsets, 4px track, and static Rajdhani weights without fighting the design.**
- Design decisions taken honestly rather than faked: neutral placeholder instead of the design's cover art (extraction out of scope), real single-track queue count instead of the design's static `7`.
- Tooling limit found: GPUI's offscreen Metal `render_to_image` path omits all text, including system-font controls, so automated pixel comparison of text is not possible. The live window renders text fine.
- Scope creep to note: CI was added beyond the mission brief. Kept — it is good — but it was not asked for.

## 2026-07-26 — Stages 7.5 and 8 planned (impls 0010, 0011)

- Read the full design set to plan the next milestone. Wrote `0010-app-shell.md` and `0011-output-device-management.md`.
- **The app shell is the real prerequisite.** Device management and Storage both live inside chrome that does not exist — today the app is a bare row in an empty window. The chrome is identical across all four screen frames and has no design gaps, so it goes first as stage 7.5.
- Findings from the design read, each recorded in the notes:
  - The shipped row was built from the standalone `qKkw7` component, but every screen instantiates it with a **52px cover and 330px now-playing zone** (component says 60/320). `0010` corrects it.
  - The sidebar has an OUTPUT → **Devices destination with no page frame behind it**. Stage 8 therefore ships through the Output Device Popover (`vH78z`); `pv9Av` Device Row and `Y8Ojv` Status Pill belong to that unbuilt page.
  - The popover's "Up to 24-bit / 192 kHz" line needs **capability data the engine does not expose** — `hal.rs` is private with `pub(crate)` helpers, and `validate_output_format` only validates one requested format. `0011` calls for a narrow public capability query, not opening `hal` up.
  - **Token drift:** the Storage panels use `#151514` and `#111110`, which are in no token (nearest: `bg-surface #161615`, `bg-page #0F0F0F`), and the MANAGE nav group is raw hex. Needs a re-tokenize pass in Pencil before stage 11 rather than new near-duplicate constants in `theme.rs`.
- **Stage 9 deliberately left unplanned.** The Add Storage flow, scan progress/loading, and offline-root/scan-failure states are undesigned; a note now would invent surfaces. Blocker recorded on the roadmap row.
- Next: `0010` app shell, then `0011` device management. Blockers: Jason's hardware smoke pass (carried from stage 6 and 7), and Pencil design passes before stage 9.
