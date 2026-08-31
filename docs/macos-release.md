# macOS Release Runbook

Pulse ships as an arm64-only, non-sandboxed Developer ID application. The bundle identifier is `com.wycstudios.pulse`; changing it would change the app's signature and TCC identity. Pulse has no entitlements file, App ID, or provisioning profile because audio output and non-sandboxed file access need no additional entitlement.

The bundle declares `LSMinimumSystemVersion` 13.0 and builds against the same deployment target (`MINIMUM_SYSTEM_VERSION` in `script/bundle-mac`, raised from 12.0 on 2026-08-29). The floor is a policy, not a measurement: Pulse ships arm64-only, every Apple Silicon Mac can run macOS 13 or newer, and macOS 12 left security support in September 2024. Pulse has only ever been launched on the current OS, and because `objc2` sends messages at runtime no compile-time availability check would catch an API newer than the floor — raise it again rather than guess if an old-OS report ever appears. Release consequence: Sparkle's `generate_appcast` derives `sparkle:minimumSystemVersion` from `LSMinimumSystemVersion`, so an already-installed Pulse on macOS 12 is silently no longer offered updates from v0.1.8 on; it does not error.

## Local bundle

Install full Xcode, download the Metal compiler with `xcodebuild -downloadComponent MetalToolchain`, and install the Rust target with `rustup target add aarch64-apple-darwin`.

Run `make bundle` to compile the release binary with the `updater` feature and assemble the unsigned app at `target/release/Pulse.app`. The command derives the bundle version from `[workspace.package]` in `Cargo.toml`, downloads the Sparkle release pinned by URL and SHA-256 in the Makefile, preserves the framework's symlinks while embedding it under `Contents/Frameworks`, generates `Pulse.icns` from the embedded 1024px source icon, and validates `Info.plist`.

## Local signed release

Install the Developer ID Application certificate in the login keychain, then select its full identity:

```sh
security find-identity -v -p codesigning
export APPLE_SIGNING_IDENTITY="Developer ID Application: ..."
```

Build and sign the app, create the DMG, then submit and staple it:

```sh
export APPLE_ID="developer@example.com"
export APPLE_PASSWORD="app-specific-password"
export APPLE_TEAM_ID="TEAMID"

make bundle
make sign
make notarize-app
make dmg
make sign-dmg
make notarize
```

The app is notarized and stapled *before* the DMG is built, so a copy dragged out of the image launches without needing a network round trip to Gatekeeper. `make dmg` stages `Pulse.app` beside an `Applications` symlink targeting `/Applications`. The DMG is then notarized and stapled in its own right. That means two `notarytool` submissions per release, which is the intended cost.

`make sign` never uses `--deep`. It explicitly re-signs Sparkle and the app from the innermost code outward, in this order:

1. `Sparkle.framework/Versions/B/XPCServices/Installer.xpc`
2. `Sparkle.framework/Versions/B/XPCServices/Downloader.xpc`, preserving its entitlements
3. `Sparkle.framework/Versions/B/Autoupdate`
4. `Sparkle.framework/Versions/B/Updater.app`
5. `Sparkle.framework`
6. `Pulse.app`

`make release-macos` runs the complete bundle, inside-out sign, app notarize/staple, DMG, DMG sign, and DMG notarize/staple sequence when all credentials are already exported. `make sign-dmg` signs the disk image without changing the credential-free `make dmg` target. Both signing targets refuse to run without `APPLE_SIGNING_IDENTITY`; `make notarize-app` and `make notarize` refuse to run without all three notarization credentials.

Inspect a local result with `codesign --verify --strict --verbose=2 target/release/Pulse.app`, `codesign -dv --verbose=4 target/release/Pulse.app`, `codesign --verify --verbose=2 target/release/Pulse-<workspace-version>-arm64.dmg`, and `xcrun stapler validate target/release/Pulse.app`, and `xcrun stapler validate target/release/Pulse-<workspace-version>-arm64.dmg`. Run `spctl --assess --verbose=4 --type install target/release/Pulse-<workspace-version>-arm64.dmg` as a local Gatekeeper check. The real distribution check must still use a freshly downloaded DMG whose quarantine attribute is intact.

## GitHub Actions secrets

| Secret | Purpose |
|---|---|
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12`; CI imports it into a temporary keychain. |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting the `.p12`. |
| `APPLE_ID` | Apple developer account used by `notarytool`. |
| `APPLE_PASSWORD` | App-specific password for that Apple ID, not the account password. |
| `APPLE_TEAM_ID` | Apple Developer team identifier passed to `notarytool`. |
| `SPARKLE_ED_PRIVATE_KEY` | Base64-encoded EdDSA private seed consumed by Sparkle's `generate_appcast`; it must never be written to the repository or workflow logs. |

CI derives `APPLE_SIGNING_IDENTITY` from the imported certificate at runtime. Pulse does not use any `TAURI_SIGNING_*` secret.

The Sparkle key lives under the `com.wycstudios.pulse` account in the login Keychain. Its public half is pinned as `SUPublicEDKey` in the app's `Info.plist`; the private half lives only in that Keychain and the `SPARKLE_ED_PRIVATE_KEY` Actions secret. Back up the private key alongside the Developer ID certificate: losing it can force users through a manual-download recovery, while leaking it weakens the update chain to Apple code signing alone.

## Repository

Source, CI, releases, and user bug reports all live in the single public `yicheng47/pulse` repo (open source, GPLv3). The 2026-08-28 private split into `pulse-src` plus a public releases shell was reversed on 2026-08-31 by renaming the source repo back and re-publishing the shell's releases here; releases v0.1.5–v0.2.0 therefore carry a 2026-08-31 publish date with their original notes and assets, and their tags point at the true source commits.

## Release appcast

For every matching version tag, `.github/workflows/release.yml` keeps the tag-version guard and draft-release flow, completes the existing signed/notarized DMG build, then downloads the same checksum-pinned Sparkle tools. It streams `SPARKLE_ED_PRIVATE_KEY` into `generate_appcast`, signs the built DMG entry, points its download and release-notes links at the tagged release, and uploads `appcast.xml` beside the DMG as a draft release on this repo with the workflow's own `GITHUB_TOKEN`. Release notes are written by hand when publishing the draft.

Bundled apps read the stable feed URL `https://github.com/yicheng47/pulse/releases/latest/download/appcast.xml`. The alias starts resolving to a draft's appcast only after that release is published, so publishing the GitHub release is the final activation step. GitHub's anonymous edge cache can keep serving the previous alias target for several minutes after publishing; verify with a cache-busting query string (`?cb=1`) rather than assuming the publish failed.
