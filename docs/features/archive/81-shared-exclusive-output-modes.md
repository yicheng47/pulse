# Shared And Exclusive Output Modes

Feature 81 · P2 · GitHub issue [#81](https://github.com/yicheng47/pulse/issues/81). Decided by Jason, 2026-09-03: the output control should expose the user's ownership choice, while bit-perfect remains a verified playback result rather than a selectable promise. Roon uses the same separation: [Exclusive Mode](https://help.roonlabs.com/portal/en/kb/articles/faq-what-s-exclusive-mode) configures device ownership, CoreAudio Integer Mode is an advanced mechanism, and [Signal Path](https://help.roonlabs.com/portal/en/kb/articles/signal-path) reports whether the result is lossless.

## Motivation

Pulse currently presents `Shared · Exclusive · Bit-perfect` as one axis, but the labels describe different things. Shared and Exclusive say whether Pulse owns the output device; Bit-perfect claims what happened to the samples. The current UI therefore asks the user to choose an implementation/result distinction that Pulse can resolve more accurately from the device probe, and it makes the raw integer path look like a stronger flavor of Exclusive even though both exclusive backends hold the same hog.

The product choice should be simple: let other apps share the device, or give Pulse sole control. Once Exclusive is chosen, Pulse uses the best safe exclusive backend for that device. Signal Path remains the honest place to distinguish verified bit-perfect integer delivery from transparent AUHAL output or processed output.

## Design Source

`design/pulse-desktop.pen`, Pencil pass 2026-09-03, approved by Jason the same day (`1efc40d`):

- **Output Device Popover** (`vH78z`): the active-device card's Driver line (`OPHMY`) reads `CoreAudio · Exclusive`; the capability line (`B7uEMi`, green mono) is unchanged. `Output Mode Segments` (`m14ZyM`) is just the control: `Engine Segments` (`eovxT`) holds `Seg Shared` (`wioRT`) and `Seg Exclusive` (`I0sewb`) — no label, no `AUTO` tag, no `Reset to Auto` (Jason, 2026-09-03: the popover is the quick switch — Auto picks the preselected segment silently, tapping the other one pins it). The active segment is neutral — `$bg-elevated` fill, `$text-primary` label — never the green `$quality` treatment. `Seg Exclusive` carries a lucide `lock` glyph (10 px, the label's colour) on every surface — the popover, the Devices page rows, the board — saying "Pulse holds the device" (Jason, 2026-09-03). Feature 77's release button joins this card later and must read as an action on the hold, not as a third segment.
- **Settings / Output** (`XmoeK`) rows carry the `Output mode` label, the same two segments, and the `NO INTEGER PATH` tag on gated devices — no Auto affordance. The four device rows: mini-i `WRgFF` — Exclusive; AirPods `P3bGc` — Shared + `NO INTEGER PATH` (`Y579Hz`); DELL `HrZzl` — Exclusive + `NO INTEGER PATH` (`YR00G`); Scarlett `xUye0` — Shared. Third segment removed from all four.
- **Spec — Engine Control** (`TrYbi`): board note (`PAO7t`) rewritten for feature 81; STATE 1 (`K6NoG0`) AUTO → Exclusive on the integer backend, verdict BIT-PERFECT; STATE 2 (`s0EMJ`) pinned Shared — nothing to reset, DSD refusal offers the switch; STATE 3 (`c09XA`) no integer path — Exclusive is AUHAL, verdict TRANSPARENT, tag explains it; STATE 4 (`jVgc8`) the popover's quick-switch card (`SDu8E`, a copy of `JgNYd`) with the caption that separates it from the Devices page row.
- **Spec — DSD Gate & Toasts** (`GGd35`): the mode refusal is now two blocks — `Copy / SHARED ON A DEVICE WITH AN INTEGER PATH` (`t9Xb5s`): title "DSD needs Exclusive output", action `[Switch to Exclusive]`; `Copy / NO INTEGER PATH ON THIS DEVICE` (`y8cO8`): title "This device can't play DSD", no action. Rendered toast instances carrying the old strings updated in place.
- **Signal Path popover** (`iay7h`) and its spec board (`eBomZ`): unchanged — BIT-PERFECT stays the verdict for the running integer backend, TRANSPARENT for AUHAL-exclusive at unity.


## Scope

