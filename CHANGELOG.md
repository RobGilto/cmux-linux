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

### Fixed (adversarial review round 1)

- **APP_ID** changed from `io.cmux.App` to `com.cmux_lx.terminal` in
  `src/main.rs` to match the freedesktop ID used by all packaging
  metadata. Without this, GNOME/KDE did not associate the running
  app with its `.desktop` entry — notifications, taskbar icon, and
  DBusActivatable all silently broke.
- **agent-browser is now optional in packaging.** `build-deb.sh`,
  `build-rpm.sh`, and `cmux.spec` no longer fail when the binary is
  absent. Set `CMUX_AGENT_BROWSER_REQUIRED=1` in CI to keep the old
  fail-closed behavior.
- **`build.rs`** now has `cargo:rerun-if-changed` directives for
  `ghostty-internal.a` and `glad.o`; the previous directives pointed
  at source files that `build.rs` did not compile.
- **`build.rs`** also blocks the two missing surface-lifecycle
  exports (`ghostty_surface_display_realized` / `..._unrealized`)
  via bindgen, so a future caller cannot compile-succeed into a
  link-time `undefined reference`.
- **`scripts/setup-linux.sh`** now runs `git submodule update --init
  --force ghostty` itself — existing checkouts pointing at the
  unreachable `4845e82d` SHA could not otherwise advance.
- **README** no longer instructs users to run
  `cargo build --release -p agent-browser`; the workspace member was
  removed in Phase A and the documented command would have failed.

### Added (post-review)

- **`GHOSTTY_PLATFORM_GTK4` re-introduced into the ghostty fork.**
  Without this platform variant `ghostty_surface_new` returned null
  for every GtkGLArea-backed surface — the Linux port could create
  the GTK window but every pane immediately died at startup. The fork
  pin was moved from `manaflow-ai/ghostty` to `tcsenpai/ghostty` on
  branch `cmux-fork-gtk4-platform`; the patch adds the GTK4 platform
  arm to `PlatformTag`, `Platform`, `Platform.C`, and `Platform.init()`
  in `src/apprt/embedded.zig`, and dispatches the GTK4 case to
  `prepareContext(null)` in `OpenGL.surfaceInit`. `cmux-app` now
  reaches `set_focus(true)` on the first surface instead of dying with
  `FATAL — ghostty_surface_new returned null`.

- **`must_draw_from_app_thread=true` for the embedded apprt on Linux.**
  The renderer worker thread otherwise calls `drawFrame` directly; on
  Linux the GtkGLArea's GdkGLContext is bound only on the GTK main
  thread, so the renderer thread hits unresolved GL function pointers
  and `SIGSEGV`s at frame 0. With this declaration the renderer mails
  `redraw_surface` back to the app loop, which dispatches `.render`
  through `action_cb`, which `queue_render`s on the main thread. After
  this fix `cmux-app` reaches a steady-state render loop instead of
  crashing right after `ghostty_surface_new` succeeded.

- **`agent-browser` daemon is back in the workspace.** Added
  [`vercel-labs/agent-browser`](https://github.com/vercel-labs/agent-browser)
  as a git submodule at `agent-browser/`. The `agent-browser/cli` crate is
  a workspace member again, so `cargo build --release` produces the daemon
  at `target/release/agent-browser` alongside the rest of the binaries.
  `setup-linux.sh` initializes both the ghostty and agent-browser
  submodules; packaging requires the daemon by default
  (set `CMUX_AGENT_BROWSER_OPTIONAL=1` to opt out). Browser commands
  (`cmux browser …`) are once again fully supported.

### Fixed (adversarial review round 2)

- **SSH `IoWriteContext` leak.** Round-1 introduced a
  `ghostty_surface_free` call on GLArea unrealize. SSH manual-mode
  surfaces had been incrementing an `Arc<IoWriteContext>` and handing
  the raw pointer to ghostty as `io_write_userdata`, but `ghostty.h`
  exposes no destructor for that field. Every reparent leaked one
  strong reference. `surface.rs` now tracks the raw pointer in a
  sibling cell and `Arc::from_raw`s it before the surface is freed.
- **`GL_AREA_REGISTRY` duplicates.** GTK re-realize pushed a second
  entry for the same widget pointer; `wakeup_cb` then queue_render'd
  the area N times per wakeup. Registry now dedupes on push and
  removes the entry on unrealize.
- **`GL_TO_SURFACE` stale entries.** Map now drops the entry on
  unrealize, closing the stale-pointer window for any consumer that
  reaches into it after the GL context dies.
- **`SURFACE_PTR` use-after-free window.** Clipboard callbacks read a
  global last-active-surface pointer; the unrealize free path now
  clears `SURFACE_PTR` when it pointed at the freed surface so a
  clipboard event mid-reparent early-returns instead of dereferencing
  freed memory.
- **Docs.** `README.md` and `CLAUDE.md` quickstart no longer instruct
  users to run a manual `git submodule update` (the script does it,
  with `--force`).

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
