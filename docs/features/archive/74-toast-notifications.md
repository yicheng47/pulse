# Toast Notifications

Feature 74 · P2 · GitHub issue [#74](https://github.com/yicheng47/pulse/issues/74). Decided by Jason, 2026-09-01 during the feature 71 phase-3 Pencil pass: the DSD refusal wants a toast, not the banner — and the audit found three more surfaces that want the same primitive.

## Motivation

Pulse has two error surfaces and both are wrong for one-shot verdicts. The notice banner above the playback row is persistent — right for standing conditions (dropouts, device failure) that need recovery buttons, heavy for "couldn't play that track": it lingers until dismissed and reflows the player. The library surface's error is an absolute bottom-right red box (`surfaces/library/mod.rs`) — a proto-toast with no title, no dismiss, no timer, cleared only by unrelated state changes. A toast is the right shape for a verdict about an action the user just took: it appears where the eye already is, says its piece, and leaves.

## Design Source

`design/pulse-desktop.pen`, approved 2026-09-01:

- **Components Board → Toasts Row** (`istmO`): `Toast / Error` (`x4nPhV`), `Toast / Error + Action` (`l1ulu`), `Toast / Warning` (`DqRnZ`). Flat — `$bg-elevated`, `$border-strong` 1px, `$radius-lg`, no shadow; 400px wide; danger/warning icon, 13/600 title + 12 secondary body, dismiss ×; action variant carries an accent-soft button in the copy column.
- **Spec — DSD Gate & Toasts** board (`GGd35`): behavior rules, refusal copy, migration audit.
- **Library / Tracks — Unplayable DSD** screen (`V5k1T`): the toast in context above the player bar.

## Scope

- **Primitive**: a toast layer rendered above all surfaces, anchored bottom-right — 16px above the playback row, 20px from the window edge. Newest on top; at most 3 visible, older ones drop off. Variants: Error, Error + Action, Warning.
- **Timing**: plain toasts auto-dismiss after 6s, timer pauses while hovered. A toast with an action has no timer — it stays until acted on or dismissed. The × always dismisses; clicking elsewhere never does.
- **Migration — one-shot verdicts move to toasts**:
  - Library's floating red box (playlist create/rename/delete/append failures, store-busy, reload errors) → `Toast / Error`, same position, real anatomy.
  - `PlaybackNotice::Stopped` (all emitters — decode failures and the DSD gate) → `Toast / Error`, with the action variant where a fix exists.
  - `PlaybackNotice::Skip` (missing/unreadable file, playback continues) → `Toast / Warning`.
  - `PlaybackNotice::ExclusiveFallback` → `Toast / Warning`; the signal path popover shows the standing truth.
- **Banner keeps** `Dropouts` and `DeviceFailure` — standing conditions with Try again / Choose output.

## Non-Goals

- No success/info toasts — errors and warnings only until a need shows up.
- No migration of contextual errors: output-popover device messages, Settings update FAILED status line, storage offline/scan-failure states, dialog validation all stay inline.
- No toast persistence across restarts, no notification center/history.
- No click-elsewhere or timeout dismissal for action toasts.

## Implementation Phases

1. **Toast primitive**: toast state + layer in the app shell (above surfaces, below nothing), the three variants per the design, stacking, hover-pause timer, dismiss.
2. **Migration**: retarget the library error box and the three banner variants; delete the dead banner arms (`Skip`, `Stopped`, `ExclusiveFallback`) and the red-box rendering; banner logic reduces to `Dropouts`/`DeviceFailure`.

## Verification

- `make verify` green.
- Manual: trigger each variant (delete a playlist with the store busy → error toast; play a missing file → warning skip toast, playback continues; force exclusive fallback → warning toast; unplayable DSD → action toast). Hover pauses the 6s timer; three toasts stack newest-on-top; × dismisses; action toasts outlive the timer.
- Banner still appears for sustained dropouts and device failure with working recovery buttons.
