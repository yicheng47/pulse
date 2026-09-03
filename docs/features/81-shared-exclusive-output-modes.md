# Shared and Exclusive Output Modes

> Feature 81 · P2 · GitHub issue [#81](https://github.com/yicheng47/pulse/issues/81). Filed by Jason 2026-09-03. Collapse the three peer modes — Shared · Exclusive · Bit-perfect — into two device-ownership policies, Shared and Exclusive, and make "Bit-perfect" what Exclusive *earns* at runtime when the device has a safe integer path, reported only as the Signal Path verdict. Spec written 2026-09-03; Pencil pass pending before the designed surface lands.

## Motivation

Shared and Exclusive say whether Pulse owns the device; Bit-perfect is a claim about the active signal path. Offering them as peers makes the user pick between a policy and a result, and a "Bit-perfect" segment that can refuse at start (the AUTO gate gap in [feature 78](78-integer-engine-hardening.md) §4.3 of its review) is a promise the control cannot keep. Two modes make Exclusive the intent, the integer backend the way Pulse honours it when the device allows, and the Signal Path popover the place the verified result is reported — which is already how the code is split (`EngineKind::Universal { exclusive_mode }` vs `EngineKind::BitPerfect`, the AUHAL engine vs the integer engine).

## Design Source

Pencil pass before implementation (Jason, in the issue); Jason approves before phase 2. Screens in `design/pulse-desktop.pen`: the Output Device Popover (`vH78z`) — mode row and the active-device meta line; Settings / Output (`XmoeK`) — mode control and the `NO INTEGER PATH` tag; Spec — Engine Control (`TrYbi`) — the three states become two policies with a verdict; Spec — DSD Gate & Toasts (`GGd35`) — refusal copy. The Signal Path popover (`iay7h`) is unchanged: it already reports the runtime verdict (`Bit-perfect · hog held · mixing off`).

## Model

- **Stored modes**: `StoredOutputMode::{Shared, Exclusive}` (`backend/settings.rs`). Auto stays the per-device default (`OutputModePreferences::is_pinned` false) and Reset to Auto keeps its behaviour.
- **Resolution** (app side, at device selection, mode change, and retry — replacing `engine_kind_for_output_mode(mode)` with a resolver over mode + capabilities): Shared → `EngineKind::Universal { exclusive_mode: false }`. Exclusive → `EngineKind::BitPerfect` (the integer backend) when the capabilities say the device has a safe integer path — `integer_wire_formats` from feature 78 stage 2's `is_integer_wire_format` predicate, plus the existing transport rule (`DeviceTransport::supports_bit_perfect`) — otherwise `EngineKind::Universal { exclusive_mode: true }`. Auto → Exclusive when the device advertises any exclusive-capable format (`max_bits_per_channel.is_some()`), else Shared: today's `automatic_output_mode` / `automatic_stored_output_mode` with the Bit-perfect arm removed. The integer decision is made at resolution time from live or stored capabilities and is never stored.
- **App state**: `Playback` gains `resolved_engine_kind: EngineKind` beside `playback_output_mode` (which keeps the stored-mode vocabulary for display). Set at resolution, confirmed by `OutputDeviceChanged { kind }`, set to shared by `ExclusiveModeFallback`; `output_mode_for_engine_kind` maps both exclusive kinds to `Exclusive`.
- **Engine unchanged**: `EngineKind` keeps both variants; the two backends, the DoP packer, volume behaviour, pause hold, and feature 77's unlock are untouched (issue non-goals).
- **Bit-perfect verdict**: `signal_path_verdict` already reads `bit_perfect_active`, which the engine sets only while the integer backend runs. After this feature nothing in the app prints "Bit-perfect" from a stored mode — only the verdict does.
- **DoP gate** (unchanged in strictness): `dsd_playback_error` / `dsd_playback_error_with_sample_rate` and `stop_before_unsafe_dsd_output_change` key on `resolved_engine_kind == EngineKind::BitPerfect` instead of `output_mode == BitPerfect`. Refusal: when the device has an integer path but the mode is Shared, the toast offers "Switch to Exclusive" (the existing `SwitchToBitPerfect` action and `switch_to_bit_perfect_and_retry` renamed, retry flow unchanged); when the device has no integer path, no action and the body names the device. Final copy is the Pencil pass's.
- **Migration**: `StoredOutputMode::Exclusive` takes `#[serde(alias = "bitPerfect")]`, so a stored `"mode": "bitPerfect"` loads as Exclusive and the next settings write persists `exclusive`; the `exclusiveMode` legacy path (`migrate_legacy_mode`) keeps working. No notice: a device pinned to Bit-perfect resolves Exclusive → integer backend, the same behaviour it had.
- **Copy**: `output_mode_meta` → `CoreAudio · Shared` / `CoreAudio · Exclusive`; the popover mode row and the Devices page drop the third `output_mode_segment`; the `NO INTEGER PATH` tag stays (it now explains why Exclusive will not be bit-perfect on that device); the verdict line stays.

## Decisions to confirm

1. **Per-track integer refusals fall back to AUHAL exclusive.** Today a mono file, or a 32-bit source on a device whose widest container is 24 bits, refuses on the integer backend (`NoMatchingPhysicalFormat`) with no fallback (78 review §4.5 and its stage 5). With Exclusive as the intent, the honest behaviour is to fall back to `Universal { exclusive_mode: true }` for that track and let the verdict read Transparent, with no error; DSD never falls back — the gate refuses before play. My pick: yes, as phase 3; it also resolves 78 stage 5 without touching the packer. Declined → the errors stay as they are.
2. Whether the active-device meta line shows the resolved path (`CoreAudio · Exclusive · integer`) or leaves that to the tag and the verdict — Pencil pass.

## Non-Goals

- Removing either backend; weakening the integer-path capability checks or DoP safety.
- Changing sample processing, volume behaviour, pause-lock ownership, or manual unlock (feature 77).
- Separate Exclusive and Integer toggles (Roon-style).
- Redesigning the Devices page beyond the mode control and its directly related copy.

## Dependencies

- **Feature 78 stage 2** (`integer_wire_formats` predicate, probe flag, stored capability + reprobe). Either stage 2 lands first and phase 1 consumes it, or phase 1 carries that plumbing and stage 2 reduces to its predicate tests — the lead decides at mission time. The two must be sequenced, never concurrent: same files (`hal.rs`, `device.rs`, `settings.rs`, `logic.rs`).
- **Feature 77** lands its lock button on the same popover card; no code overlap, copy coordinated in the Pencil pass.

## Implementation Phases

1. **Model and resolution (headless)** — `StoredOutputMode` to two variants with the alias migration; `resolved_engine_kind` and the resolver over mode + capabilities; `output_mode_for_engine_kind` collapsed; the DoP gate and the stop-before-change check on the resolved kind; toast action and retry helper renamed; the minimum surface change to stay truthful and compile — third segment removed, meta copy. Tests: resolver table (capabilities × mode → kind, including no-integer-path and display transports), migration (`bitPerfect` loads as Exclusive; legacy `exclusiveMode` still migrates), DSD gate on Shared-with-integer-path (offers the switch) vs no-path (no action), verdict never derived from a mode. One `codex-crew` mission, PR into `main`, Jason reviews.
2. **Designed surface** — after the Pencil pass: final copy for the mode control, meta line, tag, refusal toasts, and the Engine Control board's states. Small mission or a lead commit.
3. **Fallback** (if decision 1 is confirmed) — in `start_backend`'s error arm for the `BitPerfect` kind, a `NoMatchingPhysicalFormat` / `UnsupportedFormat` start falls back to `Universal { exclusive_mode: true }` with an `ExclusiveModeFallback`-style event; verdict Transparent; fake-backend tests.

## Verification

- `make verify` green with the tests above.
- Manual (Jason): Matrix under Auto → popover reads Exclusive, Signal Path reads Bit-perfect while a PCM track plays; DSD plays. Switch to Shared → the DSD refusal offers "Switch to Exclusive" and the retry plays. A DELL display under Exclusive → Signal Path Transparent, `NO INTEGER PATH` tag, no start error. An old `settings.json` with `"mode": "bitPerfect"` loads as Exclusive with no notice.
