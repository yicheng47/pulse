# Stage 5 — acceptance

The hardware gate for any bit-perfect wording in the UI, run on the Matrix Mini-i Pro 4.

## Payload prep

Source material is `~/qobuz/04 - 暧昧.dff` (DSD64, ~4½ min). A dev-only script (outside the product — Pulse stays PCM-only inside) parses the DFF stream, DoP-packs it (16 DSD bits per 24-bit sample at 176.4kHz, `0x05`/`0xFA` markers in the top byte), writes 24-bit/176.4k WAV, and `flac` encodes it losslessly. To Pulse the result is an ordinary PCM FLAC. The packer logic later graduates into [feature 33](../../features/33-dsd-over-dop.md)'s decoder.

## The DoP test

The DoP FLAC through the integer engine → the Matrix display flips to DSD64 (any bit corruption destroys the markers, so the flip is the proof). Control: the same file through the AUHAL engine at unity — if the display stays at 176.4kHz PCM, that demonstrates the exact gap this engine closes.

Repeat plain PCM at 44.1/16 and 96/24 watching the rate display. Record all results in feature 32's spec; **only a pass permits any bit-perfect wording in the UI.**

## Teardown and pause checks

- Pause holds the device (stage 3): while paused, the hog is still held (another app cannot open the device) and resume is instant with no relay click.
- Formats restore only after `Stop`, device/engine switch, or quit — that is when the Audio MIDI Setup check applies: after quitting Pulse, the device shows its prior formats restored.

## Verification

- Results recorded in feature 32's spec; pass/fail per test.
- Manual sweep from the feature specs: engine switch restarts cleanly, disabled control on speakers/Bluetooth, volume slider behavior per feature 31, pause clicks expected and documented.
