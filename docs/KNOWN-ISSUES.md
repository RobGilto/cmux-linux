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

## Open

### Rendering / GL
- **llvmpipe (headless CI) is not the NVIDIA path.** The headless CI job
  proves the socket API, not rendering. Render correctness is verified on
  real hardware (`scripts/fleet-smoke.sh --screenshot`).
- **Re-realize after reparent allocates a fresh ghostty surface** (the
  `ghostty_surface_display_realized` export was dropped upstream); scrollback
  in that pane survives, but renderer state is rebuilt. Cosmetic flicker at
  worst; tracked for the next ghostty submodule bump.

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
- `tests_v2/` browser suites predate the agent-browser submodule bump on
  some machines; the CI headless job runs the orchestration subset
  (`test_orchestration_v3.py`). Known-flaky visual test: "D12" VM failure
  (see TODO.md) — quarantined, not silently green.

### Upstream
- This fork carries the full contribution stack (see MY-CONTRIBUTIONS.md).
  Upstream (tcsenpai/cmux 0.1.0) moves slowly; PRs are offered but nothing
  here blocks on acceptance.
