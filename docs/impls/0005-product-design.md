# Product Design

> Fifth product stage: define the desktop product shape in Pencil before adding frontend or Tauri app-setting code.

## Context

Stages 1 through 4 proved the engine through `pulse-cli`: device listing, file probing, format validation, AUHAL playback, and UID-backed CLI defaults. That work was intentionally infrastructure-heavy.

The next risk is different. If we start adding app settings, library screens, or playback controls before the product shape exists, the code will harden around accidental UI decisions. Stage 5 is therefore a design stage, not another backend probing stage.

## Goal

Create the first Pulse product design in Pencil:

```text
design/pulse-app.pen
```

The design should cover the first app shell surfaces:

- settings and output-device preference
- local library browsing
- search
- now-playing foundation
- navigation between those surfaces

The output-device settings design should decide whether the app default lives as a normal SQLite setting, a separate config file, or a hybrid. Do not lock that storage shape in backend code before the design is clear.

## Boundary

No React settings UI lands in this stage.

No Tauri app-settings backend lands in this stage unless the Pencil design creates a concrete contract that needs a backend spike. `pulse-cli` remains the validation harness for output-device behavior until then.

## Deliverables

- Pencil file under `design/`.
- Short design notes in `docs/impls/` or `docs/product/` describing the chosen app shell and settings model.
- Updated roadmap if the design changes the implementation order.

## Verification

- The design file exists and can be opened by the Pencil MCP.
- The settings surface has enough detail to implement without guessing.
- The implementation order after Stage 5 is still reflected in `ROADMAP.md`.
