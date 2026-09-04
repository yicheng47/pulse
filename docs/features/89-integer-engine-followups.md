# Integer Engine Follow-ups

Feature 89 · P3 · GitHub issue [#89](https://github.com/yicheng47/pulse/issues/89). Opened 2026-09-04 when feature 78 closed, to hold the two plan items that shipped nothing yet.

## Motivation

Feature 78 shipped its three concrete fixes — in-place format change at track boundaries (stage 1), the integer-wire predicate behind the Exclusive resolver (stage 2, inside feature 81 phase 2), and honest hog errors with the 10 ms pump cadence (stage 4). Two items from [`impls/78-integer-engine-plan.md`](../impls/78-integer-engine-plan.md) have no trigger yet; they live here so #78 could close on what it delivered.

## Scope

- **Stream-indexed IOProc fill** (plan § Stage 3). `select_integer_format` already returns the stream id; the raw sink fills every buffer in the IOProc list and assumes the selected stream is the first. On a device with several output streams the selected stream should be filled by index and the others zeroed, with `kAudioDevicePropertyIOProcStreamUsage` set so unused streams stay silent. Parked: every device probed so far exposes one output stream, and the change is only testable against a real multi-stream device. Trigger: a device report showing more than one output stream.
- **Mono sources** (plan § Stage 5). `integer_candidate` requires the device format's channel count to equal the source's, so a mono file has no integer candidate on a stereo DAC and fails at start with `NoMatchingPhysicalFormat`. Decision pending, Jason's: refuse with a clear "mono needs Shared" error, or duplicate the channel in the packer (a sample copy, still no arithmetic — the bit-perfect claim would need a footnote).

## Non-Goals

- Anything already shipped under #78; DoP, DST, or native DSD changes; the `IntPacker` arithmetic.

## Implementation Phases

1. Mono: Jason's decision, then one small PR (refusal text, or the packer copy plus a review-record note).
2. Multi-stream: when a report arrives, the stage 3 section of the plan is the spec — stream index threaded into `RawSink::start`, the callback loop restructured, the two-buffer IOProc unit test.

## Verification

- Mono: a mono FLAC on the Matrix in Exclusive either refuses with the chosen text or plays as dual-mono with Signal Path still reporting the verdict honestly.
- Multi-stream: the reporting device plays on its selected stream with the others silent; `make verify` with the extended IOProc test.
