---
name: feature
description: Use when creating, organizing, or updating Pulse feature specs and feature issues under docs/features.
---

# Feature Management

Pulse feature specs should live in `docs/features/` once feature planning starts. Keep specs small, numbered, and tied to the current build order.

## Workflow

1. Clarify the feature's motivation, scope, and non-goals.
2. Check existing docs in `docs/product/`, `docs/arch/`, and `docs/features/`.
3. Assign the next available number in `docs/features/`.
4. Create `docs/features/{number}-{slug}.md`.
5. State the priority (`P0`–`P3`) in the spec's header line. Do not create a GitHub issue — tracking moved to local docs on 2026-08-29.
6. Add the spec to `docs/features/README.md` (Active) and a row to the right milestone in `docs/roadmap.md`. When it ships, move the spec to `docs/features/archive/` and the row to Shipped.

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

- Music only; PCM only inside the engine (DSD rides as DoP-packed PCM — feature 33; no DST, no native DSD).
- No video, streaming integration, libmpv, FFmpeg, or GPL dependencies.
- Design in Pencil before implementing substantial UI.
- Prove engine correctness before building around UI assumptions.
