#!/usr/bin/env python3
import json
import os
import re
import sys

PLATFORMS = ("linux", "windows", "macos")
LABEL_OF = {"linux": "linux", "windows": "win", "macos": "macos"}
REBUILD_LABELS = {"build", "linux", "win", "macos", "tests"}
FLOW = {"staging": "development", "main": "staging"}


def workspace_version(root):
    text = open(os.path.join(root, "Cargo.toml"), encoding="utf-8").read()
    block = re.search(r"^\[workspace\.package\]\s*\n(.*?)(?=^\[|\Z)", text, re.S | re.M)
    found = block and re.search(r'^version\s*=\s*"([^"]+)"', block.group(1), re.M)
    if not found:
        sys.exit("no [workspace.package] version in Cargo.toml")
    value = found.group(1)
    if not re.fullmatch(r"\d+\.\d+\.\d+", value):
        sys.exit(f"version {value} is not x.y.z, the updater only reads x.y.z")
    return value


def ci_env(root):
    values = {}
    for line in open(os.path.join(root, ".github", "ci.env"), encoding="utf-8"):
        line = line.strip()
        if line and not line.startswith("#"):
            key, _, value = line.partition("=")
            values[key.strip()] = value.strip()
    return values


def decide(mode, event):
    pr = event["pull_request"]
    labels = {label["name"] for label in pr.get("labels", [])}
    base, head = pr["base"]["ref"], pr["head"]["ref"]
    action = event.get("action", "")
    if mode == "build":
        if action in ("opened", "reopened", "ready_for_review") and base in FLOW:
            labels.add("build")
        added = (event.get("label") or {}).get("name")
        if action == "labeled" and added not in REBUILD_LABELS:
            return None, f"label {added} does not change the build", labels
        if "build" not in labels:
            return None, "no build label", labels
        return "build", "", labels
    if not pr.get("merged"):
        return None, "closed without a merge", labels
    if "deploy" not in labels:
        return None, "merged without the deploy label, nothing ships", labels
    if FLOW.get(base) != head:
        return None, f"{head} into {base} is not a release flow (development into staging, staging into main)", labels
    return "release", "", labels


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode not in ("build", "release"):
        sys.exit("usage: plan.py build|release")
    root = os.environ.get("PLAN_ROOT", ".")
    event = json.load(open(os.environ["GITHUB_EVENT_PATH"], encoding="utf-8"))
    pr = event["pull_request"]
    kind, reason, labels = decide(mode, event)
    out = {"run": "false", "reason": reason}
    if kind:
        env = ci_env(root)
        picked = [p for p in PLATFORMS if LABEL_OF[p] in labels] or list(PLATFORMS)
        environment = "production" if pr["base"]["ref"] == "main" else "staging"
        version = workspace_version(root)
        build_number = os.environ.get("GITHUB_RUN_NUMBER", "1")
        out.update({
            "run": "true",
            "platforms": json.dumps(picked),
            "linux": str("linux" in picked).lower(),
            "windows": str("windows" in picked).lower(),
            "macos": str("macos" in picked).lower(),
            "tests": str(kind == "build" and "tests" in labels).lower(),
            "environment": environment,
            "profile": "release" if kind == "release" else "fast",
            "version": version,
            "build_number": build_number,
            "slug": "Riseonly" if environment == "production" else "Riseonly-Staging",
            "tag": f"v{version}" if environment == "production" else f"staging-v{version}-{build_number}",
            "sha": pr.get("merge_commit_sha") if kind == "release" else pr["head"]["sha"],
            "tools_repo": env["TOOLS_REPO"],
            "tools_ref": env["TOOLS_REF"],
            "rust": env["RUST_TOOLCHAIN"],
        })
    lines = [f"{key}={value}" for key, value in out.items()]
    print("\n".join(lines))
    target = os.environ.get("GITHUB_OUTPUT")
    if target:
        with open(target, "a", encoding="utf-8") as sink:
            sink.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
