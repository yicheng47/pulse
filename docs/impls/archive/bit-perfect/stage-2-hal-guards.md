# Stage 2 — hal guards

- `FormatRestoreGuard` (RAII, `HogGuard` style): captures physical + virtual + mixing at construction. `restore` returns every readback-verified property failure for explicit teardown; `Drop` is the silent last-resort path and cannot restore twice. mpv restores formats on teardown; Pulse must too — the device outlives the app, and leaving a DAC in a 192k integer state with mixing off breaks every other app. Held by the integer engine for its lifetime — and the backend is retained across pause (stage 3), so the guard spans pauses and fires only on `Stop`, device/engine switch, or quit.
- One shared two-second deadline bounds the complete best-effort restore: every physical, virtual, and present mixing property is attempted even after the deadline or an earlier failure, but refused writes cannot stack one timeout per property. Mixing writes now use the same readback-verification contract as stream-format writes.
- This stage lands the guard unwired. Stage 3 constructs it once when opening the integer engine and consumes its explicit restore path during backend release.
- Unit-testable pieces (format matching, flag checks) get tests; property calls themselves are hardware-only.

## Verification

- `make verify` green; format-matching and flag-check tests in place.
