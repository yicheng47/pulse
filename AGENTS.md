# Agent Instructions

Conventions for AI agents (Claude, Codex, Cursor, etc.) working in this repo. Copy this file into any new project repo as the starting point; append project-specific rules below the universal section.

## Universal collaboration rules

These apply to every repo and every session. Don't violate them without explicit user override in the current conversation.

### 1. Never commit, PR, or push without explicit ask

Don't run `git commit`, `gh pr create`, or `git push` proactively. After completing changes, report what was done and stop. Wait for "commit", "PR", "push", or equivalent before touching git.

**Why:** the user wants full control over when changes go into git history. Auto-committing forces work into commits before they've been verified.

### 2. Don't commit iterative fixes until the user confirms they work

When iterating on UI/behavior with the user testing live, don't commit each attempt. Each fix attempt should be a working-tree change only. Commit only after the user says "this works" / "good" / "ship it".

**Why:** pushing broken attempts clutters history and burns CI cycles. Multiple intermediate commits also obscure the actual fix in `git log`.

### 3. Never run destructive commands without confirmation

Destructive commands require explicit per-occurrence approval. The list, non-exhaustive:

- `rm -rf <anything>` outside `/tmp`
- `git reset --hard`, `git push --force`, `git checkout .`, `git clean -f`, `git branch -D`
- `docker compose down -v` (the `-v` wipes volumes)
- `truncate`, `DROP TABLE`, `TRUNCATE`, schema migrations that drop columns
- Any flag combination that destroys data

When the same goal can be achieved non-destructively (e.g. `docker compose restart` vs `down -v && up`, soft-delete vs hard-delete), default to the non-destructive form. If the destructive form is genuinely required, ask first.

**Why:** an early session wiped MySQL + ClickHouse volumes (assumed `down -v` was the canonical restart). Lost months of historical data. The least-destructive default is non-negotiable.

### 4. Push submodule commits before parent commit/PR

When a repo has git submodules and you've committed changes inside one, `git push` from the submodule **before** committing/pushing the parent. Otherwise the parent commit references a submodule SHA that doesn't exist on the remote, and CI / fresh clones break.

The flow:
1. `cd submodule && git commit -am '...' && git push`
2. `cd .. && git add submodule && git commit -m 'bump <submodule>' && git push`

Never do step 2 first.

### 5. `gh pr merge --auto` does NOT wait for CI on repos without required checks

`gh pr merge --auto` is misleadingly named: it only waits on configured merge gates (required status checks, required reviews, conversation resolution). If a repo's branch protection has no `required_status_checks`, `--auto` falls through to **immediate merge** — identical to plain `gh pr merge`.

Before relying on `--auto` for CI gating:

```sh
gh api repos/<owner>/<repo>/branches/main/protection | jq '.required_status_checks'
```

If null, either:
- Wait for CI yourself: `gh pr checks <n> --watch` until conclusions are SUCCESS, then merge
- Ask the user to add a required-checks rule
- Accept that `--auto` is no different from immediate merge and reason accordingly

Also: `gh pr merge` uses the user's token, so any merge appears under their account in `mergedBy`. Don't blame the user for a merge timing-matched to your own command — verify via timing first.

### 6. Markdown formatting: soft-wrap, one paragraph per line

In any Markdown file (`.md`):

- One paragraph = one line. No newline mid-paragraph.
- Separate paragraphs with a blank line.
- Inside fenced code blocks (`` ``` ``): preserve formatting exactly.
- Lists, tables, headings, blockquotes: format normally (their own line semantics).
- Don't hard-wrap at 70-80 cols "for readability in raw text" — Obsidian, GitHub, VS Code all soft-wrap visually.

**Why:** hard wraps create ugly mid-sentence breaks in any modern Markdown editor/renderer, inflate diff size when a single word changes, and conflict with paste/edit workflows.

**Gotcha when bulk-unwrapping:** a regex that only matches `^```...` will collapse indented code blocks inside list items. Detect fences with arbitrary leading whitespace.

### 7. Be opinionated when asked; don't hedge

The user explicitly wants opinions, not menus of options. When they ask "what should I do?", lead with the answer and the main tradeoff in 1-3 sentences. Don't enumerate 5 alternatives without picking one.

When you do enumerate (because the choice genuinely depends on the user's preference), the recommended option goes first and is marked "Recommended" or "My pick".

### 8. Don't over-build

- Don't add features, refactors, or abstractions beyond the task's scope. A bug fix doesn't need surrounding cleanup. Three similar lines is better than a premature abstraction.
- Don't add error handling, fallbacks, or input validation for scenarios that can't happen. Trust internal code and framework guarantees.
- Default to writing no code comments. Only add a comment when the WHY is non-obvious (hidden constraint, subtle invariant, workaround for a specific bug). Don't explain WHAT — well-named identifiers do that.

---

## Project-specific rules

- **Music only, PCM only.** No DSD, no video, no streaming integration. Don't scope-creep these back in.
- **No libmpv, no FFmpeg, no GPL deps** — keeps App Store eligibility and paid closed-source distribution open.
- **Playback path is AUHAL / Hardware AudioUnit by default.** Pulse feeds native-rate interleaved float32 through `coreaudio-rs`, while direct `objc2-core-audio` HAL code still owns device discovery, hog mode, sample-rate switching, and physical-format probing.
- **Realtime audio callback: no allocation, no locks, no syscalls.** Data crosses threads via `rtrb` only.
- **`pulse-engine` stays UI-agnostic** — no Tauri types in the engine crate. It must remain drivable from `pulse-cli` alone.
- Do not make hard bit-perfect claims for the AUHAL path. Hardware validation can prove native-rate/exclusive playback; a future raw-HAL integer engine would be needed before claiming unchanged integer frames reach the DAC.
