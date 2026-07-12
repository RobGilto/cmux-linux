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

### 1. Restore the two surface lifecycle exports — DONE

Resolved via option (a): `ghostty/src/apprt/embedded.zig` now exports
`ghostty_surface_display_realized` / `ghostty_surface_display_unrealized`,
wrapping `Surface.displayRealized()` / `displayUnrealized()`, which call
`core_surface.renderer.displayRealized()` / `displayUnrealized()` — the same
calls the native GTK4 apprt already made in
`src/apprt/gtk/class/surface.zig`. `renderer/OpenGL.zig`'s `displayRealized`
had a `@compileError` restricting it to `apprt.gtk`; it now also allows
`apprt.embedded`, matching the precedent already set by `surfaceInit`'s
"cmux fork" `apprt.embedded` branch in the same file.

`build.rs`'s bindgen blocklist for these two functions is removed;
`src/ghostty/surface.rs`'s realize/unrealize handlers now call
`ghostty_surface_display_realized`/`_unrealized` to preserve a pane's live
ghostty surface (and its pty/process) across an application-driven reparent
(split/close), instead of freeing and recreating it. This was blocking real
usage, not just cosmetic: any split or close that reparented an existing
pane's widget silently killed whatever process (shell, or an agent TUI like
claude/pi) was running in it. `PRESERVE_ON_UNREALIZE`
(`src/ghostty/callbacks.rs`) tracks per-GLArea which panes' next unrealize
should preserve vs. genuinely free, so real pane closes still terminate
correctly.

Requires rebuilding the ghostty submodule
(`zig build -Dapp-runtime=none -Doptimize=ReleaseFast -Dcpu=baseline
-Dgtk-x11=true -Dgtk-wayland=true` from `ghostty/`) whenever
`embedded.zig`/`OpenGL.zig` change — `scripts/setup-linux.sh` already does
this on a fresh checkout.

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

## Deferred low-priority items (post-adversarial review)

These were surfaced during the adversarial review on `fix/linux-port-modernize`
but deliberately not fixed in that branch.

- ~~**cmux-browser skill + packaging/CLAUDE.md vs optional
  agent-browser.**~~ Resolved post-adversarial-review by adding
  `vercel-labs/agent-browser` as a git submodule at `agent-browser/`
  and restoring `agent-browser/cli` to the cargo workspace. A normal
  `cargo build --release` now produces the daemon at
  `target/release/agent-browser`; packaging defaults to requiring it
  and only skips when `CMUX_AGENT_BROWSER_OPTIONAL=1` is set.
- **`TODO.md` and `PROJECTS.md` are historical macOS-era documents.**
  Both still describe upstream-imported items (WKWebView, Sparkle,
  Bonsplit, etc.) that no longer apply on Linux. Pre-existing — not
  introduced by Phase B. Cheap fix: add a `STATUS: historical / see
  myc epic` header to each; full triage is Phase C-adjacent.
- **Clipboard callback re-entry.** `read_clipboard_cb` snapshots
  `SURFACE_PTR` before `block_on` re-enters the GLib main loop; an
  unrealize during that window can free the surface and the cb then
  completes the request against freed memory. Preexisting — not
  introduced by the adversarial review's free-on-unrealize fix
  (which actually *narrowed* the window via `SURFACE_PTR` clear, just
  not for the local snapshot). Phase C should rework the clipboard
  path to re-check `SURFACE_REGISTRY` after `block_on`.

## Out of scope (post-port)

- AppImage + Flatpak packaging (was already labelled Phase 13 in the
  fork's `.planning/` records).
- Wayland-specific input quirks.
- Re-integration of the `agent-browser` source crate.
