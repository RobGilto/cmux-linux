#!/usr/bin/env bash
set -euo pipefail

# Package a built, signed cmux.app into a distributable .dmg, then (optionally)
# notarize and staple it.
#
# PORT STATUS: authored on Linux, NOT yet run on a Mac. hdiutil layout,
# notarytool credentials, and stapling must be validated on macOS. See
# specs/cmux-macos-extensibility.html Phase 5.
#
# Prereqs (on the build Mac):
#   - packaging/macos/build-app-bundle.sh has produced dist/cmux.app (signed)
#   - for notarization: `xcrun notarytool store-credentials` has saved a
#     keychain profile; pass its name as $NOTARY_PROFILE
#
# Usage: [NOTARY_PROFILE=cmux-notary] packaging/macos/build-dmg.sh

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

APP="dist/cmux.app"
DMG="dist/cmux-0.1.0.dmg"
STAGING="dist/dmg-staging"

if [ ! -d "$APP" ]; then
    echo "ERROR: $APP not found. Run packaging/macos/build-app-bundle.sh first." >&2
    exit 1
fi

echo "==> Staging DMG contents ..."
rm -rf "$STAGING" "$DMG"
mkdir -p "$STAGING"
cp -R "$APP" "$STAGING/"
# Drag-to-install affordance: a symlink to /Applications in the DMG window.
ln -s /Applications "$STAGING/Applications"

echo "==> Creating $DMG ..."
hdiutil create \
    -volname "cmux" \
    -srcfolder "$STAGING" \
    -ov -format UDZO \
    "$DMG"

if [ -n "${NOTARY_PROFILE:-}" ]; then
    echo "==> Submitting $DMG for notarization (profile: $NOTARY_PROFILE) ..."
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
    echo "==> Stapling notarization ticket ..."
    xcrun stapler staple "$DMG"
    xcrun stapler validate "$DMG"
else
    echo "==> Skipping notarization (set NOTARY_PROFILE to enable)."
fi

rm -rf "$STAGING"
echo "==> Built $DMG"
