# Changelog

All notable changes to **cmux for Linux** are documented here.

This is the changelog of the Linux port (the Rust + GTK4 rewrite by
[@bradwilson331](https://github.com/bradwilson331) and contributors). It does
not mirror the upstream macOS changelog at
[manaflow-ai/cmux](https://github.com/manaflow-ai/cmux). For features that
exist on both ports, see the upstream changelog for macOS history.

## [Unreleased] — fix/linux-port-modernize

### Fixed
- **Cargo workspace** referenced the never-tracked `agent-browser/cli`
  member; cleaned to `members = ["."]`.
- **Ghostty submodule** was pinned to `4845e82d` which is no longer
  reachable on `manaflow-ai/ghostty`. Submodule pointer moved to the
  current main tip (`5825120677`).
- **build.rs** now links against `ghostty-internal.a` (combined archive
  that bundles simdutf, libhighway, glslang, dcimgui, SPIRV-Cross) instead
  of the historical `libghostty.a` + separate object-file dance.
- **C++ ABI** switched from `libstdc++` to `libc++ + libc++abi` to match
  what zig builds the bundled C++ deps against.
- **Surface-lifecycle TODOs** (`ghostty_surface_display_realized` /
  `..._unrealized`) stubbed in `src/ghostty/surface.rs` — these C exports
  are missing on current ghostty main; deferred to Phase C.
- **README** install steps now point at `setup-linux.sh` and document
  the GTK4 + libclang + libc++ system-dependency requirements.
- **CONTRIBUTING.md** rewritten for Linux (it was 100% macOS Xcode).
- **License declarations** reconciled to AGPL-3.0-or-later in
  `packaging/rpm/cmux.spec` (was "Proprietary") and
  `packaging/desktop/com.cmux_lx.terminal.metainfo.xml` (was "MIT").

### Removed
- Entire macOS Swift source tree:
  `Sources/`, `cmuxTests/`, `cmuxUITests/`, `GhosttyTabs.xcodeproj/`,
  `CLI/cmux.swift`, `Resources/`, `AppIcon.icon/`, `Assets.xcassets/`,
  `Package.swift`, `Package.resolved`, `cmux.entitlements`,
  `cmux-Bridging-Header.h`.
- macOS-only scripts: `setup.sh`, `reload*.sh`, `rebuild.sh`,
  `build-ghostty-cli-helper.sh`, `build-sign-upload.sh`,
  `download-prebuilt-ghosttykit.sh`, `sparkle_*.sh`,
  `derive_sparkle_public_key.swift`, `create-virtual-display.m`,
  `generate_*_icon.py`, `launch-tagged-automation.sh`,
  `release_asset_guard.*`, `run-e2e.sh`, `run-tests-v1.sh`,
  `run-tests-v2.sh`, `smoke-test-ci.sh`, `test-unit.sh`.
- `vendor/bonsplit/` and `homebrew-cmux/` submodules (macOS-only) and
  their `.gitmodules` entries.
- `web/` (vercel-deployed marketing site, ~29 MB) and its
  `node_modules/` (~190 MB checked-in by mistake), `package.json`,
  `bun.lock`, `.vercelignore`.
- Stale tests: `tests/` (Swift-shaped Python tests targeting the macOS
  app); `tests_v2/` retained for CI socket-protocol regression coverage.
- Stale build artefacts: `glslang_stub.c` / `.o` (duplicate of `stubs.c`
  first half), `stubs.c` / `.o` (now bundled in ghostty-internal.a).
- `design/` (macOS `.icon` source asset).

### Carried forward from Phase 12.1
- X11 GDK backend forced on NVIDIA proprietary via
  `packaging/scripts/cmux-app-wrapper.sh` (used by .deb; **TODO** for .rpm).
- Baseline x86-64 target (`.cargo/config.toml`) to keep AVX-512-only
  CPUs out of the binary so VM builds run on older hosts.
- FHS lookup for `agent-browser` binary at
  `~/.local/share/cmux/bin/agent-browser` (since the source crate is
  not yet re-included in this repo).

### Known issues / Phase C work
- Pane reparent / display-move may leak GL resources until Phase C
  restores or replaces the surface display lifecycle hooks.
- The 40+ "tier-2 stub" browser RPC verbs in `src/socket/handlers.rs`
  return `not_implemented`. Tracked for Phase C.
- The fork is ~2877 commits behind upstream `manaflow-ai/cmux@main` and
  has 5 conflict / drift zones (cmuxd-remote PTY, ghostty.h, socket
  protocol packages, browser surface API, socket-discovery paths).
  Phase C will resolve these.

## [0.1.0] — fork start (~2026-03)

Initial Linux Rust + GTK4 port of cmux. See the planning records under
`.planning/` for milestone-level history of the port itself.
