<h1 align="center">cmux for Linux</h1>
<p align="center">A GPU-accelerated terminal multiplexer with tabs, splits, workspaces, browser automation, and socket CLI control — powered by Ghostty</p>

<p align="center">
  <img src="./docs/assets/main-first-image.png" alt="cmux screenshot" width="900" />
</p>

## About

cmux for Linux is a full native port of [cmux](https://github.com/manaflow-ai/cmux) (originally a macOS Swift/AppKit app) rebuilt in Rust on GTK4. It provides the same experience — tabs, splits, workspaces, notifications, browser automation, and a scriptable socket API — running natively on Linux with GPU-accelerated terminal rendering via Ghostty.

Built for developers running multiple AI coding agents (Claude Code, Codex, etc.) in parallel who need visibility into which agent needs attention and the ability to script browser interactions alongside terminal sessions.

## Features

- **GPU-accelerated terminal** — Powered by libghostty with GTK4 GtkGLArea rendering
- **Workspaces, tabs, and split panes** — Organize parallel agent sessions
- **Notification system** — Per-pane bell tracking, sidebar indicators, desktop notifications
- **In-app browser** — CDP-based browser automation with accessibility tree snapshots, element interaction, and JS evaluation via [agent-browser](https://github.com/vercel-labs/agent-browser)
- **Scriptable CLI** — `cmux` CLI with 34+ subcommands for workspaces, panes, surfaces, and browser control
- **Socket API** — v2 JSON-RPC over Unix socket with SO_PEERCRED auth
- **SSH remote workspaces** — cmuxd-remote deployment with bidirectional PTY proxy and reconnect
- **Ghostty compatible** — Reads your existing `~/.config/ghostty/config` for themes, fonts, and colors
- **Session persistence** — Atomic save/restore of full split tree topology with divider ratios

## Install

### Debian / Ubuntu (.deb)

```bash
sudo dpkg -i cmux_0.1.0_amd64.deb
sudo apt-get install -f  # install dependencies if needed
```

### Fedora / RHEL (.rpm)

```bash
sudo rpm -i cmux-0.1.0-1.x86_64.rpm
```

### Build from source

```bash
# Prerequisites:
#   - Rust toolchain (rustup)
#   - Zig 0.15.2 (mise/asdf, or download from ziglang.org)
#   - GTK4 + libclang + libc++ dev headers
#       Debian/Ubuntu:  sudo apt-get install libgtk-4-dev libclang-dev libc++-dev libc++abi-dev
#       Fedora/RHEL:    sudo dnf install gtk4-devel clang-devel libcxx-devel libcxxabi-devel
#       Arch:           sudo pacman -S gtk4 clang libc++ libc++abi
#
# setup-linux.sh runs `git submodule update --init --force ghostty` for you;
# the --force is required because the previously pinned ghostty SHA
# (4845e82d) is no longer reachable on manaflow-ai/ghostty.
./scripts/setup-linux.sh                      # builds ghostty-internal.a
cargo build --release                         # builds cmux-app, cmux, cmux-generate, agent-browser
./scripts/install-cmuxd-remote.sh             # builds + installs cmuxd-remote SSH helper
```

The `agent-browser` crate lives in the `agent-browser/` submodule
([vercel-labs/agent-browser](https://github.com/vercel-labs/agent-browser))
and is built as part of the workspace; `cargo build --release` produces it
at `target/release/agent-browser`. Browser commands (`cmux browser …`)
require this binary to be on `$PATH` or under
`~/.local/share/cmux/bin/agent-browser`.

### Quickstart

```bash
cmux launch        # find cmux-app, start it detached, wait for ping
cmux quit          # graceful shutdown (socket quit → SIGTERM → SIGKILL)
cmux doctor        # verify: socket, GL, config, session, agent CLIs, hooks
```

`cmux launch` and `cmux quit` are idempotent (launch exits 0 if already running; quit exits 0 with `not running` if not), passes `--fresh`
through to wipe the saved session, and logs the app's output to
`$XDG_STATE_HOME/cmux/launch.log`. NVIDIA GL workarounds and child-shell env
hygiene are applied **inside the binary** — no environment incantations.

### Troubleshooting: blank window / "Unable to create a GL context" on NVIDIA

On NVIDIA proprietary drivers, GDK binds the GLES API at EGL init by default
and then cannot create the desktop OpenGL context libghostty's renderer
requires. The binary now detects NVIDIA and applies `gl-prefer-gl` itself
(see `src/platform.rs`); `cmux identify` shows what was auto-applied and
`cmux doctor` checks the resulting GL context. Config override:

```toml
# ~/.config/cmux/config.toml
[launch]
gl_workaround = "auto"   # "auto" | "force" | "off"
```

If a pane still comes up blank, check `cmux doctor` and the daily log at
`$XDG_STATE_HOME/cmux/logs/`. Known remaining edges: `docs/KNOWN-ISSUES.md`.

## Browser Automation

Agents running inside cmux can discover and use browser automation via the `cmux browser` CLI:

```bash
# Open a site (https:// auto-prepended if no scheme)
cmux browser open slashdot.org            # returns surface:1 handle

# Interact with the page
cmux browser snapshot surface:1 --interactive  # accessibility tree with element refs
cmux browser click surface:1 e3               # click element by ref
cmux browser fill surface:1 e5 "search term"  # fill input field
cmux browser eval surface:1 'document.title'  # evaluate JavaScript

# Navigation
cmux browser goto surface:1 example.com
cmux browser back surface:1
cmux browser forward surface:1
cmux browser reload surface:1

# Management
cmux browser list                          # list browser surfaces
cmux browser close --surface surface:1     # close a surface
```

Browser commands default to JSON output (agents are the primary consumers). Use `--no-json` for human-readable output.

## CLI Reference

```bash
cmux --help                    # all commands
cmux browser --help            # browser subcommands

# Terminal management
cmux list-workspaces           # list all workspaces
cmux new-workspace             # create workspace
cmux list-surfaces             # list terminal surfaces
cmux split --direction horizontal  # split current pane
cmux spawn                     # add a pane via fibonacci/spiral auto-layout (no direction arg)
cmux close                     # remove the active pane (or: cmux close-surface <uuid>)
cmux list-panes                # list all panes

# System
cmux identify                  # instance info (version, platform, pid)
cmux ping                      # check connectivity
cmux raw <method> --params '{}' # send arbitrary JSON-RPC
```

### Fibonacci/spiral panel layout

`cmux spawn` adds a pane with no orientation argument, and — with no `--id`
— always continues the spiral from the **spiral tail**: the pane most
recently created by a `spawn` call, tracked independently of GTK keyboard
focus. Navigating around to inspect other panes (Ctrl+Shift+arrows,
`focus-surface`) does not redirect where the next spawn lands; only
spawning (or an explicit `--id`) moves the tail. Orientation is decided
from the target pane's own on-screen aspect ratio — split along its longer
axis (wide -> vertical divider, left/right; tall -> horizontal divider,
top/bottom) — so splitting the same pane in the same state always produces
the same result. As each new pane naturally ends up narrower/shorter than
its parent, repeated spawning produces the classic i3/dwm-style spiral.
Useful for orchestrator/lead/worker agent fan-out, where each caller just
asks for a new pane without tracking or fighting over layout state:

```bash
cmux spawn --agent claude   # pane 2
cmux spawn --agent claude   # pane 3: continues from pane 2, even if you
                             # navigated elsewhere in between
cmux spawn --agent codex    # pane 4: continues from pane 3
```

`--id <uuid>` splits a specific pane directly instead of continuing the
spiral, and becomes the new spiral tail for subsequent no-id spawns.
`cmux close` (alias for `close-surface` with no id) removes the active
pane — if that was the spiral tail, the next `spawn` falls back to
whichever pane is currently active. Pass a uuid (`close-surface <uuid>`) to
close a specific pane instead.

### Socket Path

The cmux socket is at `$XDG_RUNTIME_DIR/cmux/cmux.sock` (typically `/run/user/$UID/cmux/cmux.sock`).

Override with `CMUX_SOCKET` environment variable or `--socket` flag.

## Agent Skills

When installed via .deb or .rpm, agent skills are available at `/usr/share/cmux/skills/`:

- **cmux** — Core terminal multiplexer skill (workspaces, panes, surfaces, socket CLI)
- **cmux-browser** — Browser automation skill (open sites, interact with pages, extract data)

A `CLAUDE.md` at `/usr/share/cmux/CLAUDE.md` references skill paths so Claude Code discovers them automatically.

## Architecture

- **Language:** Rust
- **UI toolkit:** GTK4 via gtk4-rs
- **Terminal engine:** Ghostty (manaflow-ai fork) via libghostty C FFI
- **Async runtime:** tokio + glib spawn_local bridge
- **Browser automation:** agent-browser daemon with CDP protocol
- **Remote sessions:** Go daemon (cmuxd-remote) reused from macOS codebase
- **Socket protocol:** v2 JSON-RPC, wire-compatible with macOS cmux

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| Ctrl+Shift+T | New workspace |
| Ctrl+1–8 | Jump to workspace 1–8 |
| Ctrl+Shift+D | Split right |
| Ctrl+D | Split down |
| Ctrl+Shift+W | Close workspace |
| Ctrl+W | Close pane |
| Ctrl+Shift+] / [ | Next / previous workspace |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous pane |
| Ctrl+Shift+F | Find |
| Ctrl+Shift+K | Clear scrollback |

Shortcuts are configurable via TOML config file.

## Building Packages

```bash
# Build all release binaries
cargo build --release --bin cmux --bin cmux-app
# agent-browser is an external sidecar binary (not in this repo). Build it
# from https://github.com/vercel-labs/agent-browser and drop the binary at
# ~/.local/share/cmux/bin/agent-browser, or `cp` it next to your packaging
# inputs before running build-deb.sh / build-rpm.sh.

# Build .deb
./packaging/scripts/build-deb.sh

# Build .rpm
./packaging/scripts/build-rpm.sh

# Validate packages
./packaging/scripts/validate-deb.sh
./packaging/scripts/validate-rpm.sh
```

## License

This project is licensed under the GNU Affero General Public License v3.0 or later (`AGPL-3.0-or-later`).

See `LICENSE` for the full text.

## Upstream

Linux port of [cmux](https://github.com/manaflow-ai/cmux) by [manaflow-ai](https://github.com/manaflow-ai).
