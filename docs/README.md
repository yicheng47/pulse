# Pulse Docs

Project markdown lives here. Keep repo docs close to the code; use the memory repo for durable cross-session context, not as the only source of project decisions.

## Layout

- `arch/` - architecture, app structure, stack choices, technical constraints.
- `impls/` - tactical implementation notes for concrete build slices.
- `product/` - product scope, positioning, user-facing feature direction.
- `reference/` - learning material, external references, validation notes.

Current architecture docs:

- `arch/tech-stack.md` - stack choices and constraints.
- `arch/pulse-engine.md` - engine crate structure and module responsibilities.
- `product/vision.md` - broad product direction and constraints.
- `product/mvp.md` - first usable desktop release scope.
- `impls/0001-engine-validation-cli.md` - first-stage engine validation plan.

Runner's docs tree is larger (`features/`, `impls/`, `tests/`, `journals/`) because it is already shipping iterative app work. Pulse should add those folders when the Tauri app and feature planning start, not before.
