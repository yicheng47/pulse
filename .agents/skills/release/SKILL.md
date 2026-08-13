---
name: release
description: Use when preparing, tagging, or publishing a Pulse release.
---

# Release

Pulse ships as a signed, notarized, stapled arm64 DMG from a tag-triggered GitHub workflow. Releases are deliberate: never start one without the user explicitly asking, and stop and report on any failed step.

The full signing/notarization runbook (local commands, credential table, Gatekeeper checks) is `docs/macos-release.md`.

## Version source

The version lives in exactly one place: `[workspace.package] version` in the root `Cargo.toml`. `script/bundle-mac --print-version` derives from it, and the release workflow refuses to build if the git tag does not equal `v{that version}`. There is no package.json or tauri.conf.json — Pulse is a GPUI Rust workspace, not a Tauri app.

## Steps

1. Confirm the version number with the user if not given. Patch releases fix bugs; releases that ship features should bump minor.

2. Preconditions: on `main`, working tree clean, synced with `origin/main` (`git status -sb`), and the release commits are merged — check `git log $(git describe --tags --abbrev=0)..main --oneline` shows what the user expects to ship.

3. Validate: `make verify` (tests + clippy `-D warnings` + fmt). Do not tag on a red tree.

4. Bump: Edit `[workspace.package] version` in the root `Cargo.toml` with the Edit tool (never `sed`), run `cargo check --workspace` to refresh `Cargo.lock`, then commit both files as `chore: bump version to v{version}` and push directly to `main`. No PR, no release branch.

5. Tag and push: `git tag -a v{version} -m "Pulse v{version}" && git push origin v{version}`.

6. Watch the workflow: `gh run list --workflow=release.yml --limit 1`, then `gh run watch <id> --exit-status` (background it; a run takes ~10 min warm-cache). It builds arm64, imports the Developer ID cert, signs the hardened-runtime app, notarizes and staples the `.app`, builds the DMG, signs, notarizes and staples that too, and creates a **draft** GitHub release with `Pulse-{version}-arm64.dmg`. On failure, read `gh run view <id> --log-failed` and report — a 401 at notarytool means the `APPLE_*` secrets (set 2026-08-10, see `docs/macos-release.md`) need attention, not a retry.

7. Release notes: review `git log v{prev}..v{version} --oneline`, categorize into **What's New**, **Improvements**, **Bug Fixes** (omit empty sections), and end with a **Download** section naming `Pulse-{version}-arm64.dmg` (Apple Silicon only — there is no Intel build). Notes are factual: no bit-perfect claims (AGENTS.md forbids them for the AUHAL path), no claims not validated on hardware.

8. Publish: `gh release edit v{version} --draft=false --notes "..."`. If this release is the first exercise of a changed signing/packaging path, hand the draft to the user for a quarantined-DMG Gatekeeper smoke test (download through a browser, mount, launch) before publishing instead.

## Debugging notarization

```
xcrun notarytool history --apple-id yuesihan@gmail.com --team-id 49B2V2W538 --password <app-specific>
xcrun notarytool info <submission-id> ...
xcrun stapler validate <file>
codesign -dvv <path-to-app>
```

The app-specific password is not stored on this machine; it lives only in the repo's GitHub secrets. For local signing work, follow `docs/macos-release.md`.
