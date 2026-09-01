# Stage 4 — app (`pulse-app`)

Entry gate: stage 3 merged, and the Pencil design pass — done 2026-08-31 (Devices-page Output mode control, popover Mode row, Spec — Engine Control board; the feature 08 exclusive-control board was deleted as superseded) — approved and saved by Jason **before** this stage's mission starts.

- `settings.rs`: `StoredDevicePreferences` replaces `exclusive_mode_override` with one `mode: Option<StoredOutputMode>` (`Shared | Exclusive | BitPerfect`; serde, `None` = **Auto**) — the one-axis Output mode control from the design pass (Jason, 2026-08-31, superseding feature 08's separate toggle). One-shot migration: a stored exclusive override becomes a pinned Shared/Exclusive mode; absent stays Auto.
- **Auto resolution ladder**: `None` resolves Bit-perfect when the transport + format gate passes; Exclusive when the device has integer physical formats but a gated transport or refused virtual; Shared for float-only — feature 08's probe defaults carried over per-mode. An explicit stored value pins (AUTO tag drops, `Reset to Auto` shown). Resolution lives app-side — the controller always receives a resolved `EngineKind` (Shared → `Universal{shared}`, Exclusive → `Universal{exclusive}`, Bit-perfect → `BitPerfect`). Unit-test the ladder × pin × migration matrix.
- Capability plumbing: the probe today collapses to `(max_bits_per_channel, max_sample_rate)` (`hal.rs::output_device_capabilities`) and discards the format list. It must additionally report the gating signal, and `StoredDeviceCapabilities` gains the matching field; stored entries without it re-probe on next sight of the device.
- **Gating criterion, resolved by stage 1**: Bit-perfect requires signed-integer physical formats (`max_bits_per_channel: Some(_)`) AND a transport that is neither display class (DisplayPort/HDMI) nor Bluetooth — the DELLs advertise integer physical formats yet refuse every integer virtual write, built-in falls out by float-only formats. Start-time virtual-format failure with a clear error is the backstop, not the display gate.
- Devices page + output-device popover: the Output mode control per the design (`Spec — Engine Control` board in `design/pulse-desktop.pen`): 3 segments in each device card's settings line (popover label: `Mode`), AUTO tag / `Reset to Auto` pinned state, Bit-perfect segment disabled with the `NO INTEGER PATH` tag on gated devices.
- Session/app_store: thread the resolved kind through boot and `SetOutputDevice`; mode change on the active device restarts playback.
- Feature 31 indicator: "bit-perfect" state = integer engine + hog held + virtual format confirmed. If 31 isn't built yet, land the state plumbing and let 31 render it.

## Verification

- `make verify` green; settings round-trip and capability re-probe covered by tests.
- Manual: engine switch restarts cleanly; speakers/Bluetooth show the disabled control; volume slider hardware-routed or disabled.
