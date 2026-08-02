# macOS Packaging and Release Infrastructure

> Feature 02 · P0 · the last thing standing between a working MVP and something installable. This is Pulse's first native-GPUI app, so there is no Tauri bundler to lean on — the `.app` is assembled by hand.

## Motivation

Everything ships as `cargo run` today. Stage 14 cannot happen without an installable, notarized artifact, and — more urgently — **the hardened runtime required for notarization can break behavior that works fine in development.** Under `cargo run` the binary inherits the terminal's TCC grants; a signed `Pulse.app` requests its own. Reading `/Volumes/Media` crosses macOS's network-volume gate, and hog mode and the `NSOpenPanel` folder picker are worth re-verifying under hardened runtime. Finding that out at v0-tag time would be miserable. Building this now de-risks stage 13 and 14 rather than following them.

## Why this is smaller than Tauri made it look

`crates/pulse-app/src/assets.rs` `include_bytes!`s every font, SVG, and the app icon, and `rusqlite` uses the `bundled` feature. The binary is fully self-contained, so the bundle is:

```
Pulse.app/Contents/
  Info.plist
  MacOS/pulse-app
  Resources/Pulse.icns
```

No resource tree, no sidecar, no webview payload. Zed — the reference GPUI app — hand-rolls its own bundling script for the same reason; there is no established GPUI bundler and nothing here needs one.

## Decisions (settled)

- **Bundle identifier: `com.wycstudios.pulse`.** Baked into `Info.plist`, the signature, and TCC identity. Changing it later silently resets the app's permission grants — treat as immutable.
- **Hand-rolled shell script over `cargo-packager`/`cargo-bundle`.** Nothing to configure, no third-party release cadence in the path of a rev-pinned-gpui project.
- **arm64-only for v0.** Universal doubles build time for users who do not exist yet; `lipo` can be added later without changing the script's shape.
- **Developer ID distribution, not App Store.** `mvp.md` already excludes App Store packaging. Non-sandboxed, so no App ID registration and no provisioning profile are required — the Developer ID Application certificate is account-wide and signs any bundle identifier.
- **Entitlements: minimal, likely empty beyond hardened-runtime defaults.** Audio *output* needs no entitlement (only input does), and non-sandboxed file access needs none. Add an entitlement only when something is proven to fail without it, and say which in the log.

## Scope

**Bundle assembly** — `script/bundle-mac` (or equivalent) producing `Pulse.app` from a release build: generate `Pulse.icns` from the existing 1024×1024 `crates/pulse-app/assets/app-icon/v06cM3.png` via `iconutil`, write `Info.plist` (identifier above, `CFBundleShortVersionString` from the workspace version `0.1.0`, `CFBundleName` "Pulse", `LSMinimumSystemVersion`, `NSHighResolutionCapable`, and `LSApplicationCategoryType` music), and place the binary.

**Signing, notarization, DMG** — `codesign --options runtime --timestamp` with the Developer ID identity, `xcrun notarytool submit --wait`, `xcrun stapler staple`, and a DMG via `hdiutil`. Each is its own Makefile target so they can be run and debugged independently.

**Makefile targets**: `bundle` (unsigned, no credentials needed), `sign`, `notarize`, `dmg`, and a `release-macos` that chains them.

**Release CI** — `.github/workflows/release.yml`, tag-triggered, reusing the exact keychain-import pattern already proven in the runner and quill repos so the secrets carry over unchanged:

| Secret | Use |
|---|---|
| `APPLE_CERTIFICATE` | base64 `.p12`, decoded into a temporary keychain |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` password |
| `APPLE_ID` | notarytool account |
| `APPLE_PASSWORD` | app-specific password for notarytool |
| `APPLE_TEAM_ID` | notarytool team |

`APPLE_SIGNING_IDENTITY` is derived at runtime from `security find-identity` into `$GITHUB_ENV`, exactly as those repos do. The `TAURI_SIGNING_*` secrets are updater-specific and do **not** apply — Pulse has no updater. The release job needs full Xcode and `xcodebuild -downloadComponent MetalToolchain`, matching the existing CI job, because gpui compiles Metal shaders at build time.

## Non-Goals

- Auto-update (Sparkle or otherwise), App Store packaging, universal binaries, Homebrew cask, Windows/Linux.
- Changing any app behavior. If TCC or hardened runtime exposes a real bug, file it — do not fix it inside this feature.

## Implementation Phases

1. **Bundle** — script, `Info.plist`, `.icns`, `make bundle`. Fully verifiable without credentials; produces a launchable app.
2. **Sign / notarize / DMG** — targets plus a short `docs/` runbook. Written here, *run* by Jason or CI (see Verification).
3. **Release CI** — tag-triggered workflow, artifacts attached to the GitHub release.

## Verification

- `make bundle` produces a structurally valid `.app`: `plutil -lint` on the plist, the icon present and non-empty, `codesign -dv` reporting the expected identifier once signed.
- **Agents must not run `make sign` or `make notarize`.** The Developer ID cert is not in the agent sandbox, and notarization submits to Apple — an external side effect. Those paths are verified by Jason locally or by the CI job on a tag.
- Jason's manual pass, the load-bearing part: launch the bundled app (not `cargo run`) and confirm the NAS root at `/Volumes/Media` is still readable, the Add Storage folder picker works, playback and hog mode work on the Matrix DAC, and library data still resolves under `~/Library/Application Support/pulse`. Then verify Gatekeeper accepts the notarized DMG on a clean download (quarantine attribute intact).
