# 0008 — GPUI-CE dependency swap

## Status

Applied 2026-07-26. Amends 0007's dependency line ("gpui (crates.io 0.2.x)"); the native-GPUI decision itself stands unchanged.

## Decision

`crates/pulse-app` depends on GPUI Community Edition (`github.com/gpui-ce/gpui-ce`) as a rev-pinned git dependency instead of crates.io `gpui`. Source-reading reference moves from Zed's checkout to a gpui-ce checkout.

## Why now

- **crates.io `gpui` is abandoned.** `0.2.2` is the last publish (~Nov 2025) and predates the platform split. Zed stopped publishing GPUI to crates.io when it deprioritized GPUI as a general-purpose framework, so there is no upgrade path on that dependency — it will never get another release.
- **gpui-ce implements `backdrop-filter`; Zed's GPUI does not.** Its `Primitive` enum carries `BackdropFilter` and `FilterBoundary` variants upstream lacks, behind a CSS-equivalent API (`Styled::backdrop_blur(radius)`, `Styled::backdrop_filter` for chains, `Window::paint_backdrop_filter`). This directly mitigates 0007's first risk — "gradients, blur, glow, and motion in GPUI mean custom elements and paint code, not stylesheets". The frosted-glass now-playing background becomes `.backdrop_blur(px(40.))` over a translucent `.bg(...)` rather than a custom element.
- **`gpui_elements::editable_text` ships `text_input` and `text_area`.** 0007 planned to carry the 0031 IME composer salvage into library search; evaluate these elements first, as they may reduce or replace that work.
- **Apache-2.0 throughout, and self-contained.** gpui-ce's first PR removed non-Apache crates, and its latest commit ("removed all of the git sources") finished decoupling from Zed's non-GPUI crates by vendoring `collections`, `sum_tree`, and `refineable` as `gpui_collections`, `gpui_sum_tree`, `gpui_refineable`. This strengthens 0007's licensing guardrail rather than weakening it.
- **It targets external consumers.** gpui-ce states general use as its goal; Zed's GPUI serves Zed only and breaks outside users without notice. Hummingbird (Apache-2.0 GPUI music player, ~577 stars, actively developed) ships on gpui-ce — the closest existence proof available for this app category.

## What changes / what doesn't

Unchanged: `crates/pulse-engine`, `crates/pulse-cli`, all HAL/AUHAL/decode decisions, roadmap stages, Pencil as design source. The app is still one native GPUI binary; only the GPUI source and the entry-point call change.

Changes: three files, described below.

## How it lands

Workspace `Cargo.toml` — replace line 23 (`gpui = "0.2"`) with:

```toml
gpui = { git = "https://github.com/gpui-ce/gpui-ce", rev = "6c799b8e99" }
gpui_platform = { git = "https://github.com/gpui-ce/gpui-ce", rev = "6c799b8e99" }
```

Both crates are named `gpui` and `gpui_platform` in that repository, so no `package = ` rename is needed. Pin the rev rather than tracking `main`; `6c799b8e99` is HEAD as of 2026-07-13.

`crates/pulse-app/Cargo.toml` — add alongside the existing `gpui.workspace = true`:

```toml
gpui_platform.workspace = true
```

No feature flags. `gpui_platform`'s default feature set is empty and it pulls `gpui_macos` via `[target.'cfg(target_os = "macos")'.dependencies]`. The `wayland`/`x11`/`font-kit` features seen in other gpui-ce consumers are Linux-only.

`crates/pulse-app/src/main.rs` — the only code change. `Application::new()` no longer exists post-platform-split; construction moved behind `gpui_platform::application()`.

```rust
// before
use gpui::{App, Application, Bounds, ...};
Application::new().run(|cx: &mut App| {

// after — drop `Application` from the import list
use gpui::{App, Bounds, ...};
gpui_platform::application().run(|cx: &mut App| {
```

`gpui_platform::application()` returns `gpui::Application`, so `.run(...)` and the entire closure body — `Bounds::centered`, `cx.open_window`, `WindowOptions`, `TitlebarOptions`, `cx.activate` — are unchanged. `gpui_platform::headless()` is the test-harness equivalent.

## Verification

- `cargo build -p pulse-app` succeeds. Expect some API drift beyond the entry point: this jumps from a Nov-2025 release to a Jul-2026 tree. The compiler names each site, and `crates/pulse-app` is two files, so the surface is small.
- `cargo run -p pulse-app` opens the window and renders the "Pulse" placeholder with `theme::bg_page()` / `theme::text_primary()` intact.
- `cargo build` at the workspace root still succeeds, and neither `pulse-engine` nor `pulse-cli` gains a GPUI dependency.
- Xcode Metal Toolchain component is still required, per 0007. Unchanged.

## Roadmap impact

Stage 7 (playback row visual spike) gains `backdrop_filter`, lowering 0007's cinematic-design risk. Do this swap before stage 7 begins — migrating two files now is far cheaper than migrating a built-out surface later.

## Risks

- **gpui-ce is a hard fork, not a tracking fork.** Its history contains Zed commits up to ~PR #44442; everything after is its own work (element and state merged into a single type, paint operations merged into `PaintContext`, async-io timer replaced by the background-executor timer). It will not automatically receive upstream Zed fixes. Accepted — we need a GPUI that treats outside consumers as users, and upstream does not.
- **Rev-pinning means deliberate bumps.** Nothing breaks unexpectedly; nothing improves without action either. Re-pin when a needed fix lands.
- **`longbridge/gpui-component` is unavailable while on gpui-ce.** That widget library depends on Zed's GPUI, and mixing lineages puts two incompatible `gpui` crates in the graph. Accepted — hand-built components are the intent, and hummingbird ships ~35 of its own.
- **A future dependency may pull its own GPUI** (crates.io `gpui`, `gpui-unofficial`, or Zed git), which again yields two incompatible GPUI crates whose types will not unify. The fix is a `[patch.crates-io]` shim crate re-exporting ours under the other crate's name; hummingbird does exactly this for `cntp-i18n` in `crates/gpui-unofficial-shim`, whose entire source is `pub use gpui_real::*;`.
- **Reference reading must move to the gpui-ce checkout.** `scene.rs` is 1247 lines there versus 915 in Zed's, and `shaders.metal` 1409 versus 1279 — reading Zed's copy would describe a renderer this binary does not run.
