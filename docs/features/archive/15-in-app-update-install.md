# In-App Update Install

> Feature 15 · P2 · Issue [#52](https://github.com/yicheng47/pulse/issues/52). Builds on [02-macos-packaging](02-macos-packaging.md) (the signing/notarization pipeline this feature extends) and supersedes [05-update-check](05-update-check.md): the hand-rolled checker and its banner states retire in favor of Sparkle, which owns checking, downloading, verifying, installing, and relaunching.

## Motivation

The checker tells the user a release exists and then abandons them: the `Available` banner's one action opens the GitHub release page in a browser, and the user downloads, mounts, drag-replaces, and relaunches by hand. Every release each user skips widens the gap between shipped fixes and running copies. The whole point of the checker was to close the update gap; the manual tail is where it actually stays open.

## Approach Decision

**Adopt Sparkle 2 (MIT), the standard updater for non-App-Store Mac apps** — the framework behind iTerm2, IINA, Transmission, and most of the indie Mac ecosystem. Decided 2026-08-16, reversing 05's deferral: Pulse is being built to professional grade, and the professional answer to "app updates" on macOS is two decades of solved installer edge cases — unwritable install locations with admin-auth escalation, atomic swaps, relaunch orchestration, delta updates and channels when they're wanted later — not a hand-rolled subset that would be rebuilt as Sparkle anyway once real users arrive. The costs 05 priced are accepted knowingly: an embedded framework introducing nested code (signing goes inside-out), an EdDSA signing key as a new CI secret with real custody obligations, and appcast generation in the release workflow. What was true in 05 stays true: GitHub Releases remains the artifact host — Sparkle points at it directly.

One consequence is structural: Sparkle is not bolted onto the existing checker, it **replaces** it. Running Sparkle's scheduled checks next to the hand-rolled `ureq` checker would mean two update systems with two sources of truth. `update.rs` (fetch, version parsing, `UpdateState`, fixtures and tests) retires; the menu item, the Settings ▸ Update page, and the launch-check preference rewire onto Sparkle.

## Design source

`design/pulse-desktop.pen`: the update flow's own UI (the update alert, release notes, download progress, restart prompt) is Sparkle's standard user driver — native AppKit, deliberately not redesigned in v1; implementing `SPUUserDriver` to render the flow in Pulse's own GPUI language is deferred (see Non-Goals). The **`Settings / Update`** screen (`D9PDB`) needs a simplifying Pencil pass: version line, a **Check for Updates** button that invokes Sparkle, and the "Check for updates on launch" toggle now bound to Sparkle's automatic-checks setting. **`Spec — Update States`** (`ADNMS`) retires with the hand-rolled checker.

## Scope

- **Framework embedding.** `make bundle` fetches a pinned Sparkle 2 release tarball (URL + SHA-256 checked into the Makefile), verifies the checksum, and copies `Sparkle.framework` into `Pulse.app/Contents/Frameworks/`. Pulse is not sandboxed, so the framework embeds as shipped — no XPC service surgery.
- **Signing goes inside-out.** The bundle now contains nested code (the framework carries its `Autoupdate` helper and `Updater.app`), so `make sign` signs the framework's nested executables first, then the framework, then the app — explicitly, never `--deep`. Notarization order in the runbook is otherwise unchanged; `docs/macos-release.md` gains the sequence.
- **EdDSA keys.** Generate once with Sparkle's `generate_keys` (private key stays in the login Keychain locally and becomes the `SPARKLE_ED_PRIVATE_KEY` CI secret; it appears nowhere in the repo). The public key ships in `Info.plist` as `SUPublicEDKey`. Custody is a real obligation: a lost key means users must manually download the next release; a leaked key weakens the update chain to Apple code signing alone. Both keys are backed up the same way the Developer ID certificate is.
- **Appcast in CI.** The release workflow runs Sparkle's `generate_appcast` over the built DMG (Sparkle installs from DMG; the existing single artifact serves first installs and updates alike) and uploads the resulting `appcast.xml` as a release asset. `SUFeedURL` points at the stable alias `https://github.com/yicheng47/pulse/releases/latest/download/appcast.xml`, so the feed always describes the newest release with no extra hosting, no Pages branch, and no repo commit from CI. Release notes: the appcast item links out to the GitHub release page in v1; embedding rendered notes in the update sheet is a later refinement.
- **Rust wiring.** Sparkle is Objective-C, consumed through hand-declared `objc2` externs (`extern_class!` for `SPUStandardUpdaterController`) — the idiom the shell already uses for `NSWorkspace` and dock/icon calls, so no new binding machinery. The controller is created at app startup; **Pulse ▸ Check for Updates…** invokes `checkForUpdates:`. Because a bare `cargo run` binary has no embedded framework to link or load, the integration sits behind a cargo feature (`updater`) that `make bundle` builds enable — consistent with 04's dev/prod isolation posture; dev builds show the Settings page with the updater controls disabled.
- **Settings ▸ Update, simplified.** Version line, **Check for Updates** button, and the launch toggle now reading and writing `automaticallyChecksForUpdates` (Sparkle persists it in user defaults). The existing `check-updates.disabled` marker file seeds the Sparkle setting once on first updater launch and is then deleted. 05's product stances carry over on Sparkle's terms: scheduled checks that fail stay silent, a manual check may report failure explicitly, and automatic *downloads* stay off — Sparkle notifies, the user clicks install.
- **Retirement.** `update.rs`, its fixtures, the `UpdateState` machine, and the banner's update states are deleted; the playback-row notice banner keeps its other duties. The one behavioral change accepted: a scheduled check that finds an update shows Sparkle's standard update window rather than the in-app banner — the normal Mac experience this feature is buying.
- **DMG gets a drag target.** `make dmg` currently images the bare bundle, so a user opening the DMG has nothing to drag onto. The target now stages a folder holding `Pulse.app` plus an `Applications → /Applications` symlink and images that. Sparkle never touches the symlink; this serves first installs and the failure-fallback of downloading a DMG by hand.

## Non-Goals

- **Custom `SPUUserDriver`.** Rendering Sparkle's flow in Pulse's own GPUI design language is the professional *end state*, but it multiplies the FFI surface (implementing an ObjC protocol from Rust) for zero functional gain. Standard driver first; revisit after the pipeline has shipped a real update.
- **Automatic background installs.** `SUAutomaticallyUpdate` stays off; the user always clicks.
- **Delta updates, channels, phased rollout.** Sparkle supports all three when wanted; none are configured in v1.
- **Sandboxing work.** Pulse is not sandboxed; the XPC-service variants of Sparkle's installer are out of scope.
- Downgrade support, Intel builds, or serving the appcast from anywhere but GitHub Releases.

## Security model

Two independent signatures gate every update: Sparkle verifies the EdDSA signature from the appcast against the `SUPublicEDKey` pinned in the running app, and verifies the new bundle's Apple code signature (Developer ID, same team). TLS to GitHub selects which bytes arrive, but trust does not rest on the transport or the host — a compromised CDN or repo cannot produce an installable update without the private EdDSA key. This is strictly stronger than both the browser-download path (Gatekeeper alone: any notarized developer passes) and the previously specced hand-rolled path (code signature + team pin, one signature scheme instead of two).

## Implementation Phases

1. **Packaging first, no code.** Framework fetch + embed in `make bundle`, inside-out signing in `make sign`, then a full notarize round. Deliverable: a notarized, stapled Pulse.app carrying an unused Sparkle.framework that launches clean and passes `spctl`. This proves the riskiest change — the signing sequence — in isolation.
2. **Keys and appcast.** `generate_keys`, Keychain + CI secret, `SUPublicEDKey` and `SUFeedURL` into the Info.plist template, `generate_appcast` + asset upload in the release workflow. Deliverable: a release whose appcast resolves at the `latest/download` URL with a valid EdDSA signature.
3. **Wiring.** `objc2` externs behind the `updater` feature, controller at startup, menu item, Settings page rework, marker-file migration, retirement of `update.rs` and the banner states.
4. **End-to-end and docs.** Downgraded-build test (below), `docs/macos-release.md` runbook update (signing order, new secret, appcast step), README privacy note updated to describe Sparkle's check (no system profiling — `SUEnableSystemProfiling` stays off).

## Verification

- Phase 1 gate: `codesign --verify --strict --verbose=2` on the bundle, `xcrun stapler validate`, `spctl --assess` on the DMG — all green with the framework embedded.
- End-to-end without shipping twice: build a bundle with the workspace version downgraded (e.g. `0.0.1`), drop it in `/Applications`, launch — Sparkle offers the real latest release from the production feed; the full download → verify → install → relaunch path runs against a genuine signed asset. Repeat with a deliberately corrupted `appcast.xml` signature to watch it refuse.
- Toggle migration: a config dir carrying `check-updates.disabled` yields a first launch with automatic checks off and the marker gone.
- After the next real release: the previous version's standard update flow succeeds on a user-clean machine, and Settings ▸ About on the relaunched app shows the new version.
- Mount the new DMG by hand and confirm the Applications symlink renders as a drag target in Finder.
