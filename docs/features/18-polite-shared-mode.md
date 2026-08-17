# Polite Shared Mode

> Feature 18 · P2 · GitHub issue [#51](https://github.com/yicheng47/pulse/issues/51). Shared (non-exclusive) playback still switches the device's nominal sample rate per track — a system-wide side effect on a device other apps are using.

## Motivation

`Engine::set_format` always switches the device nominal rate and requests an integer physical format, hog or no hog (`engine.rs:69`). The nominal rate is device-global state: changing it on a shared device interrupts or resamples every other client — a video call, YouTube, system sounds — every time Pulse crosses a rate boundary in the queue. Today's shared mode therefore gives up exclusivity but keeps the side effects, which is the most surprising possible combination.

It also makes shared mode less capable than it should be: the rate switch is mandatory and errors when the device doesn't support the track's native rate, so hi-res tracks fail outright on rate-limited outputs (Bluetooth devices being the common case) even though Core Audio could resample them fine.

Shared mode should be the device-friendly path; native-rate switching belongs to exclusive mode.

## Scope

- Exclusive mode off: skip nominal-rate and physical-format reconfiguration entirely. Open the AUHAL sink with the track's client format and let the AUHAL converter resample to the device's current rate.
- Exclusive mode on: unchanged — hog mode, native rate, integer physical format.
- Settings copy states the tradeoff so the mode choice is informed: exclusive = native rate and device lock; shared = system-friendly, resampled when rates differ.
- Any UI that implies native-rate output (format line, device capability messaging) must not imply it in shared mode.

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
- Manual: exclusive off, play YouTube audio simultaneously, alternate 44.1 kHz and 96 kHz tracks in Pulse — the other app's audio never glitches and the device rate in Audio MIDI Setup never changes. A 96 kHz track now plays on a Bluetooth output in shared mode. Exclusive on: native-rate switching still works on the Matrix DAC.
