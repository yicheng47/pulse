---
name: bug
description: Use when reporting, triaging, listing, or closing Pulse bug reports, especially bugs involving playback, Core Audio HAL, realtime audio, GPUI views, or UI behavior.
---

# Bug Reporting

Manage Pulse bugs with enough technical context to make them actionable. Bugs live as notes in `docs/bugs/` and are scheduled in `docs/roadmap.md`.

## Workflow

1. Clarify what failed, what was expected, and how to reproduce it.
2. Inspect the relevant code before filing or summarizing the bug.
3. Assign a priority using the rubric below.
4. File it as a note: `docs/bugs/{slug}.md` using the template below with the priority in the header line, add a line to `docs/bugs/README.md`, and add a row to the right milestone in `docs/roadmap.md`. Do not create GitHub issues in this repo (tracking moved to local docs on 2026-08-29). Customer reports arrive on the public `yicheng47/pulse` repo: reply and close there with `--repo yicheng47/pulse`, and carry the engineering work into a note here with a link back. When a bug is fixed, move its note to `docs/bugs/archive/` with a line naming the commit.
5. Include relevant file paths, environment details, and verification status.

## Note Template

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
