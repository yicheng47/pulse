# Typography — Eyebrow Text Size

Feature 88 · P3 · GitHub issue [#88](https://github.com/yicheng47/pulse/issues/88). Jason, 2026-09-04, on the album page at interface scale: "the section header should be larger? I mean the font size issue … the album header 'Album' should be larger?"

## Motivation

Three kinds of label share the 10 px `CAPTION` token in Geist Mono: the sidebar section labels (`LIBRARY`, `MANAGE` — `ui/sidebar.rs`, `SidebarSection`), the page eyebrows (`ALBUM` — `surfaces/library/albums.rs`, and the equivalents on the artist and playlist pages), and the table column headers (`#`, `TITLE` — `render_album_track_header` and the Tracks and playlist tables). Against 15 px sidebar items, a 24 px title, and 13 px rows they read as lost rather than quiet: a mono face with letter-spacing renders smaller than Inter at the same nominal size, which is why the 10 px badges inside their pills look fine while the bare labels don't. Every size is in rems, so this is the ratio, not the interface scale.

## Scope

- **One new token.** `theme::text::EYEBROW` at 11 px (`rpx(11.)`), between `CAPTION` and `SMALL` in intent: the size for a label that names a group without competing with it.
- **Three uses.** Sidebar section labels, page eyebrows on the album, artist, and playlist pages, and table column headers on the album, Tracks, and playlist tables move from `CAPTION` to `EYEBROW`. Font, weight, colour, and letter-spacing stay as they are.
- **Nothing else moves.** Badges (`FLAC 24/88.2`, `HI-RES`, `NO INTEGER PATH`) stay at `CAPTION`; titles, items, and rows are unchanged.
- **Pencil first.** The change is a typography token across three surfaces, so the frames get the new size before the mission and Jason approves them; the Design Source is pinned here afterwards.

## Non-Goals

- Changing badge sizes, titles, body sizes, fonts, or letter-spacing; any interface-scale change; a wider typography audit.

## Implementation Phases

1. **Pencil pass**: the sidebar, album page, and track table frames with the labels at 11 px; Jason approves.
2. **Token + call sites**: add `EYEBROW`, switch the three uses, and keep `CAPTION` for badges. `make verify`; no rendering test is expected.

## Verification

- `make verify` green.
- Visual pass against the frames at 100% and at Jason's usual scale: section labels, eyebrows, and column headers read as one family, one step below the items they label; badges unchanged.
