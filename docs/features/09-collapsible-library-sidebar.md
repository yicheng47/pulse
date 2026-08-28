# Collapsible Library Sidebar

> Feature 09 · P3 · GitHub issue [#34](https://github.com/yicheng47/pulse-src/issues/34). The Pencil library sidebar contains a collapse affordance, but the runtime sidebar has no collapsed state and intentionally omits the inert control.

## Motivation

Collapsing the 236px library sidebar would give album grids and tables more room in narrower windows. The control should appear only when it has a designed, functional destination state.

## Scope

- Design the collapsed library-sidebar width, brand treatment, navigation tooltips, active state, badges, Settings entry, and expand control in Pencil.
- Add an in-memory expanded/collapsed state and animate only if GPUI can do so without complicating layout or input handling.
- Preserve navigation, Settings access, native titlebar drag regions, and content sizing in both states.

## Non-Goals

- Collapsing the dedicated Settings sidebar, automatic collapse based on window width, sidebar resizing, or persistence across launches.

## Implementation Phases

1. Add and approve the collapsed state in `design/pulse-desktop.pen`.
2. Implement the two sidebar layouts behind one state in `shell.rs`.
3. Verify every library destination, badge, titlebar interaction, and Settings entry in both layouts.

## Verification

- `make verify` is green with view-model coverage for state toggling and destination preservation.
- Manual: collapse and expand from every library destination, use every icon and Settings while collapsed, resize the window, and compare both states against Pencil.
