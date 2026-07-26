# 0007 — GPUI native UI pivot

## Status

Decided 2026-07-20. Supersedes the Tauri 2 + React app-shell decision in `arch/tech-stack.md` (2026-06-03). Engine work is unaffected; stage 6 (playback controller) proceeds unchanged.

## Decision

Pulse's UI ships as a native GPUI app in a new `crates/pulse-app` workspace member. The Tauri + React scaffold (`src/`, `src-tauri/`, the JS toolchain) is deleted. One Rust binary: GPUI renders the UI, `pulse-engine` stays the standalone, UI-agnostic core, `pulse-cli` stays the scriptable harness.

## Why now

The original "Why Tauri" reasoning said the UI wedge is faster in React/CSS and the toolchain was proven in Quill and Runner. Two things changed:

- Runner's cancelled 0031 GPUI rewrite (postmortem + salvaged docs/code: memory repo `projects/runner/gpui-rewrite/`) de-risked GPUI end-to-end on real work: rendering at production quality, Chinese IME via `EntityInputHandler` with no platform code, hand-rolled .app assembly + Developer ID codesign proven, API churn real but never blocking. That rewrite died to the parity treadmill — replacing 28k LOC of shipped UI. Pulse has zero shipped UI, so the killer risk does not exist here; this is a greenfield choice, not a rewrite.
- Pulse is macOS-only by nature (Core Audio) and already deep in the `objc2-*` ecosystem, so GPUI's macOS-first posture and Metal renderer cost nothing, and shell services (media keys, Now Playing via `MPRemoteCommandCenter`/`MPNowPlayingInfoCenter`) are idiomatic `objc2` calls from the native app instead of "later from the Tauri backend".

Structural wins for an audio app specifically: the UI observes the engine in-process — controller state, playback events, and later the `realfft` level/spectrum taps flow to render code with no IPC hop, no serialization boundary, no webview frame pacing in the way.

## What changes / what doesn't

Unchanged: `crates/pulse-engine`, `crates/pulse-cli`, all HAL/AUHAL/decode decisions, the roadmap's engine stages, Pencil as the design source (`design/pulse-desktop.pen`), the stage rule that no frontend surface is built before its design exists.

Changes:

- Delete `src/`, `src-tauri/`, `index.html`, `vite.config.ts`, `package.json`, `pnpm-lock.yaml`, `tsconfig*.json`, `dist/`, `node_modules/`; drop `tauri`/`tauri-build` from the workspace.
- Add `crates/pulse-app`: gpui + `pulse-engine`. Theme carries over as Rust constants derived from the Pencil design — data, not CSS. The GPUI source was swapped to rev-pinned GPUI-CE in [impl 0008](0008-gpui-ce-dependency.md).
- Reuse from the 0031 salvage: the IME-capable input field (`composer.rs` pattern — marked-range composition, `bounds_for_range` anchoring; NSTextInputClient offsets are relative to the marked string) for library search, and grapheme-cluster text editing (`text_util.rs`, `unicode-segmentation`).
- Build prerequisite: Xcode 26 Metal Toolchain component (`xcodebuild -downloadComponent MetalToolchain`, one-time ~3 GB) for gpui's shader build; CI needs it too.
- Licensing guardrail: `gpui` is Apache-2.0 and fine as a dependency; Zed's app crates are GPL — architectural reference only, never copy code.

## Roadmap impact

- Stage 6 boundary reads "thin CLI/app adapters" — the Tauri adapter half becomes the `pulse-app` adapter, delivered in stage 7.
- Stage 7 becomes: scaffold `crates/pulse-app` (window, theme, playback row surface from the Pencil design) and wire the stage 6 controller into it directly — app-owned controller state, in-process event subscription, no command/IPC layer.
- Stages 8–14 keep their scope; "frontend" now means GPUI surfaces built from `design/pulse-desktop.pen`.
- Packaging (stage 14) follows the proven 0031 pattern: scripted .app assembly + codesign + notarize of the bare cargo binary.

## Risks

- The cinematic/cyberpunk design language was the argument for CSS. Gradients, blur, glow, and motion in GPUI mean custom elements and paint code, not stylesheets. Mitigation: stage 7 deliberately builds the playback row — a visually demanding surface — as the visual spike; if the design language fights the framework, we learn it in the first slice, not stage 11.
- Album-art at grid scale (hundreds of covers, scroll perf, image decode/caching) is unproven in our hands. Validate with `gpui::img` + `uniform_list` early in stage 11's first pass.
- GPUI docs remain thin; the gpui-ce checkout is the working reference. Accepted — known from 0031, never blocking.
- No updater story for a native binary yet; same deferral as 0031 (GitHub-Releases check or Sparkle), decide at v0 release, not before.
