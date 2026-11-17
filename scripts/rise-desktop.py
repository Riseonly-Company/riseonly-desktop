#!/usr/bin/env python3
"""rise desktop: the desktop app on this Mac, the way `rise dev` runs the backend.

  rise desktop dev                  watch loop: every saved change rebuilds and restarts the window
  rise desktop dev --linux          ... and checks the Linux branches after each green build
  rise desktop dev --headless --drive A
                                    no visible window, one hidden window driven by files
  rise desktop wait                 block until the change saved before it is built (exit 0 green, 1 red)
  rise desktop status               state of the running loop: last build, windows, errors
  rise desktop logs [WINDOW] [-f]   log of the visible window (main) or of a --drive window
  rise desktop stop                 stop the running loop and its windows
  rise desktop check [--all]        one compiler check of macOS (and --linux, --windows), no window
  rise desktop test [filter]        unit tests in the app crates that hold the named module or file
  rise desktop build [--open]       a fast .app for trying by hand (profile fast, production backend)
  rise desktop linux start|app|snap|stop|install
                                    a Linux desktop in the OrbStack machine for trying by hand

WORKFLOW FOR SCRIPTS AND AUTOMATION

  1. Start one loop for the session:   rise desktop dev --headless --drive NAME   (in the background)
  2. Edit the code, then:              rise desktop wait
     exit 0: the window runs the change; exit 1: the errors are printed, fix and wait again.
  3. Drive the hidden window through its drive dir (printed by wait and status).
  4. Read its log when it misbehaves:  rise desktop logs NAME --tail 200
  5. Do not start a second loop on the same tree; `rise desktop status` tells whether one runs.
"""

import argparse
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(os.path.realpath(__file__)).parent.parent
SCRIPTS = ROOT / "scripts"
LOCK = "/tmp/riseonly-desktop-build.lock"


def target_dir():
    return Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")


def state_dir(profile="watch"):
    return target_dir() / ("debug" if profile == "dev" else profile)


def read_status(profile="watch"):
    try:
        return json.loads((state_dir(profile) / "watch-status.json").read_text())
    except (OSError, ValueError):
        return None


def alive(pid):
    try:
        os.kill(int(pid), 0)
        return True
    except (OSError, TypeError, ValueError):
        return False


def running_loop(profile="watch"):
    status = read_status(profile)
    if status and status.get("state") != "stopped" and alive(status.get("watcher_pid")):
        return status
    return None


def selected_oses(arguments):
    oses = [name for name in ("macos", "linux", "windows") if getattr(arguments, name)]
    if getattr(arguments, "all", False):
        oses = ["macos", "linux", "windows"]
    return oses or ["macos"]


def command_dev(arguments):
    if platform.system() != "Darwin":
        print("rise desktop dev runs the watch loop on macOS; on this host use cargo make dev", file=sys.stderr)
        return 2
    if running_loop(arguments.profile):
        print("a loop already runs on this tree (rise desktop status); stop it first: rise desktop stop", file=sys.stderr)
        return 1
    oses = selected_oses(arguments)
    argv = ["bash", str(SCRIPTS / "watch-macos.sh"), "--profile", arguments.profile, "--env", arguments.env]
    if "macos" not in oses:
        argv.append("--check")
    if "linux" in oses:
        argv.append("--linux")
    if "windows" in oses:
        argv.append("--windows")
    for name in arguments.drive:
        argv += ["--drive", name]
    for flag in ("headless", "check", "warnings", "adhoc", "mute", "design", "no_run"):
        if getattr(arguments, flag):
            argv.append("--" + flag.replace("_", "-"))
    os.chdir(ROOT)
    os.execvp(argv[0], argv)


def command_wait(arguments):
    argv = [sys.executable, str(SCRIPTS / "watch-wait.py"), "--timeout", str(arguments.timeout),
            "--profile", arguments.profile]
    if arguments.since is not None:
        argv += ["--since", str(arguments.since)]
    os.execvp(argv[0], argv)


def describe_window(window):
    pid = window.get("pid")
    state = "running" if pid and alive(pid) else "down"
    drive = f"  drive {window['drive_dir']}" if window.get("drive_dir") else ""
    return f"  {window['name']:<10} {state:<8} pid {pid or '-'}{drive}\n             log {window.get('log')}"


