# My cmux-linux contributions

Private backup of my fixes and features for [cmux for Linux](https://github.com/tcsenpai/cmux-linux)
(a Rust + GTK4 port of [manaflow-ai/cmux](https://github.com/manaflow-ai/cmux)).
This branch (`all-contributions`) is the full feature stack plus the NVIDIA
packaging fix, merged into one buildable branch. Submitted upstream as PRs #1–#8
on tcsenpai/cmux-linux; kept here in case they aren't merged.

Starting point was upstream 0.1.0, which on this machine (Arch/Hyprland, NVIDIA
RTX 3060 Ti) would not render, could not be driven over its socket, and crashed
on workspace close. After this work it passes all 16 of the IndyDevDan
`learning-cmux-with-agents` tier-1–3 orchestration prompts.

## What's here (each was one upstream PR / branch)

| Branch | PR | What |
|--------|----|----|
| `nvidia-gl-context-fix` | #1 | `GDK_DEBUG=gl-prefer-gl` on NVIDIA in the deb/rpm/AppImage wrappers — GDK binds GLES by default and can't create the desktop-GL context libghostty needs (blank window / "Unable to create a GL context"). Also README browser-syntax + troubleshooting fixes. |
| `fix-socket-terminal-io` | #2 | Made `send-text`/`send-key`/`read-text` actually drive the terminal (they were silent no-ops — the split tree's surface pointer was a never-backfilled null; `send-key` had a `len==1` guard; `read-text` was a stub). Plus `read-text --scrollback`, honor `split --id`, fix a double-free on workspace/pane close, and `surface.health` across all workspaces. |
| `feat-events-notify` | #3 | Event stream (`cmux events`) + `cmux notify`, `surface.bell`/`notification.created` events. Also fixed a `ghostty.h` action-tag ABI skew (missing `SET_TAB_TITLE` made every tag from 33 up off-by-one vs the Zig enum — bells were silently dropped). |
| `feat-declarative-layouts` | #4 | `cmux new-workspace --name/--cwd/--layout <json>` — layouts-as-code with per-terminal cwd and startup commands. |
| `feat-status-board` | #5 | Sidebar status pills + progress bars + log lines: `cmux set-status`/`set-progress`/`log`. |
| `feat-agent-sessions` | #6 | Native agent sessions with resume: `cmux new-workspace --agent claude`, `cmux hooks setup`, `cmux agent-sessions`. Captures the agent's session id via a SessionStart hook and, on restart, re-runs `cd <proj>; claude --resume <id>`. Verified end-to-end: a live Claude session's conversation survived a cmux restart. |
| `fix-surface-cwd` | #7 | Start shells in their working directory via `surface_config.working_directory` instead of a deferred `cd` — fixes split children / layout panes / agents opening in `$HOME`. |
| `fix-min-pane-size` | #8 | Reject splits that would make a pane too small to render (axis-aware; some TUI agents crash below ~20 cols). |

`fix-min-pane-size` is the tip of the linear stack (contains #2–#8);
`all-contributions` = that + `nvidia-gl-context-fix` merged.

## Build & run

Prereqs: Rust, GTK4 dev headers, clang, libc++/libc++abi, and **Zig 0.15.2**
for libghostty (Arch ships 0.16; grab the 0.15.2 tarball).

```bash
./scripts/setup-linux.sh                       # submodules + build libghostty
cargo build --release --bin cmux-app --bin cmux
GDK_DEBUG=gl-prefer-gl ./target/release/cmux-app   # NVIDIA needs the flag
```

The `cmux` CLI talks to the running app over `$XDG_RUNTIME_DIR/cmux/cmux.sock`.
See the `cmux-linux` and `cmux-fleet` skills (in `~/.claude/skills/`) for driving
it. Key gotchas: NVIDIA needs `GDK_DEBUG=gl-prefer-gl`; launch from a clean env
(strip `CLAUDE_CODE_*`) so nested agents don't misfile sessions; maximize the
window before running TUI-agent grids.
