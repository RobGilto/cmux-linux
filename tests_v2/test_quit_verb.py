#!/usr/bin/env python3
"""`cmux quit` coverage: graceful socket path, SIGTERM fallback, and the
not-running no-op. Relaunches the app at the end so later tooling finds it.

NOTE: this test stops and restarts the app — run it LAST (CI does).
"""

import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

CLI = os.environ.get("CMUXTERM_CLI") or shutil.which("cmux") or "cmux"


def run(args, **kw):
    return subprocess.run([CLI, *args], capture_output=True, text=True, timeout=40, **kw)


def app_pids():
    out = subprocess.run(["pgrep", "-u", str(os.getuid()), "-x", "cmux-app"],
                         capture_output=True, text=True)
    return [int(p) for p in out.stdout.split()]


def must(cond, msg):
    if not cond:
        raise AssertionError(msg)


def main() -> int:
    # Precondition: app up (launch is idempotent).
    r = run(["launch", "--wait-secs", "25"])
    must(r.returncode == 0, f"launch failed: {r.stderr}")

    # 1. Graceful path: socket answers → quit exits 0, process gone.
    r = run(["quit"])
    must(r.returncode == 0, f"graceful quit failed: {r.stdout}{r.stderr}")
    must("quitting gracefully" in r.stdout, f"missing stage line: {r.stdout!r}")
    must("stopped" in r.stdout or "force-killed" in r.stdout, f"no terminal stage: {r.stdout!r}")
    time.sleep(0.5)
    must(not app_pids(), "cmux-app still alive after graceful quit")
    must(run(["ping"]).returncode != 0, "socket still answering after quit")
    print("PASS: graceful quit via socket")

    # 2. No-op path: nothing running → exit 0, says so.
    r = run(["quit"])
    must(r.returncode == 0, f"no-op quit should exit 0: {r.stderr}")
    must("not running" in r.stdout, f"expected 'not running': {r.stdout!r}")
    print("PASS: quit is a no-op when nothing runs")

    # 3. SIGTERM fallback: process alive but socket dead. Simulate by
    #    launching, then SIGSTOPping the app so the socket stops answering
    #    while the process persists... SIGSTOP'd processes don't answer but
    #    also can't handle SIGTERM until continued — instead simulate the
    #    dead-socket case by removing the socket file after launch.
    r = run(["launch", "--wait-secs", "25"])
    must(r.returncode == 0, f"relaunch failed: {r.stderr}")
    sock = os.environ.get("CMUX_SOCKET") or (
        f"{os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')}/cmux/cmux.sock"
    )
    os.rename(sock, sock + ".hidden")  # socket unreachable, process alive
    try:
        r = run(["quit"])
        must(r.returncode == 0, f"fallback quit failed: {r.stdout}{r.stderr}")
        must("SIGTERM" in r.stdout, f"expected SIGTERM stage: {r.stdout!r}")
        time.sleep(0.5)
        must(not app_pids(), "cmux-app survived SIGTERM fallback")
        print("PASS: SIGTERM fallback when socket is dead")
    finally:
        for leftover in (sock + ".hidden",):
            try:
                os.remove(leftover)
            except FileNotFoundError:
                pass

    # Leave the app running for whatever comes next.
    r = run(["launch", "--wait-secs", "25"])
    must(r.returncode == 0, f"final relaunch failed: {r.stderr}")
    print("PASS: all quit paths")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
