#!/usr/bin/env python3
import hashlib
import json
import os
import re
import subprocess
import sys
import urllib.request

PLATFORMS = ("linux", "windows", "macos")
OS_LABEL = {"linux": "linux", "windows": "win", "macos": "macos"}
ARCH_LABELS = {
    "linux": (("linux-x64", "x86_64"), ("linux-arm64", "aarch64")),
    "windows": (("win-x64", "x86_64"), ("win-arm64", "aarch64")),
    "macos": (("macos-silicon", "aarch64"), ("macos-intel", "x86_64")),
}
DEFAULT_ARCHS = {"linux": ["x86_64"], "windows": ["x86_64"], "macos": ["aarch64", "x86_64"]}
MACOS_LOCAL_LABEL = "macos-local"
MACOS_LOCAL_RUNNER = ["self-hosted", "macOS", "ARM64", "riseonly-desktop-macos"]
LABELS_OF = {p: {OS_LABEL[p]} | {label for label, _ in ARCH_LABELS[p]} for p in PLATFORMS}
LABELS_OF["macos"].add(MACOS_LOCAL_LABEL)
ENV_LABELS = (("env-dev", "dev"), ("env-staging", "staging"), ("env-prod", "production"))
SLUGS = {"production": "Riseonly", "staging": "Riseonly-Staging", "dev": "Riseonly-Dev"}
RELEASE_PROFILE_LABEL = "release-profile"
REBUILD_LABELS = (
    {"build", "tests", RELEASE_PROFILE_LABEL}
    | {label for _, label_set in LABELS_OF.items() for label in label_set}
    | {label for label, _ in ENV_LABELS}
)
FLOW = {"staging": "development", "main": "staging"}
NOT_COMPILED = (".github/workflows/", "docs/", "scripts/ci/", "scripts/bundle-")
COMPILE_SCRIPTS = ("scripts/ci/build-", "scripts/ci/in-toolchain.sh", "scripts/ci/toolchain/")
PAYLOAD_ASSET = re.compile(r"([a-z]+-[a-z0-9_]+)\.tar\.gz")


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


def archs(platform, labels):
    every = [arch for _, arch in ARCH_LABELS[platform]]
    if OS_LABEL[platform] in labels:
        return every
    picked = [arch for label, arch in ARCH_LABELS[platform] if label in labels]
    return picked or DEFAULT_ARCHS[platform]


def environment_of(kind, base, labels):
    if kind == "build":
        picked = [env for label, env in ENV_LABELS if label in labels]
        if len(picked) > 1:
            sys.exit(f"pick one environment label, not {', '.join(label for label, _ in ENV_LABELS if label in labels)}")
        if picked:
            return picked[0]
    return "production" if base == "main" else "staging"


def github(path):
    request = urllib.request.Request(
        f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/{path}",
        headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}", "Accept": "application/vnd.github+json"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def source_key(entries):
    lines = sorted(
        f"{sha} {path}"
        for path, sha in entries
        if path.startswith(COMPILE_SCRIPTS) or not path.startswith(NOT_COMPILED)
    )
    return hashlib.sha256("\n".join(lines).encode()).hexdigest()[:12]


def local_entries(commit):
    listing = subprocess.run(["git", "ls-tree", "-r", "-z", commit], check=True, capture_output=True).stdout
    for record in listing.split(b"\0"):
        if record:
            meta, _, path = record.partition(b"\t")
            yield path.decode(), meta.split()[2].decode()


def stored_by_hand(key, fetch):
    tag = re.compile(rf"payload-{key}-b(\d+)")
    builds = {}
    for release in fetch("releases?per_page=100"):
        found = tag.fullmatch(release.get("tag_name") or "")
        if not found or release.get("draft"):
            continue
        assets = {}
        for asset in release.get("assets", []):
            name = PAYLOAD_ASSET.fullmatch(asset["name"])
            if name and asset.get("state") == "uploaded":
                assets[name.group(1)] = asset["name"]
        if assets:
            builds[int(found.group(1))] = (release["tag_name"], assets)
    if not builds:
        return None, "", {}
    build = max(builds)
    return str(build), builds[build][0], builds[build][1]


