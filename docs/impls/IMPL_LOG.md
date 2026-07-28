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

## 2026-07-27 — Stage 7.5: app shell built (impl 0010)

- Landed `shell.rs` (sidebar, three nav groups, top bar, routed placeholder body, docked row) and `menu.rs` (macOS app menu + standard shortcuts). Nav routing is a plain `Destination` enum on the shell view, no router. Six new lucide SVGs plus a `pulse-mark.svg` traced from the design's brand path. No engine changes, no new dependencies.
- **The window went 1280×800 → 1440×900** to match the design frames. Later stages' content (album grid, Storage panels) is laid out for the 1204px right column left after the 236 sidebar, so matching the canvas keeps those at designed proportions. Plenty of the chrome is still fixed to its designed size — search 420, top bar 74, player 92, player sections 330/300 — but the right column and body flex, which is what the decision rests on: nothing assumes a 1440 window.
- **Row corrected to its in-context size** — cover 52×52, now-playing 330. Confirmed from the `E3N1P` instance overrides (`iIdh0`, `MVw5k`); the standalone `qKkw7` component really is 60/320, so the instance is what to trust.
- The window-wide drop target and the scrub mouse handlers moved from `PlaybackRow`'s root up to the shell root, which delegates into the row entity. The row's render is now just the 92-tall bar.
- **gpui-ce has no letter-spacing API at all.** The three group headers specify `letterSpacing: 0.8` and it cannot be built — grepped `gpui/src` and `gpui_elements/src` for `letter_spacing`/`tracking`, zero hits. First case in this project of the design genuinely fighting the framework rather than just being fiddly.
- **Rajdhani 600 is not embedded** — only Medium (500) and Bold (700) are vendored, and the design uses 600 for nav and Settings labels. Code says `FontWeight::SEMIBOLD` and GPUI matches to the nearest face. Vendoring the SemiBold TTF is the fix if it reads too heavy.
- gpui-ce menu findings, all verified against the pinned checkout rather than Zed's docs:
  - **`SystemMenuType` has only a `Services` variant.** `0010` says to use `MenuItem::os_submenu` for "Services, Window"; there is no `::Window` at this rev, so Window is a plain `Menu` with custom Minimize/Close actions and does not get AppKit's auto-managed window list.
  - **Element-level `.on_action` will not fire when nothing is focused** — `focus_node_id_in_rendered_frame(None)` falls back to the dispatch-tree root, whose path excludes child nodes. Window-scoped actions therefore use global `cx.on_action` plus `cx.active_window()`, not a handler on the shell's root div.
  - **An action with no registered handler is greyed automatically** — `validate_menu_item` returns `is_action_available`. About Pulse exploits this deliberately: the item sits in its standard position and is disabled until an About surface is designed.
  - Cmd+W quits, because closing the only window would otherwise leave Pulse running with no way to get one back.
- Corrections to `0010` found while building: the brand is a **one**-line copy block (`WA1Ph` has a single text child), not two; and the body inset is gap 18 on `E3N1P`, not the gap 20 the note took from the Storage screen.
- **Token drift in this stage's scope is a non-issue.** Every raw hex in the MANAGE group and the Storage badge (`#6A6A66`, `#9D9D99`, `#1E1E1C`, `#312F2C`, plus literal `"Geist Mono"`/`"Rajdhani"`/`6`/`4`) is an exact match for an existing token, so everything bound cleanly with no new constants. The genuinely-untokenized `#151514`/`#111110` are Storage-screen panels and still need the Pencil re-tokenize pass before stage 11.
- `make verify` green at 34 tests (22 engine, 10 app, 2 CLI). Codex crew review pass on the working-tree diff found no must-fix code issues.
- **Gotcha for every future UI stage: agents cannot verify the GPUI window in this sandbox.** `screencapture` fails with "could not create image from display" (no Screen Recording permission) and `osascript`/System Events fails with `-1719` (no Accessibility permission); only process enumeration works, which is enough to confirm the app launched as a regular GUI process and nothing more. The visual diff against the design, nav click-through, menu shortcuts, and the drop-to-play regression were therefore left to Jason rather than claimed. A headless option exists but is not free — `gpui::TestAppContext` needs `gpui` added as a dev-dependency with the `test-support` feature.
- Also worth knowing: the Pencil MCP needs `/Applications/Pen.app` actually running, not just its MCP server process. "transport not connected to app: desktop" means launch it — `open -a /Applications/Pen.app design/pulse-desktop.pen`.
- Next: `0011` device management. Blockers: Jason's interaction pass on this shell, his hardware smoke on the Matrix Mini-i Pro 4 (carried from stages 6 and 7), and the Pencil re-tokenize plus stage 9 design passes.

