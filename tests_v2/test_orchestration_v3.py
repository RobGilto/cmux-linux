#!/usr/bin/env python3
"""Roadmap Phase 3 orchestration API coverage: wait-for rendezvous, surface.top,
workspace groups, and background-workspace I/O (the realize-sweep P0 fix)."""

import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from cmux import cmux, cmuxError


def _must(cond: bool, msg: str) -> None:
    if not cond:
        raise cmuxError(msg)


def test_wait_for(c: cmux) -> None:
    # Latch: signal-before-wait releases immediately, once.
    c._call("rendezvous.signal", {"name": "t3-latch"})
    res = c._call("rendezvous.wait", {"name": "t3-latch", "timeout_ms": 2000})
    _must(res.get("released") == "t3-latch", f"latch not consumed: {res}")

    # Consumed latch → timeout error.
    try:
        c._call("rendezvous.wait", {"name": "t3-latch", "timeout_ms": 300})
        raise cmuxError("second wait should have timed out")
    except cmuxError as exc:
        _must("timeout" in str(exc).lower(), f"expected timeout, got {exc}")

    # Blocked wait released by a signal from a second connection.
    result = {}

    def waiter() -> None:
        with cmux() as c2:
            result["res"] = c2._call(
                "rendezvous.wait", {"name": "t3-live", "timeout_ms": 10000},
                timeout_s=15,
            )

    t = threading.Thread(target=waiter)
    t.start()
    time.sleep(0.4)
    c._call("rendezvous.signal", {"name": "t3-live"})
    t.join(timeout=5)
    _must(result.get("res", {}).get("released") == "t3-live", f"waiter not released: {result}")
    print("PASS: wait-for latch/timeout/release")


def test_top(c: cmux) -> None:
    rows = (c._call("surface.top", {}) or {}).get("top", [])
    _must(len(rows) >= 1, "surface.top returned no rows")
    with_pid = [r for r in rows if r.get("pid")]
    _must(len(with_pid) >= 1, f"no surface matched a process: {rows}")
    for r in with_pid:
        _must("cpu_secs" in r and "rss_bytes" in r, f"row missing stats: {r}")
    print(f"PASS: surface.top ({len(with_pid)}/{len(rows)} rows matched to processes)")


def test_groups(c: cmux) -> None:
    ws = c._call("workspace.list", {})["workspaces"][0]["id"]
    c._call("workspace.set_group", {"workspace": ws, "group": "t3-group"})
    groups = (c._call("workspace_group.list", {}) or {}).get("groups", [])
    names = [g["name"] for g in groups]
    _must("t3-group" in names, f"group not listed: {names}")
    listed = c._call("workspace.list", {})["workspaces"][0].get("group")
    _must(listed == "t3-group", f"workspace.list group field wrong: {listed}")
    # Clearing removes the group when it was the only member.
    c._call("workspace.set_group", {"workspace": ws, "group": ""})
    groups = (c._call("workspace_group.list", {}) or {}).get("groups", [])
    _must(
        all(g["name"] != "t3-group" for g in groups),
        f"cleared group still listed: {groups}",
    )
    print("PASS: workspace groups set/list/clear")


def test_background_io(c: cmux) -> None:
    # Create a workspace (steals focus), then switch away and drive it from
    # the background — send, read, and title must all work unfocused.
    created = c._call("workspace.create", {"name": "t3-bg"})
    bg_ws = created.get("uuid") or created.get("id")
    time.sleep(1.5)
    surfaces = c._call("surface.list", {})["surfaces"]
    bg_surf = next(s["uuid"] for s in surfaces if s["workspace_uuid"] == bg_ws)

    other = next(
        w["id"] for w in c._call("workspace.list", {})["workspaces"] if w["id"] != bg_ws
    )
    c._call("workspace.select", {"id": other})
    time.sleep(0.5)

    c._call("surface.send_text", {"id": bg_surf, "text": "echo T3-BG-$((2+2))"})
    c._call("surface.send_key", {"id": bg_surf, "key": "enter"})
    time.sleep(1.5)
    text = c._call("surface.read_text", {"id": bg_surf, "scrollback": True}).get("text", "")
    _must("T3-BG-4" in text, f"background surface not drivable: {text[-200:]!r}")

    c._call("workspace.close", {"id": bg_ws})
    print("PASS: background workspace send/read")


def main() -> int:
    with cmux() as c:
        test_wait_for(c)
        test_top(c)
        test_groups(c)
        test_background_io(c)
    print("PASS: all orchestration v3 tests")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
