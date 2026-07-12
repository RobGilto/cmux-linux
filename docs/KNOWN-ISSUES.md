# Known Issues & Honest Edges

Living document — updated as edges are found or closed. Last review: 2026-07-11.

## Closed (fixed on `roadmap/agentic-backbone`)

- ~~NVIDIA needs `GDK_DEBUG=gl-prefer-gl` typed by hand~~ — the binary now
  self-configures at startup (`src/platform.rs`); `system.identify` reports
  what was applied, `cmux doctor` verifies it.
- ~~Nested agents misfile sessions unless `CLAUDE_CODE_*` is stripped
  manually~~ — stripped in-process before any shell spawns (`[env] strip`).
- ~~Background/restored workspaces have empty `read-text` and no shells~~ —
  realize-sweep at session restore; every workspace's surfaces are live.
- ~~Terminal title updates dropped (P0)~~ — `SET_TITLE` handled; titles in
  `surface.list` + `surface.title` events, background workspaces included.
- ~~Workspace UUIDs change across restarts~~ — restore reuses persisted uuids.
- ~~A GL failure in one pane killed the whole app (`exit(1)`)~~ — pane is
  disabled, `surface.error` emitted, app survives.
- ~~Splitting or closing next to a pane with a live process (shell, or an
  agent TUI like claude/pi) silently killed it~~ — the previous "re-realize
  after reparent allocates a fresh ghostty surface" workaround wasn't cosmetic:
  it was a real process kill (verified: env vars, cwd, and scrollback were all
  wiped by any split/close that reparented an existing pane). Fixed by
  re-exporting `ghostty_surface_display_realized`/`_unrealized` from the
  vendored ghostty submodule (`ghostty/src/apprt/embedded.zig`, `myc task #1`,
  see `docs/phase-c-plan.md` §1) and wiring cmux's GLArea realize/unrealize
  handlers (`src/ghostty/surface.rs`) to preserve the live surface across a
  reparent instead of freeing it. `PRESERVE_ON_UNREALIZE`
  (`src/ghostty/callbacks.rs`) marks exactly which panes' unrealize should
  preserve vs. genuinely free, so real pane closes still terminate their
  process correctly.
- ~~`cmux close`/`spawn` (no `--id`) could silently act on a stale, unrelated
  pane~~ — `SplitEngine::active_pane_id` ("the active pane" these commands
  target) was only ever updated by cmux's own actions (split, close,
  Ctrl+Shift+arrow nav, `focus-surface`), never by real GTK focus changes
  like a plain mouse click or Tab. Clicking into a pane changed what received
  keystrokes but left `active_pane_id` pointing at whatever was true before —
  so typing `cmux close` into the pane you'd just clicked into could close a
  different pane instead, or spawn could split the wrong one. Fixed by
  syncing `active_pane_id` (and the `active-pane` CSS class) inside the same
  `EventControllerFocus::connect_enter` handler that already keeps Ghostty's
  and the clipboard's focus state in sync (`src/ghostty/surface.rs`); guarded
  with `try_borrow_mut` since this signal can fire reentrantly from inside
  split/close's own `grab_focus()` call while a socket handler still holds
  `AppState`'s borrow.

## Open

### Rendering / GL
- **llvmpipe (headless CI) is not the NVIDIA path.** The headless CI job
  proves the socket API, not rendering. Render correctness is verified on
  real hardware (`scripts/fleet-smoke.sh --screenshot`).

### Orchestration
- **`cmux top` surface↔process matching is exact only for agent panes**
  (via `CMUX_PANE` env). Plain shells match by PTY creation order — correct
  in practice, but a heuristic (rows carry `matched_by: "env" | "order"`).
- **Workspace groups are labels, not entities** (`set-group`/`list-groups`).
  The sidebar does not yet render collapsible colored group headers; use
  status pills (`set-status --color`) for visual identity meanwhile.
- **Agent resume**: exact for Claude (SessionStart hook). pi resumes via
  `pi -c` (most recent session in cwd). Codex resumes only if an id was
  reported. Gemini always starts fresh (no resume CLI surface).

### Integration tests
- **Much of `tests_v2/` is macOS-harness, not Linux-verified.** The suites
  were inherited from macOS cmux and depend on protocol this port doesn't
  implement: `app.focus_override.set` (notifications suite), identify
  caller-context refs (`test_cli_identify_ref_resolution`), CLI flags the
  Linux CLI lacks (`test_workspace_relative`, `test_cli_id_format_defaults`),
  and macOS close-selection semantics (close selects *next* there, *previous*
  here). These fail for parity reasons, not regressions.
- **The Linux-verified set** (all green): `test_orchestration_v3.py`,
  `scripts/fleet-smoke.sh`, and the 80 Rust unit tests. CI runs exactly this
  set headlessly. Porting the remaining harness expectations is tracked as
  follow-up work.
- Known-flaky visual test: "D12" VM failure (see TODO.md) — quarantined,
  not silently green.

### Upstream
- This fork carries the full contribution stack (see MY-CONTRIBUTIONS.md).
  Upstream (tcsenpai/cmux 0.1.0) moves slowly; PRs are offered but nothing
  here blocks on acceptance.
