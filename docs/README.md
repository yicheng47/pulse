# Pulse Docs

Project markdown lives here. Keep repo docs close to the code; use the memory repo for durable cross-session context, not as the only source of project decisions.

## Layout

- `arch/` - architecture, app structure, stack choices, technical constraints.
- `impls/` - tactical implementation notes for concrete build slices.
- `product/` - product scope, positioning, user-facing feature direction.
- `reference/` - learning material, external references, validation notes.

Each of those has an `archive/` subdirectory. A doc moves there once it stops describing current truth — a shipped stage's impl note, a superseded decision — keeping its filename so links and stage numbers stay stable. Anything still in the parent directory is expected to be accurate now, so the listing itself signals what is live. Before archiving a doc, move any decision it carries that outlives the work into `arch/`.

Current architecture docs:

- `arch/tech-stack.md` - stack choices and constraints.
- `arch/pulse-engine.md` - engine crate structure and module responsibilities.
- `product/vision.md` - broad product direction and constraints.
- `product/mvp.md` - first usable desktop release scope.
- `impls/ROADMAP.md` - canonical implementation stage order.
- `impls/README.md` - index of active and archived implementation notes.

Runner's docs tree is larger (`features/`, `tests/`, `journals/`) because it is already shipping iterative app work. Pulse should add those folders when feature planning starts, not before.