def compiled_before(key, fetch):
    name = re.compile(rf"payload-{key}-b(\d+)-([a-z]+-[a-z0-9_]+)")
    builds = {}
    for page in range(1, 11):
        batch = fetch(f"actions/artifacts?per_page=100&page={page}")["artifacts"]
        for artifact in batch:
            found = name.fullmatch(artifact["name"])
            if found and not artifact["expired"]:
                build, target = int(found.group(1)), found.group(2)
                builds.setdefault(build, {}).setdefault(target, artifact["workflow_run"]["id"])
        if len(batch) < 100:
            break
    if not builds:
        return None, {}
    build = max(builds)
    return str(build), builds[build]


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
    if mode == "key":
        print(source_key(local_entries(sys.argv[2] if len(sys.argv) > 2 else "HEAD")))
        return
    if mode not in ("build", "release"):
        sys.exit("usage: plan.py build|release|key [commit]")
    root = os.environ.get("PLAN_ROOT", ".")
    event = json.load(open(os.environ["GITHUB_EVENT_PATH"], encoding="utf-8"))
    pr = event["pull_request"]
    kind, reason, labels = decide(mode, event)
    head_repo = (pr["head"].get("repo") or {}).get("full_name")
    local = MACOS_LOCAL_LABEL in labels and head_repo == pr["base"]["repo"]["full_name"]
    out = {
        "run": "false",
        "reason": reason,
        "linux_archs": json.dumps(archs("linux", labels)),
        "windows_archs": json.dumps(archs("windows", labels)),
        "macos_archs": json.dumps(archs("macos", labels)),
        "macos_runner": json.dumps(MACOS_LOCAL_RUNNER) if local else os.environ.get("MACOS_RUNNER") or '"macos-15"',
        "macos_local": str(local).lower(),
    }
    if kind:
        env = ci_env(root)
        picked = [p for p in PLATFORMS if LABELS_OF[p] & labels] or list(PLATFORMS)
        environment = environment_of(kind, pr["base"]["ref"], labels)
        version = workspace_version(root)
        build_number = os.environ.get("GITHUB_RUN_NUMBER", "1")
        sha = pr.get("merge_commit_sha") if kind == "release" else pr["head"]["sha"]
        tree, payloads, payload_tag, payload_assets = "", {}, "", {}
        if kind == "release" and os.environ.get("GH_TOKEN"):
            root = github(f"git/commits/{sha}")["tree"]["sha"]
            listing = github(f"git/trees/{root}?recursive=1")
            if listing.get("truncated"):
                tree = root[:12]
            else:
                tree = source_key((e["path"], e["sha"]) for e in listing["tree"] if e["type"] != "tree")
            inherited, payload_tag, payload_assets = stored_by_hand(tree, github)
            build, runs = compiled_before(tree, github)
            if inherited is None:
                inherited, payloads = build, runs
            elif build == inherited:
                payloads = {target: run for target, run in runs.items() if target not in payload_assets}
            build_number = inherited or build_number
        out.update({
            "run": "true",
            "platforms": json.dumps(picked),
            "linux": str("linux" in picked).lower(),
            "windows": str("windows" in picked).lower(),
            "macos": str("macos" in picked).lower(),
            "tests": str(kind == "build" and "tests" in labels).lower(),
            "environment": environment,
            "profile": "release" if kind == "release" or RELEASE_PROFILE_LABEL in labels else "fast",
            "version": version,
            "build_number": build_number,
            "slug": SLUGS[environment],
            "tag": f"v{version}" if environment == "production" else f"staging-v{version}-{build_number}",
            "sha": sha,
            "tree": tree,
            "payloads": json.dumps(payloads),
            "payload_tag": payload_tag,
            "payload_assets": json.dumps(payload_assets),
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
