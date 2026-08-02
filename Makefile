# VERSION is lazily evaluated so everyday targets (build, test, verify) do not
# depend on the packaging script being present and executable.
VERSION = $(shell ./script/bundle-mac --print-version)
APP_BUNDLE := target/release/Pulse.app
APP_ZIP = target/release/Pulse-$(VERSION)-arm64.zip
DMG = target/release/Pulse-$(VERSION)-arm64.dmg

.PHONY: build release run check test clippy fmt fmt-check verify clean-rust-stale bundle sign sign-dmg notarize notarize-app dmg release-macos check-version check-sign-credentials check-notarize-credentials

build:
	cargo build --workspace

release:
	cargo build --release --workspace

run:
	cargo run -p pulse-app

check:
	cargo check --workspace

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

verify: check test clippy fmt-check

check-version:
	@test -n "$(VERSION)" || { echo "error: could not derive workspace version from ./script/bundle-mac --print-version" >&2; exit 1; }

bundle:
	./script/bundle-mac

check-sign-credentials:
	@test -n "$$APPLE_SIGNING_IDENTITY" || { echo "error: APPLE_SIGNING_IDENTITY is required for make sign" >&2; exit 1; }

sign: check-sign-credentials
	@test -d "$(APP_BUNDLE)" || { echo "error: $(APP_BUNDLE) does not exist; run make bundle first" >&2; exit 1; }
	codesign --force --options runtime --timestamp --sign "$$APPLE_SIGNING_IDENTITY" "$(APP_BUNDLE)"
	codesign --verify --strict --verbose=2 "$(APP_BUNDLE)"

check-notarize-credentials:
	@test -n "$$APPLE_ID" || { echo "error: APPLE_ID is required for notarization" >&2; exit 1; }
	@test -n "$$APPLE_PASSWORD" || { echo "error: APPLE_PASSWORD is required for notarization" >&2; exit 1; }
	@test -n "$$APPLE_TEAM_ID" || { echo "error: APPLE_TEAM_ID is required for notarization" >&2; exit 1; }

# Notarize and staple the app itself, before it is packaged into the DMG, so a
# first launch after dragging it out of the image succeeds without network
# access. Notarizing only the DMG leaves the extracted app unstapled.
notarize-app: check-version check-notarize-credentials
	@test -d "$(APP_BUNDLE)" || { echo "error: $(APP_BUNDLE) does not exist; run make bundle first" >&2; exit 1; }
	@codesign --verify --strict --verbose=2 "$(APP_BUNDLE)" || { echo "error: $(APP_BUNDLE) is not signed; run make sign first" >&2; exit 1; }
	ditto -c -k --keepParent "$(APP_BUNDLE)" "$(APP_ZIP)"
	xcrun notarytool submit "$(APP_ZIP)" --apple-id "$$APPLE_ID" --password "$$APPLE_PASSWORD" --team-id "$$APPLE_TEAM_ID" --wait
	xcrun stapler staple "$(APP_BUNDLE)"
	xcrun stapler validate "$(APP_BUNDLE)"

dmg: check-version
	@test -d "$(APP_BUNDLE)" || { echo "error: $(APP_BUNDLE) does not exist; run make bundle first" >&2; exit 1; }
	hdiutil create -volname Pulse -srcfolder "$(APP_BUNDLE)" -ov -format UDZO "$(DMG)"

sign-dmg: check-version check-sign-credentials
	@test -f "$(DMG)" || { echo "error: $(DMG) does not exist; run make dmg first" >&2; exit 1; }
	codesign --force --timestamp --sign "$$APPLE_SIGNING_IDENTITY" "$(DMG)"
	codesign --verify --verbose=2 "$(DMG)"

notarize: check-version check-notarize-credentials
	@test -f "$(DMG)" || { echo "error: $(DMG) does not exist; run make dmg first" >&2; exit 1; }
	@codesign --verify --verbose=2 "$(DMG)" || { echo "error: $(DMG) is not signed; run make sign-dmg first" >&2; exit 1; }
	xcrun notarytool submit "$(DMG)" --apple-id "$$APPLE_ID" --password "$$APPLE_PASSWORD" --team-id "$$APPLE_TEAM_ID" --wait
	xcrun stapler staple "$(DMG)"
	xcrun stapler validate "$(DMG)"

release-macos:
	$(MAKE) bundle
	$(MAKE) sign
	$(MAKE) notarize-app
	$(MAKE) dmg
	$(MAKE) sign-dmg
	$(MAKE) notarize

# Drop accumulated dev generations of the workspace crates, keeping
# dependency artifacts warm
clean-rust-stale:
	cargo clean -p pulse-app -p pulse-engine -p pulse-cli --profile dev
