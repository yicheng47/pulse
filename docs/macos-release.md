# macOS Release Runbook

Pulse ships as an arm64-only, non-sandboxed Developer ID application. The bundle identifier is `com.wycstudios.pulse`; changing it would change the app's signature and TCC identity. Pulse has no entitlements file, App ID, or provisioning profile because audio output and non-sandboxed file access need no additional entitlement.

## Local bundle

Install full Xcode, download the Metal compiler with `xcodebuild -downloadComponent MetalToolchain`, and install the Rust target with `rustup target add aarch64-apple-darwin`.

Run `make bundle` to compile the release binary and assemble the unsigned app at `target/release/Pulse.app`. The command derives the bundle version from `[workspace.package]` in `Cargo.toml`, generates `Pulse.icns` from the embedded 1024px source icon, and validates `Info.plist`.

## Local signed release

Install the Developer ID Application certificate in the login keychain, then select its full identity:

```sh
security find-identity -v -p codesigning
export APPLE_SIGNING_IDENTITY="Developer ID Application: ..."
```

Build and sign the app, create the DMG, then submit and staple it:

```sh
make bundle
make sign
make dmg
make sign-dmg

export APPLE_ID="developer@example.com"
export APPLE_PASSWORD="app-specific-password"
export APPLE_TEAM_ID="TEAMID"
make notarize
```

`make release-macos` runs the complete sequence when all credentials are already exported. `make sign` signs the hardened-runtime app, and `make sign-dmg` signs the disk image without changing the credential-free `make dmg` target. Both signing targets refuse to run without `APPLE_SIGNING_IDENTITY`; `make notarize` refuses to run without all three notarization credentials.

Inspect a local result with `codesign --verify --deep --strict --verbose=2 target/release/Pulse.app`, `codesign -dv --verbose=4 target/release/Pulse.app`, `codesign --verify --verbose=2 target/release/Pulse-<workspace-version>-arm64.dmg`, and `xcrun stapler validate target/release/Pulse-<workspace-version>-arm64.dmg`. Run `spctl --assess --verbose=4 --type install target/release/Pulse-<workspace-version>-arm64.dmg` as a local Gatekeeper check. The real distribution check must still use a freshly downloaded DMG whose quarantine attribute is intact.

## GitHub Actions secrets

| Secret | Purpose |
|---|---|
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12`; CI imports it into a temporary keychain. |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting the `.p12`. |
| `APPLE_ID` | Apple developer account used by `notarytool`. |
| `APPLE_PASSWORD` | App-specific password for that Apple ID, not the account password. |
| `APPLE_TEAM_ID` | Apple Developer team identifier passed to `notarytool`. |

CI derives `APPLE_SIGNING_IDENTITY` from the imported certificate at runtime. Pulse has no updater, so it does not use any `TAURI_SIGNING_*` secret.
