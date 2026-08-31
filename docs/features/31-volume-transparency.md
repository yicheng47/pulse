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

## Implementation Phases

1. **Backend**: expose the active volume domain and unity state from the engine to the app store (extend the existing volume/`HardwareVolume` plumbing; no new engine behavior). Unit-test the state transitions: hog acquired with settable scalar → hardware; fallback → software; slider at 100% → transparent, below → not.
2. **Design**: Pencil pass for the disclosure — volume popover label + transparent-path state, and the Devices page line. Small, but it is user-facing copy on an audiophile-sensitive claim, so it gets a design pass first.
3. **UI**: implement per the design; copy review against `docs/arch/pulse-engine.md`.

## Verification

- `make verify` green; backend state-transition tests cover the four combinations (exclusive±settable scalar, unity±).
- Manual: built-in speakers exclusive (hardware volume, per the archived `builtin-speakers-exclusive-volume` bug) shows "Device volume"; shared mode shows "Software volume"; slider at 100% shows the transparent state, dragging below clears it; Matrix DAC checked in both modes to record which camp its USB volume falls in.
- Copy check: no wording claims bit-perfectness beyond Pulse's boundary.
