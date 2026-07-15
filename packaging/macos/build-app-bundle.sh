#!/usr/bin/env bash
set -euo pipefail

# Assemble cmux.app from a release build, bundling the Homebrew GTK4 dylib
# stack so the app runs on a Mac without Homebrew installed.
#
# PORT STATUS: authored on Linux, NOT yet run on a Mac. dylibbundler behavior,
# codesigning identity, and the exact GTK4 dylib set must be validated on
# macOS. See specs/cmux-macos-extensibility.html Phase 5.
#
# Prereqs (on the build Mac):
#   - `cargo build --release` has produced target/release/{cmux-app,cmux}
#   - dylibbundler  (brew install dylibbundler) to vendor GTK4 dylibs
#   - a "Developer ID Application" signing identity in the keychain (for
#     distribution outside the App Store), or omit --sign for a local build
#
# Usage: packaging/macos/build-app-bundle.sh [SIGN_IDENTITY]

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

SIGN_IDENTITY="${1:-}"
APP="dist/cmux.app"
BIN="target/release/cmux-app"
CLI="target/release/cmux"

if [ ! -x "$BIN" ]; then
    echo "ERROR: $BIN not found. Run: cargo build --release" >&2
    exit 1
fi

echo "==> Assembling $APP ..."
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"

cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
cp "$BIN" "$APP/Contents/MacOS/cmux-app"
# Ship the CLI alongside the app binary; the Homebrew formula symlinks it onto
# PATH (see cmux.rb).
cp "$CLI" "$APP/Contents/MacOS/cmux"

# Icon: convert the existing PNG icon set to .icns if not already present.
# (iconutil expects a .iconset dir; left as a Phase 5 TODO — a placeholder
# name is referenced by Info.plist's CFBundleIconFile.)
if [ -f "packaging/macos/cmux.icns" ]; then
    cp packaging/macos/cmux.icns "$APP/Contents/Resources/cmux.icns"
else
    echo "==> NOTE: packaging/macos/cmux.icns missing — bundle will use a default icon."
fi

# Vendor the GTK4 dylib closure into Contents/Frameworks and rewrite the
# binary's load paths to @executable_path/../Frameworks so the .app is
# self-contained.
if command -v dylibbundler >/dev/null 2>&1; then
    echo "==> Bundling dylibs with dylibbundler ..."
    dylibbundler \
        --create-dir \
        --bundle-deps \
        --dest-dir "$APP/Contents/Frameworks" \
        --install-path "@executable_path/../Frameworks" \
        --fix-file "$APP/Contents/MacOS/cmux-app"
else
    echo "==> WARNING: dylibbundler not found — the .app will depend on a" >&2
    echo "    Homebrew GTK4 install at runtime. brew install dylibbundler." >&2
fi

if [ -n "$SIGN_IDENTITY" ]; then
    echo "==> Codesigning with: $SIGN_IDENTITY"
    codesign --force --deep --options runtime \
        --sign "$SIGN_IDENTITY" "$APP"
    codesign --verify --deep --strict --verbose=2 "$APP"
else
    echo "==> Skipping codesign (no identity passed). Ad-hoc signing for local run:"
    codesign --force --deep --sign - "$APP" || true
fi

echo "==> Built $APP"
