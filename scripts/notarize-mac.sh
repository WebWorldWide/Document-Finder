#!/usr/bin/env bash
# Build, sign (Developer ID), notarize and staple the macOS app locally.
#
# One-time setup (stores notary credentials in your login keychain; you type
# an app-specific password from appleid.apple.com → Sign-In and Security):
#
#   xcrun notarytool store-credentials document-finder \
#     --apple-id YOUR_APPLE_ID --team-id RSDJXF7DXN
#
# Then:  scripts/notarize-mac.sh
#
# Env overrides: APPLE_SIGNING_IDENTITY (default: the first "Developer ID
# Application" identity in the keychain), NOTARY_PROFILE (default
# document-finder), PROFILE (cargo profile, default release).
set -euo pipefail
cd "$(dirname "$0")/.."

IDENTITY="${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application:[^"]*\)".*/\1/p' | head -1)}"
PROFILE_NAME="${NOTARY_PROFILE:-document-finder}"
CARGO_PROFILE="${PROFILE:-release}"
[ -n "$IDENTITY" ] || { echo "No 'Developer ID Application' identity found in the keychain."; exit 1; }
echo "==> signing as: $IDENTITY"

if ! xcrun notarytool history --keychain-profile "$PROFILE_NAME" >/dev/null 2>&1; then
  echo "No notary credentials named '$PROFILE_NAME'. Run the one-time setup at the top of this script."
  exit 1
fi

export APPLE_SIGNING_IDENTITY="$IDENTITY"
ARGS=(build --bundles app,dmg)
[ "$CARGO_PROFILE" != "release" ] && ARGS+=(-- --profile "$CARGO_PROFILE")
node node_modules/@tauri-apps/cli/tauri.js "${ARGS[@]}"

BUNDLE="src-tauri/target/$CARGO_PROFILE/bundle"
APP="$BUNDLE/macos/Document Finder.app"
DMG="$(ls -t "$BUNDLE"/dmg/*.dmg | head -1)"

echo "==> verifying signature"
codesign --verify --deep --strict --verbose=2 "$APP"
codesign -dv "$APP" 2>&1 | grep -E "Authority=Developer ID|TeamIdentifier|flags"

echo "==> notarizing $DMG (usually 1–5 minutes)"
xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE_NAME" --wait

echo "==> stapling"
xcrun stapler staple "$DMG"
xcrun stapler staple "$APP"

echo "==> Gatekeeper check"
spctl --assess --type execute --verbose=2 "$APP"
spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG" || true
echo "Done: $DMG"
