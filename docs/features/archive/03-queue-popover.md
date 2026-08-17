# Queue Popover

> Feature 03 · P1. Gives the playback bar's queue button a hit target. The button has shown a live remaining-count badge since stage 12 but was deliberately inert because no queue surface existed; the drawer was designed nowhere and deferred. It is designed now.

## Motivation

The queue is real (stage 12: transient app-side queue, auto-advance, badge) but invisible. You cannot see what plays next, jump ahead, drop a track you regret queueing, or clear the tail — the badge teases state the UI never shows. This is the last inert control in the playback bar apart from the deliberately-inert Shuffle/Repeat.

## Design source

`design/pulse-desktop.pen`, screen `Playback — Queue Popover` (`LjEHI`), popover node `qfVOF`, committed on this branch (`b580f75`). Read via the `pencil` MCP only. 376-wide panel in the Output Device Popover's chrome, opening upward, seated just above the playback bar, anchored to the queue button (`cOeyi` inside the `qKkw7` component). The trigger's `list-music` icon renders accent while open.

## Scope

- **Toggle** on the queue button click; dismissal via the stage-8 popover pattern (outside click, Escape) plus the artist picker's press-closed guard so clicking the open trigger closes rather than closes-then-reopens.
- **Header**: "Queue" title, mono meta `N UP NEXT · M MIN` (sum of remaining durations, unknown durations excluded from the minutes), and **Clear** — drops all up-next entries, keeps the current track playing, badge hides at zero.
- **NOW PLAYING block**: current track title, `artist · album`, live `elapsed / duration` in `quality` lime, `audio-lines` accent icon. Absent entirely when nothing is playing.
- **UP NEXT list**: numbered rows — queue position, title, `artist · album`, duration. Row click **jumps**: the queue index moves to that entry and it plays; entries are not removed by a jump (Previous still walks back). Hover shows `bg-muted` fill and an ✕ that removes just that entry, renumbering the rest. Removing entries never touches the current track.
- **Empty state**: with a current track but no upcoming, the UP NEXT section shows a single muted "Nothing up next" line; with no queue at all the popover still opens showing only that line (text-only, no new design needed).
- **Overflow**: the list scrolls within the popover's designed max height (~540px) rather than growing past the top bar.
- **Badge**: unchanged binding (remaining count, hidden at zero) — it must stay correct through jump/remove/clear.

## Non-Goals

- Reordering inside the popover (drag or move up/down) — the queue is transient; reorder lives in playlists.
- Queue persistence across launches, save-queue-as-playlist, history section.
- Shuffle/Repeat (still deliberately inert), a full-page queue view.

## Implementation Shape

Queue mutations (`jump_to`, `remove_at`, `clear_upcoming`) belong in `crates/pulse-app/src/queue.rs` beside the existing advance logic, unit-tested there — including interaction with the stage-13 attempt-ordinal correlation and failure-skip marks (a jump to a marked-missing entry follows the existing skip-and-report path). The popover itself follows the artist-picker/output-popover component shape; row and header styling from `components.rs` where pieces exist.

## Verification

- `make verify` green (currently 130 passing + 2 ignored). New queue view-model tests: jump moves index without dropping entries, remove renumbers and fixes the badge count, clear keeps the current track, jump-to-missing skips per stage-13 rules, minutes meta excludes unknown durations.
- Manual (GUI, Jason): open/close/press-closed guard, jump from the middle, remove a row, clear, badge correctness throughout, popover over a long queue scrolls, live time ticks in the NOW PLAYING block.
