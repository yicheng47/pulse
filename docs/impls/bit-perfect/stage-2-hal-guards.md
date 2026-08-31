# Stage 2 — hal guards

- `FormatRestoreGuard` (RAII, `HogGuard` style): captures physical + virtual + mixing at construction, restores in `Drop`. mpv restores formats on teardown; Pulse must too — the device outlives the app, and leaving a DAC in a 192k integer state with mixing off breaks every other app. Held by the integer engine for its lifetime, dropped on stop/teardown — and the backend is retained across pause (stage 3), so the guard spans pauses and fires only on `Stop`, device/engine switch, or quit.
- Unit-testable pieces (format matching, flag checks) get tests; property calls themselves are hardware-only.

## Verification

- `make verify` green; format-matching and flag-check tests in place.