def command_status(arguments):
    status = read_status(arguments.profile)
    loop = running_loop(arguments.profile)
    if arguments.output == "json":
        print(json.dumps({"running": loop is not None, "status": status}, indent=2))
        return 0 if loop else 3
    if not status:
        print("no loop has run on this tree yet (rise desktop dev)")
        return 3
    started = float(status.get("started") or 0)
    finished = float(status.get("finished") or 0)
    state = status.get("state")
    print(f"loop      {'running, pid ' + str(status.get('watcher_pid')) if loop else 'not running'}")
    print(f"build     {status.get('seq')} {state}" + (
        f" in {finished - started:.0f}s, {time.time() - finished:.0f}s ago" if state != "building" and finished
        else f" for {time.time() - started:.0f}s" if state == "building" else ""))
    print(f"profile   {status.get('profile')}, backend {status.get('env')}")
    print(f"bundle    {status.get('bundle') or '-'}")
    print(f"errors    {status.get('errors')}")
    windows = status.get("windows") or []
    print("windows" + ("" if windows else "   none"))
    for window in windows:
        print(describe_window(window))
    return 0 if loop else 3


def log_path(name, profile):
    status = read_status(profile) or {}
    for window in status.get("windows") or []:
        if window.get("name") == name:
            return Path(window["log"])
    if name == "main":
        return state_dir(profile) / "watch-app.log"
    root = Path(os.environ.get("WATCH_DRIVE_ROOT") or target_dir() / "watch-drive")
    return root / f"{name}.log"


def command_logs(arguments):
    path = log_path(arguments.window, arguments.profile)
    if not path.exists():
        print(f"no log at {path}", file=sys.stderr)
        return 4
    argv = ["tail", "-n", str(arguments.tail)] + (["-F"] if arguments.follow else []) + [str(path)]
    os.execvp(argv[0], argv)


def command_stop(arguments):
    loop = running_loop(arguments.profile)
    if not loop:
        print("no loop is running")
        return 0
    pid = int(loop["watcher_pid"])
    os.kill(pid, 15)
    deadline = time.time() + 15
    while alive(pid) and time.time() < deadline:
        time.sleep(0.2)
    if alive(pid):
        print(f"the loop (pid {pid}) did not stop in 15 s", file=sys.stderr)
        return 1
    print(f"stopped the loop (pid {pid}) and its windows")
    return 0


