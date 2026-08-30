# Type Scale

> Feature 29 · P3 · refactor. Give `theme.rs` a named type scale and replace the literal `text_size(px(N))` calls across `pulse-app` with it, the way colors, radii, and font families already go through tokens. Filed 2026-08-30 after Jason asked whether sizes were centralized; not urgent.

## Motivation

Colors are fully tokenized — `theme.rs` holds every color, the three radii, and the three font families, and there is no raw `rgb(0x…)` outside it. Font sizes are not: `text_size(px(N))` appears at ~218 call sites with 17 distinct values (9, 10, 11, 12, 13, 14, 15, 17, 18, 20, 22, 23, 24, 28, 30, 34, 38). The top four (12 / 11 / 13 / 10) cover about 70% of uses; several of the rest look like drift rather than intent (22 vs 23 vs 24 for headings, 28 vs 30, 34 vs 38 for display numbers). A scale makes the design's type styles the source of truth and makes the rule grep-enforceable, like the color rule is today.

## Scope

1. **Read the design's type styles.** Collect the text styles actually used in `design/pulse-desktop.pen` (Pencil `get_style` / the text nodes on the reference frames) and map the 17 code sizes onto them. Expect roughly eight steps: caption (9–10), small (11), body (12–13), label (14–15), title (17–18), heading (20–22), display (34+). Sizes that differ by 1–2 px from a design style are drift — collapse them; anything the design genuinely uses stays.
2. **Tokens.** A `theme::text` module (or constants beside `RADIUS_*`) naming each step, with the design style name in a one-line comment where the name is not self-explanatory. Font family stays a separate token; the scale is size only (line height where gpui needs it explicitly).
3. **Replace the literals.** Every `text_size(px(N))` in `crates/pulse-app/src` goes through a token. Mechanical; no visual change except where a drifted size collapses onto its neighbor — list those in the handoff so the smoke can look at them.
4. **Gate.** A test or `make verify` grep that fails on `text_size(px(` outside `theme.rs`, mirroring the color rule.

## Non-Goals

- A spacing scale for gaps and paddings. Same class of issue, larger diff, less payoff; revisit if it bothers anyone.
- Changing any size the design actually specifies.
- Touching `pulse-engine` or `backend/**`.

## Implementation Phases

1. Design read-through and the token list (a short note in the spec or the handoff, agreed before the replacement starts).
2. Replacement + gate, one mission.

## Verification

- `make verify` green; `grep -rn "text_size(px(" crates/pulse-app/src --include=*.rs | grep -v theme.rs` is empty.
- Manual: screenshots of Albums, an album detail, Tracks, Settings ▸ General, and the playback row before and after; only the listed drift collapses differ.
