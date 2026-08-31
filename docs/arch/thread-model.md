# Thread Model

> Companion to [`pulse-engine.md`](pulse-engine.md) and [`tech-stack.md`](tech-stack.md). This doc records which thread owns what, how data crosses between them, and the standing decisions that shape the model. Decision date: 2026-08-07 (stage 13).

## 1. Overview

Pulse is one process with a thread per concern. Audio never passes through the UI thread at any point: the chain from decoder to DAC runs entirely on dedicated threads, so a stalled or frozen window cannot cause a dropout.

A separate playback process was considered and rejected (2026-08-07). To be precise about what the trade is: a split *would* buy real crash/hang/memory-corruption containment — Pulse does run native code in-process (Core Audio/AUHAL, SQLite, the gpui/Metal stack), and a fault there takes the whole app down today. What the split does **not** buy here is audio robustness against UI behavior: the realtime callback already runs at OS realtime priority in-process, Rust has no GC pauses to isolate, and the file-format attack surface (symphonia) is memory-safe Rust whose failures surface as errors or an unwound worker thread. Against that, a split costs IPC plumbing, helper-bundle signing, process lifecycle management, and — decisive on macOS — a second per-process TCC grant for network-volume access, the exact pain stage 12's packaging work exists to manage once. For a desktop player whose failure recovery is "reopen the app," the containment benefit does not pay for those costs. If a headless player or remote control ever becomes a product goal, the cut line is the `PlaybackController` command/event channels, which are already IPC-shaped; keeping `pulse-engine` UI-agnostic (standing `AGENTS.md` rule) is what keeps that future split cheap.

## 2. Thread Inventory

| Thread | Spawned by | Owns | Lifetime |
| --- | --- | --- | --- |
| Main / UI | gpui runtime | All views, view models, the `LibraryStore` (when not lent to a worker), all SQLite queries, playback event drain, device watchdog | Process |
| `pulse-playback-controller` | `PlaybackController::spawn` (`controller.rs`) | Command loop, symphonia decode, `Engine` (hog guard, ring producer), position/state bookkeeping | One per controller; joined on `Drop` via a shutdown flag |
| Core Audio realtime IO | AUHAL (`AuhalSink::start`) | The render callback: drains the ring buffer to the device | While the sink is playing |
| `pulse-library-open` | `LibraryView::begin_open_store` | `LibraryStore::open` and the v1→v2 metadata backfill, reporting `BackfillProgress` | Until the store opens or fails |
| `pulse-library-scan` | `LibraryView::start_scan` | The `LibraryStore` (moved in), file walk, metadata extraction, cover cache writes | Until scan completes or is cancelled |
| `pulse-album-delete` | `LibraryView::confirm_delete_album` | The `LibraryStore` (moved in), audio-file unlinks, row/playlist-entry deletion, cover-cache cleanup | Until the delete reports back |

## 3. Data Flow

```text
Main / UI thread
  │  PlaybackCommand (mpsc)                 ▲  PlaybackEvent (mpsc, drained every 16 ms)
  ▼                                         │
pulse-playback-controller ──────────────────┘
  │  decode (symphonia) → Engine::feed → float32 pack
  ▼
rtrb SPSC ring buffer (~4 s of audio)
  │
  ▼
Core Audio realtime IO thread → device (hogged, native rate)

Main / UI thread
  │  spawn                                  ▲  WorkerEvent (mpsc, drained every 50 ms)
  ▼                                         │
pulse-library-open / pulse-library-scan ────┤   (LibraryStore moves with the worker and returns in the event)
pulse-album-delete ─────────────────────────┘
```

The UI thread polls: playback events every 16 ms, worker events every 50 ms, and the active-device watchdog every ~2 s while playing (the engine has no mid-playback device-loss signal, so the row watches for it).

## 4. Rules and Invariants

- **Realtime callback: no allocation, no locks, no syscalls.** Audio crosses threads via the `rtrb` SPSC ring only (`AGENTS.md` standing rule). The ring holds roughly four seconds at the native rate, which is also the tolerance the UI thread gets for free.
- **`pulse-engine` stays UI-agnostic.** No gpui or `pulse-app` types in the engine crate; it must remain drivable headless. This is also the future process-split seam.
- **The `LibraryStore` has exactly one owner at a time.** rusqlite's `Connection` is `Send` but not `Sync`, so the store is *moved* to the scan worker and returned through the finished event; UI code holds `Option<LibraryStore>` and treats `None` as "a worker has it or boot hasn't finished." Never clone the connection; the one sanctioned second connection is the short-lived read fallback in `matching_tracks`.
- **All cross-thread communication is message passing.** `mpsc` channels for commands and events, `rtrb` for audio. No shared mutable state beyond the controller's internal `Arc<Mutex<subscribers>>`, which is never touched by the realtime callback.
- **SQLite page queries run synchronously on the UI thread — a deliberate decision (2026-08-07).** Measured on the live 253-track library: track page 0.06 ms, album page 0.16 ms, album count 0.08 ms; on a synthetic 50k-track / 5k-album library the grouped album page reaches ~25 ms plus ~14 ms for its count — at worst a frame or two of scroll hiccup at a scale ten times the real NAS library, and never audible, because audio does not depend on the UI thread. The trade accepted: a query-time loading state is impossible by construction (queries complete before the next paint), so the list-level loading UI only appears during app open. Escape hatches if a giant library ever makes this visible: an index or materialized albums table (stays synchronous), or moving just the page queries behind the existing worker-event pattern. What must *never* return to the UI thread is file I/O against music storage — that is why store open, backfill, and scans run on workers.
- **Worker panics do not kill the app, and do not strand it either.** The workspace uses default unwind panics, and every worker catches unwinds at its boundary: the playback controller clears its subscriber senders on exit so receivers observe `Disconnected` (`Playback engine disconnected.`), the library open worker reports a failed boot (with Retry), and a panicked scan worker — whose moved-in store is lost to the unwind — signals the UI to reopen the library. A panic therefore lands in a visible, recoverable state instead of a silent forever-`Opening` or dead event channel.
