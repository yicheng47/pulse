# Engine Modularization and Review

Feature 93 · P2 · GitHub issue [#93](https://github.com/yicheng47/pulse/issues/93). Requested by Jason, 2026-09-05: split the engine's oversized files and complete a thorough code review before continuing playback work.

Accepted by Jason on 2026-09-06; commit, PR, and merge authorized. Implementation and independent review are complete, with `make verify` passing. The [review report](../../impls/93-engine-review.md) records pre-existing defects and hardware uncertainties separately; those follow-ups are not fixed by this refactor. Pending the next release.

## Motivation

At `963365f` (v0.3.3), `controller.rs` is 6,473 lines, roughly 4,600 of them tests. `hal.rs` is 1,560 lines and combines property FFI, format negotiation, ownership/restoration, hardware volume, and capability probing. `integer_engine.rs` (773 lines) and `decode_dsd.rs` (754 lines) also combine independently reviewable responsibilities. File boundaries should make ownership, sample handling, and transport behavior easier to inspect without changing the design.

The [feature 78 review](../../impls/78-integer-engine-review.md) is historical evidence, not a current whole-engine verdict: it predates subsequent fixes and focused on the integer path. This task reviews both output paths at the current source state, including the small modules that remain intact.

## Scope

### Mechanical module splits

- `controller/`: `mod.rs` for the public handle, spawning, subscriptions, and shutdown; `backend.rs` for backend adapters and release handles; `decoder.rs` for decoder adapters; `worker.rs` for the existing worker state machine and its state; `tests/` grouped around transport, output, gapless, dropouts, and shutdown with shared fakes. Keep one `Worker`; dividing its state into new collaborating objects is unnecessary.
- `hal/`: a stable facade with focused files for property access, formats/rates, ownership/restoration, hardware volume, and capability probing. Preserve existing `hal::` paths, including those used by `examples/integer_probe.rs`, through re-exports at their existing visibility. Keep format restore state, explicit restore, and drop semantics together.
- `integer_engine/`: extract integer packing and its byte-exact tests into `packing.rs`, and device resource ownership/release synchronization into `release.rs`; keep the engine lifecycle and source-dependent format selection together. Preserve the resource field order and explicit teardown order.
- `decode_dsd/`: extract DSF and DFF container parsing into `dsf.rs` and `dff.rs`; keep the shared decoder, seek/marker state, and DoP output flow together. Retain small shared binary-reading helpers without adding a parser framework.
- Keep `decode.rs`, `auhal_engine.rs`, `gain.rs`, `raw_sink.rs`, `auhal.rs`, and the small public model modules intact unless a specific dependency of the agreed splits requires a mechanical import adjustment.
- Update current architecture documentation for the new layout and any verified stale descriptions. Historical feature/review records remain historical; do not bulk-rewrite their line references.

Names may follow existing repository conventions where a simpler equivalent fits. The objective is coherent responsibilities, not a file-size quota. Re-export existing APIs, keep new internals narrowly visible, preserve the existing test cases and fixtures, and keep any necessary fixture-path adjustment correct after the move.

### Whole-engine review

Review the post-split code in this order, tracing relevant callers and tests rather than checking only moved lines:

1. **Device ownership and realtime safety:** HAL property access and unsafe boundaries, hog acquisition/ownership, format/mixing restoration, partial-start failures, resource drop order, quit deadlines, cross-thread release synchronization, callback lifetime, ring producer/consumer ownership, and the absence of allocations, locks, syscalls, or waits in realtime callbacks.
2. **Sample correctness:** PCM decode and accurate seek, integer packing/width/alignment/sign handling, capability predicate versus format selection, DSF/DFF bounds and channel layouts, DoP marker/seek behavior, float conversion, and software/hardware/fixed volume domains. Audit the actual sample path and the limits of existing bit-perfect evidence.
3. **Transport and reporting:** the controller state machine, pause/resume/seek, same-format and format-changing boundaries, buffered gapless transitions, output/mode switches, frame-to-time accounting, dropout/stall detection, command/event ordering, panic/disconnection, fallback, and shutdown.

The coder writes the evidence-backed report at `docs/impls/93-engine-review.md`; the reviewer validates its findings and coverage after the normal implementation handoff. Each actionable finding needs a P0–P3 priority, current file/line pointers, a concrete trigger and consequence, existing test coverage or a focused reproduction, and a recommended next step. Distinguish refactor regressions, confirmed pre-existing defects, unresolved concerns, and hardware-only validation. State which areas were reviewed and which could not be verified; do not imply DAC validation from unit tests.

Fix regressions introduced by the split within this task. Record pre-existing defects as follow-up findings; do not silently mix their behavioral fixes into the file moves. Reference feature 89 for already-known multi-stream/mono work and the existing volume-domain lag note where relevant. Report other deferred findings in the final Runner handoff for Jason/the lead session to triage.

## Non-Goals

- Implementing features 76, 77, or 89; changing sample processing, callback behavior, transport semantics, output-mode policy, or public commands/events.
- New dependencies, crates, parser frameworks, backend abstractions, UI work, or unrelated cleanup.
- Automatically exercising hardware probes, opening the player, or changing a live device's format/ownership during the review.
- Commits, pushes, PRs, merges, new worktrees/checkouts, additional agents beyond the configured two-person crew, or edits to crew/model configuration.

## Implementation Phases

1. Record baseline branch/SHA and relevant checks. In the existing Pulse checkout, create `refactor/93-engine-modularization` from the current `main` before code edits. The spec/index/roadmap drafts supplied by the lead session are part of this task and must be preserved; unrelated changes require coordination.
2. Move one module at a time, preserving behavior, API visibility, resource order, tests, and fixtures; check the engine after each coherent move. Update architecture navigation for the final layout.
3. Perform the whole-engine review and write the report. The coder then explicitly hands the working-tree diff, validation evidence, and report to the reviewer through Runner. The reviewer remains idle until that handoff and reviews inline without spawning more agents.
4. Resolve refactor regressions and report inaccuracies through the normal coder/reviewer loop, complete final verification, and leave the changes uncommitted for Jason's review.

## Verification

- Before edits: `cargo test -p pulse-engine --all-targets`; record baseline test counts and any existing failures. This includes example tests without running the hardware probe itself.
- After coherent module moves: use engine compile/tests as appropriate. Compare the moved functions and tests against the baseline so accidentally dropped coverage, widened APIs, or semantic changes are visible.
- Final: `make verify` green. Any unrelated baseline failure must be identified with evidence rather than weakened/skipped tests or an unrelated fix. Reviewer reruns only checks needed by findings or unresolved concerns.
- Reviewer explicitly reports whether the refactor has any remaining must-fix issues. Pre-existing review findings remain visible even when the mechanical refactor is clean.
- Final Runner handoff: branch and baseline SHA, files/modules changed, check results, reviewer verdict, review-report path, prioritized findings/deferred work, and remaining hardware checks. Crew does not edit `docs/roadmap.md`, the feature index, or a global implementation log; the lead session owns those updates.
