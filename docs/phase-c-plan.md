# Phase C — Upstream Sync (planning record)

Status: **in progress** on branch `fix/linux-port-modernize`.

Phase A delivered a buildable Linux port against the current
`manaflow-ai/ghostty` main. Phase B removed ~232 MB of dead macOS code,
web bloat, and stale tooling. Phase C closes the gap with upstream
`manaflow-ai/cmux@main`.

This document is the source of truth for the remaining work. Per-item
tracking lives in Mycelium (epic #1).

## Divergence baseline (as of 2026-06-06)

- Merge-base SHA: `e0e0e352` (2026-03-22)
- Fork tip: `d0089f08` after Phase B (was `17d50882`)
- Upstream tip: `97cf2139`
- Commits ahead (fork-only): 361
- Commits behind upstream: 2877

The fork forked just before the upstream Mobile/iOS SPM replay refactor.
Most upstream commits since touch files that no longer exist in this
fork (`Sources/`, `Resources/`, `*.pbxproj`, `vendor/bonsplit/`,
`homebrew-cmux/`), so a rebase will be mechanically clean for the
biggest chunk.

## Conflict zones

| File | Class | Tracking |
|---|---|---|
| `daemon/remote/cmd/cmuxd-remote/main.go` | textual conflict — upstream PTY hardening (`25a710d1`, `d1a2ba74`, `6735ace3`) vs fork's `489d61fe` | myc task #3 |
| `ghostty.h` | textual conflict with upstream `67d003db` (bridge drift fix) | folded into myc task #1 |
| `ghostty` submodule pointer | fork carried 3 GTK4-platform patches; need re-forward over the upstream main line | myc task #1 |
| `src/socket/handlers.rs`, `src/socket/commands.rs` | semantic drift — upstream extracted `CmuxControlSocket` package | myc task #4 |
| `src/browser.rs`, `src/cli/format.rs` | semantic drift — upstream passkey/WebAuthn, omnibar UTF-16, hidden screenshots | folded into myc task #2 |
| `src/cli/discovery.rs` | path drift — upstream `efe37a9c` moved socket/markers/password out of Application Support | myc task #5 |

## What landed in Phase A

- Cargo workspace stripped to `["."]`.
- Ghostty submodule re-pinned to current manaflow-ai/ghostty main
  (`5825120677`); the previously-pinned `4845e82d` is unreachable.
- `build.rs` rewritten:
  - link `ghostty-internal.a` by absolute path (combined archive,
    no `lib` prefix);
  - drop separate simdutf.o / libhighway.a / stubs.o links (now
    bundled);
  - link `libc++` + `libc++abi` instead of `libstdc++` (zig builds
    ghostty's C++ deps against libc++).
- `scripts/setup-linux.sh` updated to install `libcxx-devel` /
  `libc++-dev`.
- `src/ghostty/surface.rs` stubbed the two C exports that were dropped
  with the missing `4845e82d` ghostty branch.
- README + CONTRIBUTING + packaging (license, RPM wrapper) cleaned up.

## What landed in Phase B

- Entire macOS Swift tree (`Sources/`, XCTest bundles, Xcode project,
  Package.swift, Resources/, AppIcon.icon/, Assets.xcassets/, etc.)
  deleted.
- `vendor/bonsplit/` and `homebrew-cmux/` submodule entries removed
  from `.gitmodules`.
- macOS-only scripts (`reload*.sh`, `sparkle_*.sh`, `setup.sh`, etc.)
  removed.
- `web/` (Next.js marketing site) + checked-in `node_modules/` (~219 MB)
  removed.
- Duplicate / now-redundant build artefacts (`glslang_stub.{c,o}`,
  `stubs.{c,o}`) removed.
- `CLAUDE.md` rewritten for Linux/Rust/GTK4.
- `CHANGELOG.md` reset with Linux-port history; the upstream macOS
  0.62.2 entries were wholly unrelated to this fork.
- `packaging/rpm/cmux.spec` and `packaging/scripts/build-rpm.sh` now
  install the X11 GDK-backend wrapper, mirroring `build-deb.sh`.

## What's landed so far in Phase C

- `src/cli/discovery.rs` — preference order swapped so `CMUX_SOCKET_PATH`
  wins over `CMUX_SOCKET`, matching upstream `054cc9ff`. Both still
  honoured for back-compat.

## Open work (the contents of myc epic #1)

### 1. Restore the two surface lifecycle exports

`src/ghostty/surface.rs` has two `TODO(cmux-linux)` stubs for
`ghostty_surface_display_realized` / `..._unrealized`. Resolve by one of:

- (a) Open a manaflow-ai/ghostty branch that re-exports
  `renderer.displayRealized()` / `displayUnrealized()` through
  `src/apprt/embedded.zig`, then bump the parent-repo submodule
  pointer; **or**
- (b) Replace the call sites with whatever lifecycle hook upstream
  currently uses for GTK4 GLArea reparent.

(a) is cheaper but requires push access to the manaflow-ai/ghostty
fork. (b) is more durable.

### 2. Finish Tier-2 browser RPC dispatch

`src/socket/handlers.rs` returns `not_implemented` for ~40 browser
methods. `src/socket/commands.rs` lists them. Wire each through to
the agent-browser CDP daemon. While doing so, fold in upstream's
passkey/WebAuthn additions, the omnibar UTF-16 length fix, and the
hidden-screenshot routing.

This is the single biggest chunk of work — likely a multi-session
effort by itself.

### 3. Resolve `cmuxd-remote/main.go` conflict

Three upstream commits hardened the SSH PTY path:
- `25a710d1` — detachable SSH PTY persistence
- `d1a2ba74` — harden remote websocket PTY sessions
- `6735ace3` — E2BIG fix

Fork has `489d61fe` modifications. Manual merge required.

### 4. Backport `CmuxControlSocket` extraction

Upstream commits `fd9aa838`, `a30b11e4`, `5d7ceb40`, `954911ff`
extracted `CmuxControlSocket` / `CmuxSocketControl` Swift packages
from `TerminalController`. The wire-level command set was reorganized.
The fork's `src/socket/handlers.rs` and `commands.rs` need to be
re-validated against the new contract; rename/add methods to keep
the Python `tests_v2/` suite passing.

### 5. Sync remaining path discovery (low priority)

`efe37a9c` moved socket / markers / password out of macOS Application
Support. Phase C already did the socket env-var preference swap; the
marker file + password file paths haven't been audited.

## Out of scope (post-port)

- AppImage + Flatpak packaging (was already labelled Phase 13 in the
  fork's `.planning/` records).
- Wayland-specific input quirks.
- Re-integration of the `agent-browser` source crate.
