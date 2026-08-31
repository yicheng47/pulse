# Stage 4 — app (`pulse-app`)

Entry gate: stage 3 merged, and the Pencil design pass for the Devices-page Engine control approved by Jason **before** this stage's mission starts.

- `settings.rs`: `StoredDevicePreferences` gains `engine: Option<StoredEngineKind>` (serde, `None` = Universal), accessors beside `exclusive_mode_override`.
- Capability plumbing: the probe today collapses to `(max_bits_per_channel, max_sample_rate)` (`hal.rs::output_device_capabilities`) and discards the format list. It must additionally report the gating signal, and `StoredDeviceCapabilities` gains the matching field; stored entries without it re-probe on next sight of the device.
- **The gating criterion is provisional until stage 1 reports.** The spec's "no integer physical formats" test likely does not gate built-in speakers — they typically do report integer physical formats; what they refuse (if anything) is the integer *virtual* format, and that can only be probed under hog, which is too intrusive for a background capability probe. If stage 1 confirms, gate on transport type (built-in/Bluetooth classes excluded) combined with integer physical formats present, and let start-time failure with a clear error cover the remainder.
- Devices page: an Engine control per device row — enabled only when the stored gating signal allows it; disabled state carries the capability note. Per the design pass, which also decides the existing Exclusive control's interplay when Engine = Bit-perfect: hog is mandatory, so exclusive is implied — lock the control on or hide it, but don't leave a dead toggle.
- Session/app_store: thread the kind through boot and `SetOutputDevice`; engine change on the active device restarts playback.
- Feature 31 indicator: "bit-perfect" state = integer engine + hog held + virtual format confirmed. If 31 isn't built yet, land the state plumbing and let 31 render it.

## Verification

- `make verify` green; settings round-trip and capability re-probe covered by tests.
- Manual: engine switch restarts cleanly; speakers/Bluetooth show the disabled control; volume slider hardware-routed or disabled.
