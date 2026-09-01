# Volume Transparency

P2. Successor to the disclosure half of feature 01's deferred work ("hardware/device volume and a fixed-output purist mode remain future work").

## Motivation

Pulse's volume slider drives one of two very different mechanisms and never says which. When the engine holds the hog on a device with a settable volume scalar, the slider writes `kAudioDevicePropertyVolumeScalar` — the DAC's own attenuator — and the sample path is pinned to unity (`Engine::set_volume`). Everywhere else it multiplies samples through the software gain stage (`gain.rs`, cubic curve). The user-facing consequences differ (hardware volume keeps full-scale samples on the wire; software gain at unity is value-exact, below unity it is not), but the UI presents both as the same anonymous slider.

Decision (Jason, 2026-08-31): disclose, don't restrict. No locked slider, no purist cage — show the user which domain is active and whether the current path is value-transparent, and let them choose. Wording must respect the honest claim in `docs/arch/pulse-engine.md`: Pulse controls its own path up to the AUHAL client boundary and makes no bit-perfect claim past it (Core Audio owns the f32 → physical conversion).

## Scope

- Engine/backend surfaces which volume domain is active (hardware/device vs software gain) and whether the software path is at unity. The engine already knows both (`hardware_volume.is_some()`, gain level); this is plumbing, not new audio behavior.
- A signal-path disclosure in the UI: the volume control (popover or playback row) names its domain — e.g. "Device volume" vs "Software volume" — and shows a transparent-path state when samples leave Pulse untouched (unity gain or hardware volume active, no engine-side processing).
- The Devices page states per-device whether hardware volume is available while hogged, alongside the existing capability line.
- Copy reviewed against the honest-claim rule: "samples leave Pulse untouched", never "bit-perfect to the DAC".

## Non-Goals

- No fixed-output/locked-volume mode in this feature (remains a possible later opt-in, per the same decision).
- No claims about Core Audio's conversion or the wire past the client boundary; no hardware verification (DoP/bit-test) tooling.
- No change to volume behavior itself — dispatch, cubic curve, fades, and the unity pin stay exactly as they are.
- Transient fade ramps (pause/resume/seek, ~10 ms) do not demote the indicator; it reflects steady state.

## Findings

- **2026-08-31, first bit-perfect hardware run**: the Matrix Mini-i Pro 4 exposes **no settable volume scalar while hogged** — bit-perfect on the Matrix is the fixed-at-100% camp (`FIXED` domain). On current main the playback-row slider still slides and silently no-ops in that mode — exactly the dead-control state this feature's UI phase removes (slider disabled at 100%, `FIXED` label, reason disclosed). The design for the domain label and disabled state landed in the 2026-08-31 Pencil pass (volume popover + Spec — Signal Path board).

## Design (2026-08-31 Pencil pass, approved — `design/pulse-desktop.pen`: Signal Path Popover, Spec — Signal Path, Volume Popover, Spec — Volume States addendum)

- **Signal Path popover**: opens from the quality badge in the playback row (queue-popover dismissal pattern). Anatomy: header row — "SIGNAL PATH" (mono, 10px, letter-spaced, `$text-muted`) left, verdict pill right; five chain rows (Source, Decode, Volume, Engine, Output), each a 5px status dot + stage name (sans 12 semibold `$text-primary`) left and a mono 10 `$text-secondary` detail right, with thin 8px vertical connectors between rows; divider; sans 10 `$text-muted` footer note.
- **Verdicts**: `BIT-PERFECT` (`$quality` on `$quality-soft`, `$quality-border`) only on the integer engine with hog held and the integer virtual format confirmed by readback — the state stage 4 already plumbs; footer "Source integers reach the DAC unmodified." `TRANSPARENT` (`$primary`) on Universal with untouched samples (unity software gain, or device volume) — dots past the Engine row muted, footer "Samples leave Pulse untouched. Core Audio owns the conversion past the client boundary." `PROCESSED` (`$warning`) when software gain is below unity — the Volume row and its dot flagged, footer points at 100%. Transient fade ramps (~10 ms) never demote the verdict; it reflects steady state.
- **Volume popover**: a domain label at the foot (mono 8, `$text-muted`) — `DEVICE` (hardware volume while hogged), `SOFTWARE` (engine gain), `FIXED` (bit-perfect without hardware volume). The % readout renders `$quality` on a transparent path.
- **Slider**: in the `FIXED` state the playback-row slider and popover slider are disabled, pinned at 100% — no slides that do nothing.
- **Devices page**: a device's class line appends "Device volume" when hardware volume is available while hogged.
- Release note: the `BIT-PERFECT` verdict state is implemented with this feature, but the v0.3.0 release (and any public bit-perfect wording) stays gated on the stage 5 DoP pass.

## Implementation Phases

1. **Backend**: expose the active volume domain and unity state from the engine to the app store (extend the existing volume/`HardwareVolume` plumbing; no new engine behavior). Unit-test the state transitions: hog acquired with settable scalar → hardware; fallback → software; slider at 100% → transparent, below → not.
2. **Design**: Pencil pass for the disclosure — volume popover label + transparent-path state, and the Devices page line. Small, but it is user-facing copy on an audiophile-sensitive claim, so it gets a design pass first.
3. **UI**: implement per the design; copy review against `docs/arch/pulse-engine.md`.

## Verification

- `make verify` green; backend state-transition tests cover the four combinations (exclusive±settable scalar, unity±).
- Manual: built-in speakers exclusive (hardware volume, per the archived `builtin-speakers-exclusive-volume` bug) shows "Device volume"; shared mode shows "Software volume"; slider at 100% shows the transparent state, dragging below clears it; Matrix DAC checked in both modes to record which camp its USB volume falls in.
- Copy check: no wording claims bit-perfectness beyond Pulse's boundary.
