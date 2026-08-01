# Feature Specs

Product features that are not part of a numbered roadmap stage. Roadmap stages live in [`docs/impls/`](../impls/); this folder is for features discovered outside that sequence — usually during acceptance passes.

Each spec is `{number}-{slug}.md` and states motivation, scope, non-goals, phases, and verification.

- [`01-volume-control.md`](01-volume-control.md) - P1, issue #17. Software gain in the engine's float32 render path with a unity default, mute, and a playback-row slider. Hardware/device volume and a fixed-output purist mode are explicitly future work. Blocked on a Pencil pass for the playback bar.
