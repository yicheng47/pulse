# Polite Shared Mode

> Feature 18 · P1 · GitHub issue [#51](https://github.com/yicheng47/pulse-src/issues/51). Shared (non-exclusive) playback still switches the device's nominal sample rate per track — a system-wide side effect on a device other apps are using, and a hard failure on devices that can't do the track's rate.

## Motivation

`Engine::set_format` always switches the device nominal rate and requests an integer physical format, hog or no hog (`engine.rs:69`). The nominal rate is device-global state: changing it on a shared device interrupts or resamples every other client — a video call, YouTube, system sounds — every time Pulse crosses a rate boundary in the queue. Today's shared mode therefore gives up exclusivity but keeps the side effects, which is the most surprising possible combination.

It also makes the mandatory rate switch a hard gate, and the gate is worse than "hi-res fails": AirPods Pro advertise only 48 kHz nominal, so even a 44.1 kHz / 16-bit CD-rate track fails at `set_nominal_sample_rate` (`hal.rs:237`) before playback starts, in both modes. Confirmed on hardware 2026-08-17 — essentially the entire library is unplayable on the Bluetooth device class, which is why this is P1: Core Audio could resample all of it fine.

Shared mode should be the device-friendly path; native-rate switching belongs to exclusive mode.

## Scope

- Exclusive mode off: skip nominal-rate and physical-format reconfiguration entirely. Open the AUHAL sink with the track's client format and let the AUHAL converter resample to the device's current rate.
- Exclusive mode on: unchanged — hog mode, native rate, integer physical format.
- Settings copy states the tradeoff so the mode choice is informed: exclusive = native rate and device lock; shared = system-friendly, resampled when rates differ.
- Any UI that implies native-rate output (format line, device capability messaging) must not imply it in shared mode.
- Split `EngineError::NoMatchingFormat`: it renders as "no physical format matches" but is raised both for an unsupported nominal rate (`hal.rs:238`) and for a genuinely absent physical format (`hal.rs:282`), so the AirPods failure today surfaces the wrong diagnosis.

## Non-Goals

- Pulse-side resampling, SRC quality settings, or per-device rate policy — Core Audio owns the conversion.
- Changing exclusive-mode or hog semantics.
- Automatic mode selection by device type (per-device persistence is feature 08).

## Implementation Phases

1. Confirm the semantics in this spec and update the Settings copy.
2. Engine: gate `set_nominal_sample_rate` / `set_matching_physical_format` on exclusive mode; validate the AUHAL SRC path (44.1 kHz track while the device sits at 48 kHz) plays correctly.
3. Tests plus manual validation against a concurrent shared-device client.

## Verification

- Engine/controller tests: a shared-mode start never touches the HAL rate/format setters; the exclusive path is unchanged.
- `make verify` is green.
- Manual: exclusive off, play YouTube audio simultaneously, alternate 44.1 kHz and 96 kHz tracks in Pulse — the other app's audio never glitches and the device rate in Audio MIDI Setup never changes. A 44.1 kHz CD-rate track and a 96 kHz track both play on AirPods Pro in shared mode (today the 44.1 kHz track fails with "no physical format matches"). Exclusive on: native-rate switching still works on the Matrix DAC.