## 2026-07-27 — No text had ever rendered: `gpui_platform` was missing the `font-kit` feature

Jason ran the shell and screenshotted it: every icon, fill, border and radius correct, and **100% of text missing** — brand wordmark, nav labels, group headers, badge count, search placeholder, body heading, drop hint, and the whole playback row.

Severity **P1** — a common, user-visible failure that blocked every text-bearing UI workflow and every stage built on them. Not P0: no data loss, no crash, and engine validation and the CLI harness were unaffected. Against pinned gpui-ce rev `6c799b8e99`.

**Root cause: `gpui_platform`'s `font-kit` feature defaults to off, and `MacPlatform::new` silently falls back to `gpui::NoopTextSystem` when it is absent** (`crates/gpui_macos/src/platform.rs:193-196`). The entire real macOS text system, `gpui_macos/src/text_system.rs`, is `#[cfg(feature = "font-kit")]`. Pulse depended on `gpui_platform` with default features, so it had been running a no-op text system ever since the gpui-ce swap. The fix is one line in the workspace manifest: `features = ["font-kit"]` on `gpui_platform`. No new package or version and no rev-pin change — this activates `gpui_macos`'s existing *optional* `zed-font-kit` dependency, whose package was already locked because `gpui` enables it for its own crate. That is exactly why nothing looked missing: the lockfile diff is a single new dependency edge, not a new crate.

Verification status: feature resolution (`cargo tree -p pulse-app -e features -i gpui_macos`), system font count `11` → `370`, and `Rajdhani`/`Inter`/`Geist Mono` going from all collapsing onto one shared fallback `FontId` to three distinct real ids were all measured directly. **Jason then confirmed eyes-on that text renders in the live window.** The remaining `0010` acceptance checks — nav click-through, menu shortcuts, and the drop-to-play regression — are tracked separately below and are not covered by this fix.

- **It predated the shell.** Built `main` (8e21507, none of the shell work) in a throwaway worktree and reproduced the identical failure, so stage 7.5 was cleared as the cause before the fix was applied.
- **This corrects the 2026-07-26 stage 7 entry twice over.** Its claim that "the live window renders text fine" was never verified and was false. And its "GPUI's offscreen `render_to_image` omits all text, so automated pixel comparison of text is not possible" was not a tooling limitation at all — it was this same no-op text system seen from the offscreen side. Offscreen text comparison may be viable now; retry it before treating it as blocked.
- **Dead ends, recorded so nobody re-walks them:** the TTFs are well-formed with correct `name`-table families and PostScript names present, so the font files were never the problem; font-kit's `MemSource::add_font` silently dropping fonts whose `postscript_name()` is `None` (`src/sources/mem.rs:191`) looked promising but was a red herring; and switching `assets::fonts()` from `Cow::Borrowed` to `Cow::Owned`, to take font-kit's `Handle::from_memory` path instead of `Handle::from_native(CGFont)`, changed nothing. All of that was chasing a font-registration bug while the text system itself was a stub.
- **Why it hid for two stages, and the lesson.** gpui swallows every text failure with `.log_err()` (`crates/gpui/src/elements/text.rs`, on both `shape_text` and `line.paint`) and Pulse installs no logger, so a total text failure produced zero output and exit code 0. `NoopTextSystem` also *succeeds* at shaping and returns plausible advances, so layout reserved correctly-sized gaps and only glyphs were missing — which is why it read as a rendering bug rather than a missing text backend. `make verify` cannot catch any of it. **When debugging gpui text, check the enabled features on `gpui_macos` first — `cargo tree -p pulse-app -e features -i gpui_macos` — before suspecting fonts or the renderer.** To surface swallowed errors, temporarily add `log = "0.4"` (already in the lockfile) with a ten-line stderr `log::Log` impl.
- Still unguarded: nothing in the build fails if that feature is dropped again. The manifest carries an explanatory comment at the point of failure, but a real guard would need either a startup assertion or `gpui`'s `test-support` dev-dependency for a headless text test. Worth deciding before stage 8.

## 2026-07-27 — Session close: branch state and pickup point

Mission archived here. Stage 7.5 and the P1 font fix are both **complete and reviewed but deliberately uncommitted** — the mission authorized working-tree changes only. Anyone resuming starts from this snapshot.

