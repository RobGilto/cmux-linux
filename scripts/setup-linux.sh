#!/usr/bin/env bash
set -e

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Install system dependencies required for cargo build:
#   - GTK4 dev headers (gtk4-rs needs them)
#   - libclang (bindgen needs it to parse ghostty.h)
#   - libc++ + libc++abi (ghostty's bundled C++ deps link against libc++,
#     not libstdc++ — see build.rs for the ABI rationale)
echo "==> Checking system dependencies..."
if ! pkg-config --exists gtk4 2>/dev/null; then
    echo "==> Installing GTK4 development headers..."
    if command -v apt-get &>/dev/null; then
        sudo apt-get install -y libgtk-4-dev libclang-dev libc++-dev libc++abi-dev
    elif command -v dnf &>/dev/null; then
        sudo dnf install -y gtk4-devel clang-devel libcxx-devel libcxxabi-devel
    elif command -v pacman &>/dev/null; then
        sudo pacman -S --noconfirm gtk4 clang libc++ libc++abi
    else
        echo "ERROR: Cannot install GTK4 dev headers automatically."
        echo "Please install: libgtk-4-dev libclang-dev libc++-dev libc++abi-dev (Debian/Ubuntu)"
        echo "             or gtk4-devel clang-devel libcxx-devel libcxxabi-devel (Fedora)"
        echo "             or gtk4 clang libc++ libc++abi (Arch)"
        exit 1
    fi
fi

echo "==> Refreshing submodules..."
# --force handles the case where an existing ghostty checkout points at the
# old pinned SHA (4845e82d) that is no longer reachable on
# manaflow-ai/ghostty. agent-browser is the vercel-labs/agent-browser daemon
# crate; restored as a workspace member after the adversarial review.
git submodule update --init --force ghostty
git submodule update --init agent-browser

echo "==> Building libghostty.a from ghostty submodule..."
cd ghostty

# Verify submodule is initialized
if [ ! -f "build.zig" ]; then
    echo "ERROR: ghostty submodule not initialized. Run: git submodule update --init --recursive"
    exit 1
fi

zig build \
    -Dapp-runtime=none \
    -Doptimize=ReleaseFast \
    -Dcpu=baseline \
    -Dgtk-x11=true \
    -Dgtk-wayland=true

echo "==> ghostty-internal.a built at: $(pwd)/zig-out/lib/ghostty-internal.a"
ls -lh zig-out/lib/ghostty-internal.a
