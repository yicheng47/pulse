# Validate or raise the declared macOS 12 deployment target

> Bug · P2 · filed 2026-08-13 as GitHub issue #36 (closed when tracking moved to docs, 2026-08-29). **Resolved** 2026-08-29, commit `84cf129` — the floor was raised to macOS 13.0 rather than validated on 12: Pulse ships arm64-only, every Apple Silicon Mac can run 13+, and Monterey left security support in September 2024. Validation on the floor itself is still open (see `docs/macos-release.md`).

## Description
Pulse declares `LSMinimumSystemVersion` 12.0 and builds with `MACOSX_DEPLOYMENT_TARGET=12.0`, but the app has only been launched on the current macOS version. GPUI Metal behavior and runtime Objective-C selectors are not fully guarded by compile-time availability checks, so the advertised floor may permit installation on a system where Pulse cannot launch or complete its core flow.

## Expected Behavior
The declared minimum is verified with the signed release app on macOS 12, or it is raised to the oldest version that Pulse can honestly support.

## Steps To Reproduce
1. Install a signed, notarized Pulse release artifact on clean macOS 12 hardware or a VM with suitable graphics support.
2. Launch Pulse and verify the GPUI window renders.
3. Exercise the storage picker, library open, device enumeration, and playback where hardware access is available.
4. If any required API is unavailable, raise the deployment target and `LSMinimumSystemVersion` together.

## Relevant Code
- `script/bundle-mac`: defines `MINIMUM_SYSTEM_VERSION`, exports `MACOSX_DEPLOYMENT_TARGET`, and writes `LSMinimumSystemVersion`.
- `docs/macos-release.md`: records that 12.0 is conservative rather than measured.

## Environment
- OS: macOS 12.x target; current development OS has passed
- Device / DAC: any built-in output for launch validation; Matrix Mini-i Pro 4 for full playback validation
- Input file format: supported PCM file
- Pulse version: 0.1.0 and current mainline

## Verification
The release script and documentation were inspected. No macOS 12 machine or VM has been used yet, so this remains an explicit validation gap rather than a reproduced crash.