Tree: branch `feature/stage-7.5-app-shell`, **0 commits ahead**, `main` untouched at `8e21507`. Fifteen files of working-tree change — two new source files (`shell.rs`, `menu.rs`), three modified (`main.rs`, `playback_row.rs`, `assets.rs`), seven new SVGs under `crates/pulse-app/assets/icons/`, `Cargo.toml` + `Cargo.lock` for the font fix, and this log. No diagnostic residue; the temporary `log` dependency, stderr logger, probe blocks, and throwaway worktree used to diagnose the font bug were all reverted and removed.

Verified: `make verify` exit 0 at 34 tests (22 engine, 10 app, 2 CLI), clippy under `-D warnings`, fmt clean. Codex crew reviewer reached a clean review over three rounds covering the full tracked and untracked diff, and confirmed the delegation of the drop/scrub handlers into the row entity by reading it. Text renders in the live window, confirmed eyes-on by Jason after the font fix.

**Not verified, and must be done before stage 7.5 is called accepted** — all four need a human, since agents cannot drive or capture the window here:

- Nav click-through: each item swaps both selection and body, with active styling following.
- `Cmd+Q` / `Cmd+W` / `Cmd+M` / `Cmd+H`.
- **Drop-to-play regression, the must-fix one:** drop a real audio file and confirm it plays, the row updates, play/pause toggles, and the progress bar seeks. This is the highest-risk check in the stage, because the shell moved the window-wide drop target and the scrub mouse handlers off `PlaybackRow`'s root and onto the shell root, which now delegates into the row entity.
- Hardware smoke on the Matrix Mini-i Pro 4, carried unresolved since stage 6.

Decisions left open, none blocking: whether to add a regression guard for the `font-kit` feature (an offscreen/headless glyph test is the candidate and may now be viable, since the stage 7 claim that offscreen rendering omits all text turned out to be the no-op text system rather than a tooling limit); whether to vendor Rajdhani SemiBold so nav and Settings labels hit the designed weight 600 instead of matching to the nearest embedded face; whether a greyed-out "About Pulse" is preferable to no About item; and whether `Cmd+W` behaving as `Cmd+Q` is acceptable while Pulse is single-window.

Housekeeping: `git worktree list` reports `/private/tmp/pulse-review-pr3-dc634a2` as `prunable`, left over from an earlier PR-3 review and unrelated to this work. `git worktree prune` clears the stale registration.

Next: land this branch, close out the four checks above, then `0011` device management. Still-standing blockers beyond those: the Pencil re-tokenize pass for the Storage panels' `#151514`/`#111110` before stage 11, and the stage 9 design passes for the Add Storage flow.

## 2026-07-27 — Stage 7.5 merged (session close)

- Branch committed (`73eedbf`) and merged to `main` as `e6fd6a4`. `make verify` green on the merge result at 34 tests; CI green on a clean machine, including the `Cargo.lock` check that guards the `font-kit` manifest change.
- Jason ran the shell and confirmed it looks right — the first look with text actually rendering. The remaining interaction checks (nav click-through, Cmd shortcuts, and the drop-to-play regression) were not walked through and stay open.
- Correction recorded against the stage 7 entry above: the claim that GPUI's offscreen renderer omits text "but the live window renders text normally" was wrong. The live window never rendered text either; the offscreen result had been showing the truth and was explained away as a tooling limit.
- Next session: `0011` device management. Carried blockers unchanged — Jason's hardware smoke on the Matrix Mini-i Pro 4 (open since stage 6), the Pencil re-tokenize pass before stage 11, and the stage 9 design passes for the Add Storage flow, scan progress, and failure states.

## 2026-07-28 — Stage 8: output device management built

- Added the `vH78z` playback-row popover with live Core Audio enumeration, confirmed active-device switching, UID-only app persistence, launch fallback messaging, relisting on open, permanent network empty state, and plain undesigned error text. The Devices nav destination remains the existing placeholder.
- Added the narrow engine `output_device_capabilities` query over advertised signed-integer PCM physical formats and a pure max-format test. Successful device changes now emit `OutputDeviceChanged`; a failed active switch restores the prior controller target instead of leaving UI and engine state split.
- The popover says "Exclusive during playback" rather than asserting idle per-device hog state. Pencil's 0.8 section-label tracking remains impossible at the pinned gpui-ce revision.
- Initial `make verify` was green at 39 tests. `make run` launched without panic until manually stopped. `pulse-cli devices` exited successfully but returned no devices in the sandbox, so live list equality, visual interaction, persistence across relaunch, audio switching, unplug/hog errors, and the Matrix hardware pass remain Jason-only checks.
- Reviewer round one fixed a capture-phase toggle race that reopened the popover, made transient launch-enumeration recovery resolve the saved UID again, restored the playback row's shipped 132px format block, clarified unsupported physical-format wording, removed an unreachable selection branch, and added app device-state tests. `make verify` is green at 41 tests after the fixes.
- Reviewer round two reported no remaining must-fix issues. Changes remain uncommitted on `feature/stage-8-device-management`; the GUI, persistence, and hardware checks above are the remaining acceptance work.

