# 0010 — App shell

> Stage 7.5: the chrome every later surface drops into. Prerequisite for stages 8–13 — device management and Storage both live inside this frame, and today there is nowhere to put them.

## Goal

Build the shared app chrome: sidebar navigation, top bar, a routed body region, and the existing playback row docked at the bottom. No page content — this stage delivers the frame and the navigation, not the destinations.

**Do not regress the working MVP.** Dropping an audio file anywhere in the window must still play it, with the row still showing state and seeking. That behavior is the only thing Pulse currently does; the shell wraps it, it does not replace it.

## Design source

Every screen frame in `design/pulse-desktop.pen` shares identical chrome — `E3N1P` (Library / Main), `C6IrDC` (Library / Storage), `KfJr9` (Library / Tracks), `MHrLm` (Library / Playlists). Read through the `pencil` MCP; `.pen` files are encrypted, never Read or Grep them. Screen frames are 1440×900.

Root: `bg-page`, horizontal — sidebar, then a right column holding Main (top bar + body) above the docked player.

**Sidebar** (`vNEQj` in `E3N1P`): 236 wide, `bg-surface`, 1px `border` on the right edge, padding [24,14,16,14], vertical, gap 22.

- Brand: 32×32 app-icon mark (radius 7, 1px `border`, vertical gradient `bg-surface`→`bg-inset`, rotation 180) plus a two-line copy block, gap 10.
- Navigation: vertical, gap 20, three groups each vertical gap 6 — a header wrap (padding [0,10]) over an items list (vertical, gap 4).
- A `fill_container` spacer.
- Settings footer: padding [9,10], gap 10, radius-md — `settings` icon 18px `text-muted`, "Settings" label Rajdhani 14/600 `text-secondary`, a 1px spacer filling, then a `chevrons-left` collapse icon 17px `text-muted`.

Group headers are Geist Mono 10/700 `text-muted`, letterSpacing 0.8: `LIBRARY`, `MANAGE`, `OUTPUT`.

| Group | Item | Icon (lucide) | Extra |
|---|---|---|---|
| LIBRARY | Albums | `library` | active in the design |
| LIBRARY | Tracks | `music` | |
| LIBRARY | Playlists | `list-music` | |
| MANAGE | Storage | `database` | trailing count badge: `bg-muted`, 1px `border`, radius-sm, padding [2,6] |
| OUTPUT | Devices | `speaker` | |

Nav item: radius-md, padding [9,10], gap 10, icon 17px, label Rajdhani 15/600.

- Active: fill `accent-soft`, icon `accent`, label `text-primary`.
- Inactive: transparent fill, icon `text-muted`, label `text-secondary`.

**Top bar** (`U1fnkM`): 74 tall, padding [0,28], `space_between`, 1px `border` on the bottom edge. Contains one child — an `Input / Search` instance (`VoSds`) overridden to 420 wide. The component is `bg-inset`, 1px `border`, radius-md, padding [10,12], gap 10, a 16px `search` icon `text-muted`, and placeholder "Search library" in Inter 13 `text-muted`.

**Body**: fills the remaining space, `bg-page`. Storage's body uses padding [26,28,24,28] and gap 20 — reuse that as the page content inset.

**Docked player**: a `qKkw7` instance, 92 tall, full width.

## Application menu and standard shortcuts

GPUI installs no macOS menu bar, and Tauri used to provide one. On macOS the standard shortcuts are delivered *through* the menu bar, so **Cmd+Q currently does nothing** — along with Cmd+W, Cmd+M, and Cmd+H. This is shell chrome, so it lands here.

Declare an app menu with `cx.set_menus(..)`, register actions via the `actions!` macro plus `cx.on_action(..)`, and bind keys with `cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)])`. `App::quit()` exists for the Quit handler.

Minimum: an application menu with About, a separator, Hide/Hide Others, and Quit; plus a Window menu with Minimize and Close. Use `MenuItem::os_submenu` with `SystemMenuType` where the OS manages the contents (Services, Window).

Note for later: standard edit commands must use `MenuItem::action` with an `os_action` of `OsAction::Cut/Copy/Paste/SelectAll` — custom actions will not drive a text field. Not needed now, since nothing accepts text input until search in stage 10, but the Edit menu should be added at the same time as that field or Cmd+C will silently do nothing inside it.

`gpui-ce`'s `Menu` struct carries a `disabled` field that Zed's does not, so struct literals need it or use the `Menu::new` builder. Verify against the pinned checkout rather than Zed's docs.

## Fix the row to its in-context size

The shipped row was built from the standalone `qKkw7` component, but both screens instantiate it with overrides: cover **52×52** (not 60) and Now Playing **330** wide (not 320). The instance is what the design actually shows, so adjust `playback_row.rs` to match. Verify the current override values in the file rather than trusting this note.

## Scope

In:

- The chrome above, rendered from `theme.rs` tokens.
- Nav state: clicking an item selects it and swaps the body; active styling follows selection. Albums is the initial selection, per the design.
- A routed body that renders a labelled placeholder per destination, carrying the existing drop-to-play hint so the MVP behavior stays reachable.
- Window-wide file drop and the full playback row keep working exactly as they do now.

Out — render, do not wire:

- The search input (search is stage 10). Render it; do not accept focus or input.
- The sidebar collapse chevron. No collapsed-sidebar state is designed, so wiring it would mean inventing one.
- The Settings footer row. No Settings surface is designed.
- The Storage count badge's number — no library exists yet. Render the badge shape with an honest value rather than the design's placeholder count.

Out entirely: Albums, Tracks, Playlists, Storage, and Devices page bodies. Those are stages 8–11, each behind its own note.

## Token drift to watch

Parts of the design carry raw hex instead of `$token` bindings — the whole MANAGE group, and the Storage screen's panels. Two of those hex values are not in the variable set at all: the Storage panels use `#151514` and `#111110`, where the nearest tokens are `bg-surface #161615` and `bg-page #0F0F0F`.

For this stage, bind everything to `theme.rs` tokens and treat the token as correct where a raw hex differs by a hair. Do not add new near-duplicate colors to `theme.rs`. If a raw hex is visibly different rather than a rounding artifact, stop and raise it — the design likely needs a re-tokenize pass rather than the code needing a new constant.

## Verification

- `make verify` green: check, tests, clippy under `-D warnings`, fmt.
- `make run`: the shell matches the design. Compare against a `get_screenshot` of `E3N1P` rather than judging from memory, and report divergences.
- Clicking each nav item changes selection and body, with active styling following.
- Cmd+Q quits, Cmd+W closes the window, Cmd+M minimizes, Cmd+H hides. The app menu appears in the menu bar with the app's name.
- **Regression check, mandatory:** drop a real audio file and confirm it still plays, the row still updates, play/pause still toggles, and the progress bar still seeks. Any regression here is a must-fix, not a note.
- Hardware validation on the Matrix Mini-i Pro 4 is unchanged and still outstanding from stage 7 — agents cannot verify sound.

## Risks

- The window is currently 1280×800 while the design frames are 1440×900. Decide whether to widen the default window or let the layout adapt, and say which. The sidebar is fixed at 236; everything else should flex rather than assume 1440.
- Five destinations with empty bodies is scaffolding. Keep the placeholders honest and obviously unbuilt — do not mock up content that suggests a working library.
- Nav routing is the first piece of app-level state that is not playback. Keep it a plain enum on the shell view; do not introduce a router abstraction for five static destinations.
