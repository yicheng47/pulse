# Update Check and Notification

> Feature 05 · P2. v0.1.0 shipped as a notarized DMG on 2026-08-10 with no way to tell a running copy that a newer one exists. Deliberately scoped to *notify*, not *self-install* — see Distribution Decision.

## Motivation

Pulse is distributed as a direct-download DMG outside the Mac App Store, so nothing tells an installed copy that a release happened. Every v0.1.0 user is stranded on v0.1.0 until they happen to revisit the repository. That is the normal fate of direct-distributed macOS apps that skip this, and it gets worse with each release: a bug fixed in v0.2.0 stays live on every machine that never learned there was a v0.2.0.

## Distribution Decision

**Stay on GitHub.** The release workflow already publishes signed, notarized, stapled DMGs to GitHub Releases on every `v*` tag, served over GitHub's CDN at no cost, and the repository is public so the releases API needs no token. Standing up separate update infrastructure (S3 + CloudFront, a status endpoint, a paid service) would add cost, credentials, and a second thing that can be down, in exchange for nothing this app needs. GitHub Releases stays the artifact host whether or not full self-updating is ever built — Sparkle-based updaters are routinely pointed at GitHub Releases too, so this decision does not constrain the next phase.

## Scope

- **Version check against the GitHub releases API.** `GET https://api.github.com/repos/yicheng47/pulse/releases/latest`, unauthenticated (public repo), reading `tag_name` and `html_url`. A 5-second timeout, one attempt, no retry.
- **Comparison is semantic, not string equality.** Parse the running version (`CARGO_PKG_VERSION` — the release workflow guards that the git tag equals the workspace version, so the compiled constant and the shipped `CFBundleShortVersionString` are the same number by construction) and the tag's `MAJOR.MINOR.PATCH` into a numeric triple and compare. A tag that is older or equal must never notify; non-conforming or pre-release tags are ignored rather than guessed at.
- **Surface: the existing notice banner.** A newer version renders through `PlaybackNotice` / `render_notice` (`playback_row.rs`) — the dismissible banner stage 13 already built — reading "Pulse 0.2.0 is available", with an action that opens the release page in the default browser via `NSWorkspace` (the objc2 idiom already used for icon and running-application calls). Dismissal is per-launch; no new UI component, so no Pencil pass is required.
- **When it runs.** Once per launch on a background worker, after the window is up so first paint is never delayed, plus an explicit **Pulse ▸ Check for Updates…** menu item that runs the same path and reports "Pulse is up to date" when it is.
- **Failure is silent by design.** Offline, DNS failure, timeout, HTTP error, rate limit, or an unexpected response shape logs and does nothing. A music player must never show a network error the user did not ask for. The manual menu check is the one exception: an explicit user request may report an explicit failure.
- **Opt-out without a settings screen.** Presence of a `check-updates.disabled` marker file in the app config directory disables the launch check, following the existing one-file-per-preference convention (`app-output-device.uid`). This is Pulse's only network call, and a local-first audiophile player should not phone home with no way to stop it. Document it in `README.md` alongside the privacy note.

## Non-Goals

- **Self-installing updates (Sparkle or hand-rolled download-and-replace).** Deferred deliberately, not rejected. Sparkle would embed `Sparkle.framework` in the bundle, which introduces nested code into a bundle that has none today — changing the signing sequence to inside-out, making the runbook's dropped `codesign --deep` note load-bearing again, adding EdDSA appcast keys as a sixth CI secret with real key custody, and requiring a generated appcast XML in the release workflow. The update path can then only be truly tested by shipping two consecutive versions. That is a disproportionate amount of new signing and release surface for an app whose user base is currently one person. Revisit when there are enough users that "download the new DMG yourself" is a real cost.
- Delta updates, update channels (beta/stable), staged rollout, or downgrade.
- Release notes rendered in-app — the banner links out to the release page, which already has them.
- Telemetry of any kind. The update check sends no identifiers; it is a plain unauthenticated GET.
- Any check on a timer during a session. Once per launch is enough for an app that is quit and relaunched.

## Implementation Phases

1. **Version comparison, pure and testable.** Parse `vMAJOR.MINOR.PATCH` into a triple, compare, reject malformed and pre-release tags. No I/O — unit tests cover newer, older, equal, malformed, and pre-release inputs.
2. **The fetch.** One HTTP GET on a worker thread. Recommended: `ureq` (blocking, no async runtime — an audio app should not drag in tokio for one request) plus `serde_json` for the two fields; `serde` is already a workspace dependency. The alternative, `NSURLSession` via objc2, adds no Rust dependency and uses the system TLS stack, but is materially harder to unit-test — take it only if the dependency is genuinely unwanted. Response parsing is tested against captured JSON fixtures, not the live API.
3. **Wiring.** Launch check behind the marker-file opt-out, menu item, notice banner presentation, browser open. Failure paths verified to be silent.

## Verification

- `make verify` green. Unit tests: the comparison matrix from phase 1; fixture-driven parsing including a malformed body and a missing-field body; the opt-out marker suppressing the launch check.
- Manual (Jason): with the running version temporarily set below the published tag, launch and confirm the banner appears and its action opens the release page; with the versions equal, confirm nothing appears; run the menu item in both states; pull the network and confirm launch is silent and unaffected; create the marker file and confirm the launch check does not fire while the menu item still does.
- Confirm no measurable delay to first paint and no audio interruption when the check runs during playback.