def run_check(label, argv, env):
    diag = [sys.executable, str(SCRIPTS / "cargo-diagnostics.py"), f"--label={label}"]
    start = time.time()
    cargo = subprocess.Popen(argv, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    shown = subprocess.Popen(diag, stdin=cargo.stdout)
    cargo.stdout.close()
    _, err = cargo.communicate()
    shown.wait()
    seconds = time.time() - start
    if cargo.returncode in (2, 3) and label != "macOS":
        tail = err.decode(errors="replace").strip().splitlines()[-1:] or [""]
        print(f"{label}: check did not start: {tail[0]}")
        return label, None, seconds
    return label, cargo.returncode == 0, seconds


def command_check(arguments):
    oses = selected_oses(arguments)
    env = dict(os.environ, RISE_ENV="dev", MLN_PRECOMPILE=os.environ.get("MLN_PRECOMPILE", "1"))
    env.setdefault("CARGO_TARGET_DIR", str(target_dir()))
    jobs = []
    if "macos" in oses:
        jobs.append(("macOS", ["cargo", "check", "--profile", "watch", "-p", "riseonly", "--features", "dev-drive",
                               "--message-format=json-diagnostic-rendered-ansi"], env))
    if "linux" in oses:
        jobs.append(("Linux", ["bash", str(SCRIPTS / "linux-check.sh"), str(ROOT), "check"],
                     dict(env, CARGO_TARGET_DIR=str(target_dir() / "watch-linux"))))
    if "windows" in oses:
        jobs.append(("Windows", ["bash", str(SCRIPTS / "windows-check.sh"), str(ROOT)],
                     dict(env, CARGO_TARGET_DIR=str(target_dir() / "watch-win"))))
    from concurrent.futures import ThreadPoolExecutor
    with ThreadPoolExecutor(len(jobs)) as pool:
        results = list(pool.map(lambda job: run_check(*job), jobs))
    code = 0
    for label, green, seconds in results:
        verdict = "did not start" if green is None else "green" if green else "RED"
        print(f"{label:<8} {verdict} in {seconds:.0f}s")
        if green is not True:
            code = 1
    return code


def command_build(arguments):
    if platform.system() != "Darwin":
        print("rise desktop build makes the macOS .app; on this host use cargo make dev", file=sys.stderr)
        return 2
    env = dict(os.environ, RISE_ENV=arguments.env, MLN_PRECOMPILE=os.environ.get("MLN_PRECOMPILE", "1"))
    env.setdefault("CARGO_TARGET_DIR", str(target_dir()))
    script = (
        'set -e\n'
        'if [ -z "${RISE_SIGN_IDENTITY:-}" ]; then\n'
        '  RISE_SIGN_IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | grep -v CSSMERR'
        ' | sed -n \'s/^ *[0-9]*) \\([0-9A-F]\\{40\\}\\) "Apple Development.*$/\\1/p\' | head -n 1)"\n'
        '  export RISE_SIGN_IDENTITY="${RISE_SIGN_IDENTITY:--}"\n'
        'fi\n'
        f'cargo build --profile {arguments.profile} -p riseonly\n'
        f'bash scripts/bundle-macos.sh {arguments.profile} | tail -n 1\n'
    )
    argv = ["bash", "-c", script]
    if Path("/usr/bin/lockf").exists():
        argv = ["/usr/bin/lockf", "-k", LOCK] + argv
    print(f"building profile {arguments.profile} for {arguments.env} (a cold build takes 15-25 min)...")
    start = time.time()
    result = subprocess.run(argv, cwd=ROOT, env=env, capture_output=True, text=True)
    sys.stdout.write("\n".join(line for line in result.stdout.splitlines() if line.startswith("bundled:")) + "\n")
    if result.returncode != 0:
        sys.stdout.write("\n".join(result.stderr.splitlines()[-40:]) + "\n")
        print(f"RED after {time.time() - start:.0f}s")
        return 1
    print(f"green in {time.time() - start:.0f}s")
    if arguments.open:
        bundle = next((line.split("bundled: ", 1)[1].split("  (")[0] for line in result.stdout.splitlines()
                       if line.startswith("bundled: ")), None)
        if bundle:
            subprocess.run(["open", "-n", bundle])
    return 0


def command_test(arguments):
    found = subprocess.run([sys.executable, "-I", str(SCRIPTS / "gen-mod-tree.py"), "--where", arguments.filter or "::"],
                           cwd=ROOT, capture_output=True, text=True)
    crates = [line.split("\t")[0] for line in found.stdout.splitlines() if line.strip()]
    if not crates:
        print(found.stderr.strip() or "no crate found", file=sys.stderr)
        return 2
    env = dict(os.environ, RISE_ENV="dev", MLN_PRECOMPILE=os.environ.get("MLN_PRECOMPILE", "1"))
    env.setdefault("CARGO_TARGET_DIR", str(target_dir()))
    argv = ["cargo", "test", "--profile", "watch", "--features", "dev-drive", "--lib"]
    for crate in crates:
        argv += ["-p", crate]
    if arguments.filter:
        argv.append(arguments.filter)
    argv += ["--"] + arguments.rest if arguments.rest else []
    if Path("/usr/bin/lockf").exists():
        argv = ["/usr/bin/lockf", "-k", LOCK] + argv
    print(f"testing {', '.join(crates)}" + (f" for {arguments.filter}" if arguments.filter else ""))
    return subprocess.run(argv, cwd=ROOT, env=env).returncode


def command_linux(arguments):
    argv = ["bash", str(SCRIPTS / "linux-desktop.sh"), arguments.action] + arguments.rest
    os.execvp(argv[0], argv)


def add_oses(parser, all_flag=False):
    parser.add_argument("--macos", action="store_true", help="macOS (the default when no OS is named)")
    parser.add_argument("--linux", action="store_true", help="Linux branches (compiler check in the OrbStack machine)")
    parser.add_argument("--windows", action="store_true", help="Windows branches (compiler check through cargo-xwin)")
    if all_flag:
        parser.add_argument("--all", action="store_true", help="all three")


def build_parser():
    parser = argparse.ArgumentParser(prog="rise desktop", description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", metavar="COMMAND")

    dev = sub.add_parser("dev", help="watch loop: rebuild and restart on every change",
                         description="Rebuilds on every saved change (app, crates, assets, gpui, riseonly-tools) "
                                     "and restarts the windows; a red build keeps the last green windows. "
                                     "Linux and Windows cannot run on this Mac: with --linux or --windows their "
                                     "branches are compiler-checked after each green build.")
    add_oses(dev)
    dev.add_argument("--env", default="dev", choices=("dev", "staging", "production"),
                     help="backend compiled in (default dev: the local stand 127.0.0.1:8080)")
    dev.add_argument("--prod", dest="env", action="store_const", const="production", help="same as --env production")
    dev.add_argument("--drive", action="append", default=[], metavar="NAME", help="add a hidden driven window")
    dev.add_argument("--headless", action="store_true", help="no visible window")
    dev.add_argument("--check", action="store_true", help="compiler check only, no build and no window")
    dev.add_argument("--profile", default="watch", help="cargo profile (default watch)")
    dev.add_argument("--warnings", action="store_true", help="print warnings, not only their count")
    dev.add_argument("--adhoc", action="store_true", help="ad-hoc signature instead of Apple Development")
    dev.add_argument("--mute", action="store_true", help="no sound in the visible window")
    dev.add_argument("--design", action="store_true", help="open the storybook")
    dev.add_argument("--no-run", action="store_true", help="build and bundle, never start a window")
    dev.set_defaults(handler=command_dev)

    wait = sub.add_parser("wait", help="block until the saved change is built",
                          description="Exit 0 green (the windows run the change), 1 red or crashed at start "
                                      "(errors printed), 3 timeout, 4 no loop running.")
    wait.add_argument("--since", type=float, default=None, help="epoch seconds of the change (default now)")
    wait.add_argument("--timeout", type=float, default=900.0)
    wait.add_argument("--profile", default="watch")
    wait.set_defaults(handler=command_wait)

    status = sub.add_parser("status", help="state of the running loop")
    status.add_argument("-o", "--output", choices=("text", "json"), default="text")
    status.add_argument("--profile", default="watch")
    status.set_defaults(handler=command_status)

    logs = sub.add_parser("logs", help="log of a window of the loop")
    logs.add_argument("window", nargs="?", default="main", help="main (the visible window) or a --drive name")
    logs.add_argument("--tail", type=int, default=100)
    logs.add_argument("-f", "--follow", action="store_true")
    logs.add_argument("--profile", default="watch")
    logs.set_defaults(handler=command_logs)

    stop = sub.add_parser("stop", help="stop the running loop and its windows")
    stop.add_argument("--profile", default="watch")
    stop.set_defaults(handler=command_stop)

    check = sub.add_parser("check", help="one compiler check per OS, no window",
                           description="macOS shares the cache of the loop; Linux runs in the OrbStack machine, "
                                       "Windows through cargo-xwin, each in its own target dir, all in parallel.")
    add_oses(check, all_flag=True)
    check.set_defaults(handler=command_check)

    build = sub.add_parser("build", help="a fast .app for trying by hand",
                           description="Builds and bundles the app under the shared build lock and prints the "
                                       ".app path; signed with Apple Development when there is one.")
    build.add_argument("--env", default="production", choices=("dev", "staging", "production"))
    build.add_argument("--profile", default="fast", choices=("fast", "release", "devopt", "watch"))
    build.add_argument("--open", action="store_true", help="open the .app when it is built")
    build.set_defaults(handler=command_build)

    test = sub.add_parser("test", help="unit tests of the app crates that hold the named module or file",
                          description="The app tree is compiled by several crates (scripts/crate-views.json), so "
                                      "cargo test -p riseonly runs only the top one. This finds the crates whose "
                                      "files match the filter (a module path or a file name; none: every crate) "
                                      "and runs cargo test there with the filter.")
    test.add_argument("filter", nargs="?", default="", help="module path, file name or test name")
    test.add_argument("rest", nargs=argparse.REMAINDER, help="arguments for the test binary after --")
    test.set_defaults(handler=command_test)

    linux = sub.add_parser("linux", help="a Linux desktop in the OrbStack machine (scripts/linux-desktop.sh)")
    linux.add_argument("action", choices=("install", "start", "app", "snap", "stop"))
    linux.add_argument("rest", nargs=argparse.REMAINDER)
    linux.set_defaults(handler=command_linux)
    return parser


def main(argv=None):
    parser = build_parser()
    arguments = parser.parse_args(argv)
    if arguments.command is None:
        parser.print_help()
        return 0
    try:
        return int(arguments.handler(arguments) or 0)
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