## 2026-07-28 — Stage 8 merged (session close)

- Merged via PR #8 as `a629a09`. `make verify` green at 41 tests (23 engine, 16 app, 2 CLI); CI green on both the branch commit and the merge.
- Code review of what landed: the engine boundary held as specified — `hal` is still a private module and the new helper is `pub(crate)`, with `device::output_device_capabilities` handing the UI a plain `OutputDeviceCapabilities { max_bits_per_channel, max_sample_rate }` plus a new `NoOutputCapabilities` error variant. App persistence is a single plain-text UID at `<config>/pulse/app-output-device.uid`, no serde or toml, separate from the CLI's config, with a parse test. `dirs` is the only new app dependency and was already a workspace dep.
- **Caveat found while reading, not a bug but decide before it ships to users:** `maximum_physical_format_capabilities` takes the bit-depth and sample-rate maxima *independently* across formats — its own test asserts that a device advertising 32-bit up to 96 kHz and 24-bit up to 192 kHz reports `(32, 192000)`. The popover can therefore display a pair the device does not support together. The design's "Up to 24-bit / 192 kHz" phrasing reads as two independent ceilings so this matches the mockup, and the query correctly filters to signed-integer PCM (excluding float), but on an audiophile-facing surface it is the kind of claim worth being deliberate about. Options if it matters: report the best supported *pair*, or reword to make the independence explicit.
- Still Jason-only, unchanged by the merge: visual check of the popover against `vH78z`, device switching by ear, persistence across relaunch, unplug/hog error paths, and the Matrix Mini-i Pro 4 hardware smoke open since stage 6. `pulse-cli devices` returns no devices inside the agent sandbox, so even the list could not be cross-checked there.
- Next: stage 9 is still blocked on design (Add Storage flow, scan progress, offline-root and scan-failure states). The Pencil re-tokenize pass for the Storage panels' `#151514`/`#111110` is also still outstanding before stage 11.

## 2026-07-28 — Device list verified

- Jason confirmed the popover's device list is correct against his real hardware. This was the one check agents could not do at all — `pulse-cli devices` returns nothing inside the sandbox — so the live Core Audio enumeration path is now verified end to end.
- Still open from stage 8: device switching by ear, persistence across relaunch, and the unplug/hog error paths. Still open since stage 6: the Matrix Mini-i Pro 4 hardware smoke.

## 2026-07-28 — Stage 9-10 backend planned (impl 0012)

- Wrote `0012-library-scan-and-store.md` covering the headless half of stages 9 and 10: storage roots, the file walk, tag extraction, and the SQLite store. The Storage screen stays out, still blocked on its Pencil passes.
- **Placement decided: library code lives in `crates/pulse-app` as a `library/` module, not a new crate.** I had proposed a `pulse-library` crate; Jason pushed back and was right. `arch/pulse-engine.md` only requires it be outside the *playback engine*, `pulse-app` is the only consumer, `pulse-cli` exists to prove engine and playback behavior rather than library behavior, and `pulse-app` modules are unit-testable as stage 8's preferences parser already shows. Extract to a crate if and when a second consumer appears.
- The realisation that unblocked this: only the Storage *screen* needs design. Scanning, tagging, and storing are headless, so they can be built and proven now — which also sidesteps the sandbox's inability to verify a GPUI window.
- Decisions recorded in the note: `rusqlite` with the `bundled` feature so a packaged `.app` does not depend on the host's SQLite; hand-rolled `std::fs` walk over adding `walkdir` unless symlink loops prove fiddly; FTS5 deliberately deferred since it is an additive migration; scan history stored because the Storage screen shows it and it cannot be backfilled; incremental rescan keyed on modified time.
- Testing follows the existing pattern — no binary fixtures anywhere in the repo, `pulse-engine` synthesizes buffers in memory. Store tests use `:memory:` SQLite, walk tests use temp dirs of empty files, and tag extraction generates a minimal WAV in-test.
- Next: run 0012 as a mission. Blockers unchanged — Jason's Pencil passes gate the Storage screen, the Matrix hardware smoke is still open from stage 6, and the Storage panels' `#151514`/`#111110` still need re-tokenizing before stage 11.
