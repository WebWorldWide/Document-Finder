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
# (Without a stored profile it asks for the Apple ID + app-specific password.)
#
# Env overrides: APPLE_SIGNING_IDENTITY (default: the first "Developer ID
# Application" identity in the keychain), NOTARY_PROFILE (default
# document-finder), PROFILE (cargo profile, default release).
set -euo pipefail
cd "$(dirname "$0")/.."

# Make cargo reachable even when rustup's shims aren't on PATH (e.g. a
# Homebrew rustup without ~/.cargo/bin).
if ! command -v cargo >/dev/null 2>&1; then
  for d in "$HOME/.cargo/bin" "$HOME"/.rustup/toolchains/stable-*/bin; do
    [ -x "$d/cargo" ] && export PATH="$d:$PATH" && break
  done
fi
command -v cargo >/dev/null 2>&1 || { echo "cargo not found — install Rust via https://rustup.rs"; exit 1; }

IDENTITY="${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application:[^"]*\)".*/\1/p' | head -1)}"
PROFILE_NAME="${NOTARY_PROFILE:-document-finder}"
CARGO_PROFILE="${PROFILE:-release}"
[ -n "$IDENTITY" ] || { echo "No 'Developer ID Application' identity found in the keychain."; exit 1; }
echo "==> signing as: $IDENTITY"

# Notary auth: a stored keychain profile if there is one; otherwise ask for
# the Apple ID's app-specific password right now (nothing is stored).
TEAM_ID="${APPLE_TEAM_ID:-$(printf '%s' "$IDENTITY" | sed -n 's/.*(\([A-Z0-9]*\)).*/\1/p')}"
if xcrun notarytool history --keychain-profile "$PROFILE_NAME" >/dev/null 2>&1; then
  NOTARY_AUTH=(--keychain-profile "$PROFILE_NAME")
else
  APPLE_ID="${APPLE_ID:-}"
  if [ -z "$APPLE_ID" ]; then
    read -r -p "Apple ID email for notarization: " APPLE_ID
  fi
  read -r -s -p "App-specific password for $APPLE_ID (from appleid.apple.com): " APP_PW
  echo
  NOTARY_AUTH=(--apple-id "$APPLE_ID" --team-id "$TEAM_ID" --password "$APP_PW")
  echo "==> checking those credentials with Apple"
  if ! xcrun notarytool history "${NOTARY_AUTH[@]}" >/dev/null; then
    echo "Apple rejected those credentials. Use an app-specific password (not your Apple ID password)."
    exit 1
  fi
fi

export APPLE_SIGNING_IDENTITY="$IDENTITY"
# SKIP_BUILD=1 notarizes the existing signed bundle instead of rebuilding.
if [ "${SKIP_BUILD:-0}" != "1" ]; then
  ARGS=(build --bundles app,dmg)
  [ "$CARGO_PROFILE" != "release" ] && ARGS+=(-- --profile "$CARGO_PROFILE")
  node node_modules/@tauri-apps/cli/tauri.js "${ARGS[@]}"
fi

BUNDLE="src-tauri/target/$CARGO_PROFILE/bundle"
APP="$BUNDLE/macos/Document Finder.app"
DMG="$(ls -t "$BUNDLE"/dmg/*.dmg | head -1)"

echo "==> verifying signature"
codesign --verify --deep --strict --verbose=2 "$APP"
codesign -dv "$APP" 2>&1 | grep -E "Authority=Developer ID|TeamIdentifier|flags"

echo "==> notarizing $DMG (usually 1–5 minutes)"
xcrun notarytool submit "$DMG" "${NOTARY_AUTH[@]}" --wait

echo "==> stapling"
xcrun stapler staple "$DMG"
xcrun stapler staple "$APP"

echo "==> Gatekeeper check"
spctl --assess --type execute --verbose=2 "$APP"
spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG" || true
echo "Done: $DMG"
