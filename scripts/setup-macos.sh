#!/usr/bin/env bash
set -e

# macOS dev-environment bootstrap — the counterpart to scripts/setup-linux.sh.
#
# STATUS: authored on Linux as part of the macOS extensibility port and NOT yet
# run on a Mac. The Homebrew package names and the ghostty `zig build` flags
# below are the expected macOS equivalents of the Linux setup, but must be
# validated on real macOS hardware (see specs/cmux-macos-extensibility.html,
# Phase 1). Treat any failure here as "verify the flag/package on macOS", not
# "the Linux build regressed".

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v brew &>/dev/null; then
    echo "ERROR: Homebrew is required. Install it from https://brew.sh and re-run."
    exit 1
fi

# Install system dependencies required for cargo build:
#   - gtk4           GTK4 runtime + dev headers (gtk4-rs needs them; provides
#                    the Quartz GDK backend on macOS)
#   - pkg-config     build.rs discovers gtk4 link flags through it
#   - zig            builds ghostty-internal.a from the ghostty submodule
#   - oniguruma      regex engine ghostty links (build.rs resolves it via
#                    `brew --prefix oniguruma`)
#   - llvm           provides libclang so bindgen can parse ghostty.h
#     (macOS ships clang but not always the libclang dylib bindgen needs)
echo "==> Installing build dependencies via Homebrew..."
brew install gtk4 pkg-config zig oniguruma llvm

# bindgen needs to find libclang; point it at Homebrew's llvm if the system
# one isn't discoverable.
if [ -z "${LIBCLANG_PATH:-}" ] && [ -d "$(brew --prefix llvm)/lib" ]; then
    echo "==> Hint: export LIBCLANG_PATH=\"$(brew --prefix llvm)/lib\" if bindgen fails to find libclang"
fi

echo "==> Refreshing submodules..."
git submodule update --init --force ghostty
git submodule update --init agent-browser

if [ ! -f "ghostty/build.zig" ]; then
    echo "ERROR: ghostty submodule not initialized. Run: git submodule update --init --recursive"
    exit 1
fi

echo "==> Building ghostty-internal.a for macOS..."
cd ghostty
# NOTE: the Linux setup passes -Dgtk-x11=true -Dgtk-wayland=true. Those are
# Linux windowing backends and do not apply on macOS (the GDK backend is
# Quartz). We keep -Dapp-runtime=none (cmux embeds ghostty's renderer, not its
# app runtime) and let zig target the host macOS toolchain. If the fork's
# GHOSTTY_PLATFORM_GTK4 arm needs an explicit build flag on macOS, add it here
# once validated on hardware.
zig build \
    -Dapp-runtime=none \
    -Doptimize=ReleaseFast \
    -Dcpu=baseline

echo "==> ghostty-internal.a built at: $(pwd)/zig-out/lib/ghostty-internal.a"
ls -lh zig-out/lib/ghostty-internal.a
