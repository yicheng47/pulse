---
name: feature
description: Use when creating, organizing, or updating Pulse feature specs and feature issues under docs/features.
---

# Feature Management

Pulse feature specs should live in `docs/features/` once feature planning starts. Keep specs small, numbered, and tied to the current build order.

## Workflow

1. Clarify the feature's motivation, scope, and non-goals.
2. Check existing docs in `docs/product/`, `docs/arch/`, and `docs/features/`.
3. File the GitHub issue first (labels: `feature` + the priority; body summarizes motivation/target/non-goals and points at the spec path). **The issue number is the spec number** — the numbering scheme since 2026-09-01, so sequence gaps belong to bugs and PRs. Specs 01–34 predate the alignment (archive keeps them; the then-active five were renumbered 19→55, 20→56, 26→72, 33→71, 34→73).
4. Create `docs/features/{issue}-{slug}.md` with a header line `Feature N · P? · GitHub issue [#N](…)`.
5. Add the spec to `docs/features/README.md` (Active) and a row to the right milestone in `docs/roadmap.md`. When it ships, close the issue, move the spec to `docs/features/archive/`, and move the row to Shipped.

## Spec Template

```markdown
# <Feature Name>

## Motivation
<why this matters>

## Scope
<what is included>

## Non-Goals
<what stays out>

## Implementation Phases
1. <phase>
2. <phase>

## Verification
<tests, manual checks, hardware checks, screenshots, or DAC validation>
```

## Priority

- `P0` - Required to prove or ship the current milestone.
- `P1` - Core workflow or user-facing product wedge.
- `P2` - Meaningful product improvement without immediate urgency.
- `P3` - Idea, polish, or future option.

## Project Constraints

- Music only; PCM only inside the engine (DSD rides as DoP-packed PCM — feature 71; no DST, no native DSD).
- No video, streaming integration, libmpv, FFmpeg, or GPL dependencies.
- Design in Pencil before implementing substantial UI.
- Prove engine correctness before building around UI assumptions.
