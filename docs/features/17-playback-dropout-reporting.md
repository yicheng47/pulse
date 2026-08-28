# Playback Dropout Reporting

> Feature 17 · P2 · GitHub issue [#50](https://github.com/yicheng47/pulse-src/issues/50). The realtime callback counts underrun bytes but nothing reads them; audible dropouts are invisible to the controller, the UI, and diagnostics.

## Motivation

When decode can't keep up with the device — NAS hiccup, slow disk, contended CPU — the render callback fills the shortfall with silence and increments `underrun_bytes` in `auhal.rs`. That atomic is write-only today: the user hears a stutter and the app has no record that anything happened. With the library living on a NAS, decode falling behind is the most likely real-world audio failure, and it is currently undiagnosable.

## Scope

- A read side for the existing counter: expose cumulative underrun frames from `AuhalSink` through `Engine` to the controller.
- A controller-level dropout signal — a `PlaybackEvent` on threshold crossing, or a counter carried on the periodic position event; the exact shape is chosen during implementation, but events stay observable facts.
- Exclude sink-startup priming from the count: the sink starts on an empty ring, so the first callbacks always emit silence and currently pollute the metric.
- App surfaces dropouts: at minimum a diagnostic count, and a notice when dropouts are sustained.

## Non-Goals

- Auto-pause, buffering-state UI, or adaptive ring sizing.
- Telemetry, log files, or any reporting infrastructure beyond the event surface.
- Callback changes beyond what counting already does — the callback stays allocation- and lock-free.

## Implementation Phases

1. Engine: startup-priming exclusion and an underrun read path on the sink and `Engine`.
2. Controller: dropout tracking in the pump/position reporting and the event surface, with fake-backend tests.
3. App: surface the signal in the playback row (diagnostic count and/or sustained-dropout notice).

## Verification

- Unit tests: priming silence is not counted; steady-state underruns are; the event fires at the chosen threshold.
- Controller tests drive a fake backend that reports underruns.
- `make verify` is green.
- Manual: play from the NAS over a constrained link (or artificially slow decode in a debug build) and confirm dropouts register in the UI while the underrun-free path stays quiet.
