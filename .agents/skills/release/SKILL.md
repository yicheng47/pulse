---
name: release
description: Use when preparing, tagging, or publishing a Pulse release.
---

# Release

Pulse is open-source first. Releases should be deliberate and should not happen without explicit user approval.

## Preconditions

1. Confirm the user explicitly asked to release.
2. Confirm the branch and working tree state.
3. Confirm the target version.
4. Run the relevant validation for the release scope.

## Version Files

When the Tauri app exists, version bumps should stay aligned across:

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`
- root `Cargo.toml` or workspace crate versions when applicable.
- `Cargo.lock`

Read files first and edit with normal file edits. Do not use broad mechanical substitutions unless the scope is proven safe.

## Validation

For engine-only releases:

- `cargo test --workspace`
- `cargo check --workspace`
- Hardware validation when the release claims bit-perfect playback behavior.

For app releases, also run:

- TypeScript build.
- Tauri build or package command.
- Manual app smoke test.

## Notes

- Do not commit, tag, push, create releases, or publish artifacts unless the user explicitly asks.
- Do not make bit-perfect claims unless they were validated on hardware.
- Keep release notes factual: engine changes, app changes, validation performed, known limitations.
