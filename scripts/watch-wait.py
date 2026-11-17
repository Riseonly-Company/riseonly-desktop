"""Blocks until scripts/watch-macos.sh has built the change saved before this ran.

  python3 scripts/watch-wait.py [--since EPOCH] [--timeout SECONDS] [--profile watch]

Exit 0: green, the windows run the change (their names, pids and drive dirs are printed).
Exit 1: red or a window crashed at start; the errors are printed.
Exit 3: timeout. Exit 4: no watcher is running.
When the watcher saw no change after --since, the last build is reported as it is.
"""
import argparse
import json
import os
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ERROR_LINES = 150


def status_path(profile):
    target = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    return target / ("debug" if profile == "dev" else profile) / "watch-status.json"


def read_status(path):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return None


def alive(pid):
    try:
        os.kill(int(pid), 0)
        return True
    except (OSError, TypeError, ValueError):
        return False


def report(status, note=""):
    state = status.get("state")
    seconds = max(0.0, float(status.get("finished") or 0) - float(status.get("started") or 0))
    head = f"build {status.get('seq')} {state.upper()} in {seconds:.0f}s{note}"
    if state == "green":
        print(head)
        for window in status.get("windows") or []:
            drive = f" drive {window['drive_dir']}" if window.get("drive_dir") else ""
            print(f"  window {window['name']} pid {window.get('pid')}{drive} log {window.get('log')}")
        return 0
    print(head)
    try:
        lines = Path(status.get("errors", "")).read_text(errors="replace").splitlines()
    except OSError:
        lines = []
    for line in lines[:ERROR_LINES]:
        print(line)
    if len(lines) > ERROR_LINES:
        print(f"... {len(lines) - ERROR_LINES} more lines in {status.get('errors')}")
    return 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--since", type=float, default=None, help="epoch seconds of the change (default now)")
    parser.add_argument("--timeout", type=float, default=900.0)
    parser.add_argument("--profile", default="watch")
    args = parser.parse_args()
    since = args.since if args.since is not None else time.time()
    path = status_path(args.profile)
    deadline = time.time() + args.timeout
    while True:
        status = read_status(path)
        if status is None or status.get("state") == "stopped" or not alive(status.get("watcher_pid")):
            print(f"no watcher is running ({path}); start scripts/watch-macos.sh")
            return 4
        state = status.get("state")
        started = float(status.get("started") or 0)
        finished = float(status.get("finished") or 0)
        quiet = float(status.get("quiet") or 2)
        last_change = float(status.get("last_change") or 0)
        if state in ("green", "red", "crashed") and started >= since:
            return report(status)
        idle_since = max(finished, since)
        if state != "building" and last_change < since and time.time() - idle_since > quiet + 2:
            return report(status, note=" (no change seen after --since, last build)")
        if time.time() > deadline:
            print(f"timeout after {args.timeout:.0f}s, watcher state {state}")
            return 3
        time.sleep(0.3)


if __name__ == "__main__":
    sys.exit(main())
