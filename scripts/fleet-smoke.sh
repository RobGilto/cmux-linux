#!/usr/bin/env bash
# fleet-smoke.sh — end-to-end fleet orchestration smoke test (roadmap 3.6).
#
# Creates a 2x2 grid in a workspace, switches AWAY from it, then drives all
# four panes from the background: send a distinct sentinel to each, read all
# four back, run a wait-for rendezvous roundtrip, check `top` sees the shells,
# then tear everything down. Exits 0 only if every step passes.
#
# Optional: --screenshot <dir> captures the fleet workspace before teardown
# (grim + hyprctl, Hyprland only) for visual inspection.

set -euo pipefail

SCREENSHOT_DIR=""
if [[ "${1:-}" == "--screenshot" ]]; then
    SCREENSHOT_DIR="${2:?--screenshot needs a directory}"
    mkdir -p "$SCREENSHOT_DIR"
fi

fail() { echo "FAIL: $*" >&2; exit 1; }
step() { echo "── $*"; }

command -v cmux >/dev/null || fail "cmux CLI not on PATH"
cmux ping >/dev/null || fail "cmux-app not running (try: cmux launch)"

step "create fleet workspace + 2x2 grid"
WS=$(cmux new-workspace --name fleet-smoke --json | jq -r '.uuid // .id')
[[ -n "$WS" && "$WS" != "null" ]] || fail "workspace create returned no id"
sleep 1.5

TL=$(cmux list-surfaces --json | jq -r ".surfaces[] | select(.workspace_uuid==\"$WS\").uuid" | head -1)
cmux split --direction horizontal --id "$TL" >/dev/null; sleep 0.8
TR=$(cmux list-surfaces --json | jq -r ".surfaces[] | select(.workspace_uuid==\"$WS\" and .uuid!=\"$TL\").uuid" | head -1)
cmux split --direction vertical --id "$TL" >/dev/null; sleep 0.8
cmux split --direction vertical --id "$TR" >/dev/null; sleep 0.8

mapfile -t PANES < <(cmux list-surfaces --json | jq -r ".surfaces[] | select(.workspace_uuid==\"$WS\").uuid")
[[ ${#PANES[@]} -eq 4 ]] || fail "expected 4 panes, got ${#PANES[@]}"

step "switch away — all driving happens against a BACKGROUND workspace"
OTHER=$(cmux list-workspaces --json | jq -r ".workspaces[] | select(.id!=\"$WS\").id" | head -1)
[[ -n "$OTHER" ]] || fail "need a second workspace to switch to"
cmux select-workspace "$OTHER" >/dev/null; sleep 0.5

step "send a distinct sentinel to each pane"
i=0
for P in "${PANES[@]}"; do
    cmux send-text --id "$P" "echo FLEET-PANE-$i-\$((10+$i))" >/dev/null
    cmux send-key --id "$P" enter >/dev/null
    i=$((i+1))
done
sleep 2

step "read all four back (workspace still in background)"
i=0
for P in "${PANES[@]}"; do
    OUT=$(cmux read-text --id "$P" --json | jq -r .text)
    grep -q "FLEET-PANE-$i-$((10+i))" <<<"$OUT" || fail "pane $i sentinel missing"
    i=$((i+1))
done
echo "   all 4 sentinels read back ✓"

step "wait-for rendezvous roundtrip (worker signals, orchestrator waits)"
cmux send-text --id "${PANES[0]}" "sleep 1; cmux wait-for -S fleet-smoke-done" >/dev/null
cmux send-key --id "${PANES[0]}" enter >/dev/null
cmux wait-for fleet-smoke-done --timeout 15 >/dev/null || fail "wait-for roundtrip"
echo "   rendezvous released ✓"

step "top sees the fleet's shells"
ROWS=$(cmux top --format json | jq "[.top[] | select(.workspace==\"fleet-smoke\" and .pid != null)] | length")
[[ "$ROWS" -ge 4 ]] || fail "top matched only $ROWS/4 fleet shells"
echo "   top matched $ROWS fleet shells ✓"

if [[ -n "$SCREENSHOT_DIR" ]] && command -v grim >/dev/null && command -v hyprctl >/dev/null; then
    step "visual capture"
    cmux select-workspace "$WS" >/dev/null; sleep 1
    G=$(hyprctl clients -j | jq -r '.[] | select(.class=="com.cmux_lx.terminal") | "\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"' | head -1)
    if [[ -n "$G" ]]; then
        grim -g "$G" "$SCREENSHOT_DIR/fleet-smoke.png" && echo "   saved $SCREENSHOT_DIR/fleet-smoke.png"
    else
        echo "   cmux window not found for capture (skipped)"
    fi
    cmux select-workspace "$OTHER" >/dev/null
fi

step "teardown"
cmux close-workspace "$WS" >/dev/null
sleep 0.5
cmux list-workspaces --json | jq -e ".workspaces[] | select(.id==\"$WS\")" >/dev/null 2>&1 \
    && fail "fleet workspace still present after close"

echo "PASS: fleet smoke — 2x2 background fleet created, driven, synchronized, observed, torn down"
