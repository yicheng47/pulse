---
name: bug
description: Use when reporting, triaging, listing, or closing Pulse bug reports, especially bugs involving playback, Core Audio HAL, realtime audio, Tauri commands, or UI behavior.
---

# Bug Reporting

Manage Pulse bugs with enough technical context to make them actionable.

## Workflow

1. Clarify what failed, what was expected, and how to reproduce it.
2. Inspect the relevant code before filing or summarizing the bug.
3. Assign a priority using the rubric below.
4. If creating a GitHub issue, use labels `bug` and exactly one of `P0`, `P1`, `P2`, `P3`.
5. Include relevant file paths, environment details, and verification status.

## Issue Template

```markdown
## Description
<what is wrong>

## Expected Behavior
<what should happen>

## Steps To Reproduce
<commands, files, device, or UI steps>

## Relevant Code
<file paths and short notes>

## Environment
- OS:
- Device / DAC:
- Input file format:
- Pulse version:

## Verification
<what was checked>
```

## Priority

- `P0` - Data loss, security issue, crash on startup, or a regression that blocks engine validation.
- `P1` - Common user-visible failure, playback correctness issue, or broken core workflow.
- `P2` - Annoying bug with a workaround, rare path, or prominent cosmetic issue.
- `P3` - Edge case, theoretical issue, or low-impact polish.

For audio bugs, err on the side of higher priority when the issue may affect bit-perfect claims.

## Project Rules

- Do not commit, push, or create PRs unless the user explicitly asks.
- Do not suggest libmpv, FFmpeg, GPL dependencies, DSD, video, or streaming integrations as bug fixes.
- Realtime audio bugs must respect the IOProc constraints: no allocation, no locks, no syscalls on the callback thread.