- **Two user-facing modes**: replace the three-segment control on the Devices page and output popover with `Shared · Exclusive`. Shared means other applications can use the device; Exclusive means Pulse holds the device and other applications cannot. Exact supporting copy and the compact/popover layouts go through a Pencil pass before implementation.
- **Auto remains policy, not a control**: an unpinned device shows the resolved Shared or Exclusive segment preselected, and tapping the other segment pins it. There is no Auto affordance on any surface — no `AUTO` tag, no `Reset to Auto` (Jason, 2026-09-03: keep it simple; the default is Auto's choice). The stored model keeps `mode: None` as the unpinned state; `NO INTEGER PATH` stays on gated devices. Automatic resolution keeps today's intent: choose Exclusive with the raw integer backend when the hardened integer-path capability passes; choose Exclusive with AUHAL when exclusive native-rate output is useful but the raw integer path is unsafe; otherwise choose Shared.
- **Backend resolution**: Shared maps to `Universal { exclusive_mode: false }`. Exclusive maps to the raw integer backend when the feature 78 capability predicate says the device has a safe integer path, otherwise `Universal { exclusive_mode: true }`. Both engines stay; this feature removes a product-level choice, not a compatibility path.
- **Mechanism naming**: engine-facing names describe implementation rather than outcome (`Integer`, not `BitPerfect`). Runtime state may still report that the active integer path is bit-perfect after the existing format/readback checks succeed.
- **Bit-perfect is a verdict**: retain `BIT-PERFECT` in Signal Path only for the verified raw integer path. AUHAL-exclusive remains `TRANSPARENT` when Pulse applies no steady-state sample change, with the existing disclaimer that Core Audio owns conversion after the float32 client boundary. Shared and software-volume cases keep their existing honest verdicts.
- **DSD safety follows the resolved backend**: DoP remains accepted only when playback resolves to the raw integer backend; selecting Exclusive alone is not sufficient on an incompatible device. On an integer-capable device in Shared mode, the refusal action switches to Exclusive and retries. On a device whose Exclusive path is AUHAL, refusal explains that the device has no safe integer path; exact copy is part of the Pencil pass. DoP is never sent through AUHAL.
- **Settings migration**: persisted per-device `bitPerfect` selections migrate to `exclusive`; `shared`, `exclusive`, and unpinned Auto preferences retain their meaning. The normalized settings file no longer writes `bitPerfect` as an output-mode choice.
- **Surfaces and documentation**: update mode labels, mode metadata, DSD refusal/action copy, tests, and the product/architecture docs that currently describe the three-option control. Capability details may describe whether an integer path is available, but must not reintroduce it as a third mode.
- Schedule after feature 78 integer-engine hardening so the Exclusive resolver consumes its single hardened integer-path predicate instead of creating a competing capability rule.

## Non-Goals

- Removing the universal-exclusive backend or forcing devices without a safe integer path into Shared mode.
- Weakening the integer-format predicate, format readback, transport gate, DoP refusal, or the hardware-backed bit-perfect claim.
- Changing decoding, packing, sample conversion, volume-domain behavior, or Signal Path verdict criteria.
- Changing hold-on-pause, quit release, or feature 77's manual device-unlock behavior. Exclusive continues to mean the hog is held wherever either exclusive backend currently holds it.
- Adding Roon-style independent Exclusive and Integer toggles. Pulse resolves the implementation automatically instead of exposing an advanced compatibility switch.
- Redesigning the Devices page, output popover, or Signal Path beyond the mode control and directly affected copy.

## Implementation Phases

1. **Pencil pass**: reduce both mode controls to Shared and Exclusive; settle descriptions, Auto treatment, resolved-backend metadata, and the two DSD refusal states. Jason approves the design before implementation.
2. **Preference and resolver model**: reduce stored user choices to Shared/Exclusive, migrate legacy `bitPerfect`, resolve Exclusive through feature 78's integer-path predicate, and rename engine mechanism identifiers from BitPerfect to Integer without changing the sample path.
3. **App surfaces and safety gates**: wire the two controls, expose the resolved backend separately from the selected mode where UI logic needs it, gate DoP on the resolved integer backend, and update actions/copy and Signal Path inputs.
4. **Documentation and validation**: update current architecture/product language, exercise settings migration and resolver matrices, and run the hardware acceptance pass.

## Verification

- `make verify` green.
- Settings tests load every existing state (`shared`, `exclusive`, `bitPerfect`, and no pinned mode), migrate `bitPerfect` to Exclusive, and round-trip without writing the retired value.
- Resolver tests cover Auto and pinned Shared/Exclusive across a safe integer USB DAC, an integer-advertising but gated/refused device, and a float-only device. No mode-selection branch duplicates feature 78's integer predicate.
- Controller/app tests prove that Exclusive selects the integer backend when safe and AUHAL-exclusive otherwise; changing between Shared and Exclusive preserves the existing restart, fallback, volume-domain, and device-hold behavior.
- DSD tests prove that Shared on an integer-capable device offers Switch to Exclusive and retry, Exclusive on an AUHAL-only device refuses without a futile switch action, and no DoP bytes reach either AUHAL path.
- Visual comparison against the approved Pencil frames for the Devices page and output popover, including resolved-by-default and pinned states plus an Exclusive device using each backend.
- Matrix Mini-i Pro 4: Auto and pinned Exclusive use the integer backend, PCM reports `BIT-PERFECT`, DoP locks as DSD64, pause still holds the device, and feature 77 unlock still releases it. Pinned Shared releases the hog, allows another app to play, and refuses DoP safely.

## Implementation Notes

Code-level pointers from the 2026-09-03 read of `main`, for the phase 2–3 missions; the Scope above is the contract.

- `StoredOutputMode` (`backend/settings.rs`) drops `BitPerfect`; `Exclusive` takes `#[serde(alias = "bitPerfect")]` so existing files load and the next settings write normalizes — the same shape as `migrate_legacy_mode` for the old `exclusiveMode` bool.
- `engine_kind_for_output_mode(mode)` (`playback/logic.rs`) becomes a resolver over mode + capabilities: Exclusive → the integer kind (`EngineKind::Integer`, today `BitPerfect`) only when feature 78 stage 2's `integer_wire_formats` holds and `DeviceTransport::supports_bit_perfect`; otherwise `Universal { exclusive_mode: true }`. `automatic_output_mode` / `automatic_stored_output_mode` lose the Bit-perfect arm; `output_mode_for_engine_kind` maps both exclusive kinds to Exclusive.
- `Playback` gains `resolved_engine_kind: EngineKind` beside `playback_output_mode`: set at resolution, confirmed by `OutputDeviceChanged { kind }`, set to shared by `ExclusiveModeFallback`. The DoP gate (`dsd_playback_error*`, `stop_before_unsafe_dsd_output_change`, `refuse_unsafe_dsd_command`) keys on it instead of `output_mode == BitPerfect`; the `SwitchToBitPerfect` toast action and `switch_to_bit_perfect_and_retry` become the Exclusive switch.
- `signal_path_verdict` already reads `bit_perfect_active`, which the engine sets only while the integer backend runs; after this feature nothing prints Bit-perfect from a stored mode.
- Sequencing with feature 78: stage 2 and phase 2 here rewrite the same resolver in the same files (`hal.rs`, `device.rs`, `settings.rs`, `logic.rs`) — run one after the other, and whichever runs first carries the predicate.

## Log

- **2026-09-03** — Shipped in v0.3.2 on Jason's go; the Shared switch was hardware-checked the same day (the Matrix goes back to the system as soon as Shared is selected). Spec archived, #81 closed.
- **2026-09-03** — Phase 3 merged as `3e17bb2` (PR #83, codex-crew mission; lead review). The surface now matches the Design Source: the popover card is the two segments alone (hugging, no label, no `AUTO`, no `Reset to Auto`), Devices rows keep the `Output mode` label plus `NO INTEGER PATH` on every gated device whether pinned or not, the selected segment is neutral (`bg_elevated` fill, no visible ring — a lead decision over the goal text, the frames have no stroke), and Exclusive carries the 10 px lock glyph on both surfaces. Removed with the affordances: `ResetDeviceOutputMode`, `reset_device_output_mode_to_auto`, `clear_mode`, `output_mode_is_automatic`, the `automatic` snapshot fields; `is_pinned` is test-only. Phase 4 docs landed in the same commit as this entry (engine doc probe-gate sentence, the volume-lock bug note, the 78 spec's gate-parity item, roadmap and indexes). Remaining before close: Jason's visual pass against frames `vH78z` / `XmoeK` and the Matrix smoke pass in Verification.
- **2026-09-03** — Phase 2 merged as `995e426` (PR #82, codex-crew mission; lead review under Jason's drive-mode authorization, hardware pass deferred to the end). Landed: `StoredOutputMode::{Shared, Exclusive}` with the `bitPerfect` alias normalizing on write, `integer_wire_formats` on both capability records, `resolve_engine_kind` over mode + capabilities, `EngineKind::Integer`, `resolved_engine_kind` on `Playback` owned by confirmation events (install, `OutputDeviceChanged`, `ExclusiveModeFallback`) with in-flight requests treated conservatively by the DoP gate, the two refusal states with their approved copy, the third segment removed, `docs/arch/{pulse-app,pulse-engine}.md` reworded. Deferred to phase 3: the lock glyph, the popover card reduced to the two segments, `AUTO` tag and `Reset to Auto` removal, the neutral active-segment treatment. Two notes from review: a DSD play on a device whose capabilities are unknown and whose engine kind is not `Integer` now refuses as `DeviceNotVerified` (previously `NeedsBitPerfect`); after a failed mode change while playing, the stale pending kind keeps DoP refused until the next confirmation — safe side, self-clearing.
- **2026-09-03** — Pencil pass approved by Jason and committed as `1efc40d`; spec filed against issue #81.
