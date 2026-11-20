#!/usr/bin/env python3
import hashlib
import json
import os
import re
import subprocess
import sys
import urllib.error
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
LOCAL_LABELS = {"linux": "linux-local", "windows": "win-local", "macos": MACOS_LOCAL_LABEL}
LOCAL_DEFAULTS = {"linux": "LINUX_LOCAL", "windows": "WINDOWS_LOCAL", "macos": "MACOS_LOCAL"}
SERVER_RUNNERS = {
    "linux": ["self-hosted", "linux", "x64", "riseonly-desktop-linux"],
    "windows": ["self-hosted", "linux", "x64", "riseonly-desktop-windows"],
}
LABELS_OF = {p: {OS_LABEL[p], LOCAL_LABELS[p]} | {label for label, _ in ARCH_LABELS[p]} for p in PLATFORMS}
ENV_LABELS = (("env-dev", "dev"), ("env-staging", "staging"), ("env-prod", "production"))
SLUGS = {"production": "Riseonly", "staging": "Riseonly-Staging", "dev": "Riseonly-Dev"}
RELEASE_PROFILE_LABEL = "release-profile"
RELEASE_LEVELS = (("release-major", "major"), ("release-minor", "minor"), ("release-patch", "patch"))
REBUILD_LABELS = (
    {"build", "tests", RELEASE_PROFILE_LABEL}
    | {label for _, label_set in LABELS_OF.items() for label in label_set}
    | {label for label, _ in ENV_LABELS}
)
FLOW = {"staging": "development", "main": "staging"}
NOT_COMPILED = (".github/workflows/", "docs/", "scripts/ci/", "scripts/bundle-")
COMPILE_SCRIPTS = ("scripts/ci/build-",)
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
    if platform == "linux" and LOCAL_LABELS["linux"] in labels and "aarch64" not in picked:
        picked.append("aarch64")
    return picked or DEFAULT_ARCHS[platform]


def on_mac(platform, labels, same_repo):
    if not same_repo:
        return False
    return LOCAL_LABELS[platform] in labels or os.environ.get(LOCAL_DEFAULTS[platform], "").strip().lower() == "on"


def runners(labels, same_repo):
    local = {p: on_mac(p, labels, same_repo) for p in PLATFORMS}
    linux_arm = os.environ.get("LINUX_ARM_RUNNER") or '"ubuntu-24.04-arm"'
    # why: x86_64 Linux under Rosetta on the Mac took 54 min against 46 on the server, only arm64 gains there
    return {
        "linux_runners": json.dumps({
            "x86_64": SERVER_RUNNERS["linux"],
            "aarch64": MACOS_LOCAL_RUNNER if local["linux"] else json.loads(linux_arm),
        }),
        "linux_local": json.dumps({"x86_64": "false", "aarch64": str(local["linux"]).lower()}),
        "windows_runner": json.dumps(MACOS_LOCAL_RUNNER if local["windows"] else SERVER_RUNNERS["windows"]),
        "windows_local": str(local["windows"]).lower(),
        "macos_runner": json.dumps(MACOS_LOCAL_RUNNER) if local["macos"] else os.environ.get("MACOS_RUNNER") or '"macos-15"',
        "macos_local": str(local["macos"]).lower(),
    }


def staging_ships():
    return os.environ.get("STAGING_RELEASES", "").strip().lower() == "on"


def environment_of(kind, base, labels):
    if kind == "build":
        picked = [env for label, env in ENV_LABELS if label in labels]
        if len(picked) > 1:
            sys.exit(f"pick one environment label, not {', '.join(label for label, _ in ENV_LABELS if label in labels)}")
        if picked:
            return picked[0]
        if not staging_ships():
            return "production"
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


def parse_version(raw):
    found = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", raw or "")
    return tuple(int(part) for part in found.groups()) if found else None


def dotted(version):
    return ".".join(str(part) for part in version)


def next_version(version, level):
    major, minor, patch = version
    if level == "major" or (minor >= 9 and (level == "minor" or patch >= 9)):
        return (major + 1, 0, 0)
    if level == "minor" or patch >= 9:
        return (major, minor + 1, 0)
    return (major, minor, patch + 1)


def level_of(labels):
    return next((level for label, level in RELEASE_LEVELS if label in labels), "patch")


def history_of(environment):
    base = (os.environ.get("DOWNLOADS_BASE") or "https://cdn.riseonly.net/desktop").rstrip("/")
    request = urllib.request.Request(
        f"{base}/{environment}/releases.json?run={os.environ.get('GITHUB_RUN_ID', '0')}",
        headers={"User-Agent": "riseonly-desktop-ci"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response).get("releases", [])
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return []
        sys.exit(f"reading the {environment} release history failed with http {error.code}")


def published(environment):
    versions = (parse_version(release.get("version")) for release in history_of(environment))
    return [version for version in versions if version]


def already_shipped(version, targets, history=history_of):
    release = next((r for r in history("production") if r.get("version") == version), None)
    return sorted(set(targets) & set((release or {}).get("downloads", {})))


# why: Windows on arm is rare and runs the x64 build, it ships only when labelled
ONLY_LABELLED = {"windows-aarch64"}


def shipped_targets(history):
    return {target for release in history for target in release.get("downloads", {})} - ONLY_LABELLED


def production_targets(labels, manifest, history, respin=True):
    # invariant: one production version covers every target ever shipped, so no platform falls behind the number
    # (ONLY_LABELLED targets aside)
    history = [r for r in history if parse_version(r.get("version"))]
    every = {f"{p}-{arch}" for p in PLATFORMS for arch in archs(p, labels)} | shipped_targets(history)
    # why: an arch label without the os label is the owner naming the archs of that os, so shipped others stay out
    for p in PLATFORMS:
        named = {arch for label, arch in ARCH_LABELS[p] if label in labels}
        if named and OS_LABEL[p] not in labels:
            every -= {f"{p}-{arch}" for _, arch in ARCH_LABELS[p] if arch not in named}
    narrowed = [p for p in PLATFORMS if (LABELS_OF[p] - {LOCAL_LABELS[p]}) & labels]
    wanted = {f"{p}-{arch}" for p in narrowed for arch in archs(p, labels)}
    newest = max(history, key=lambda r: parse_version(r.get("version")), default=None)
    if (
        respin and wanted and wanted != every and newest
        and parse_version(newest.get("version")) >= parse_version(manifest)
        and not wanted & set(newest.get("downloads", {}))
    ):
        return newest["version"], sorted(wanted)
    return None, sorted(every)


def release_version(manifest, labels, history=published):
    floor = parse_version(manifest)
    shipped = history("staging") + history("production")
    if not shipped:
        return manifest
    return dotted(max(next_version(max(shipped), level_of(labels)), floor))


def stored_by_hand(key, fetch, manifest):
    tag = re.compile(rf"payload-{key}(?:-v(\d+\.\d+\.\d+))?-b(\d+)")
    kept = {}
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
            kept[(found.group(1) or manifest, int(found.group(2)))] = (release["tag_name"], assets)
    return kept


def compiled_before(key, fetch, manifest):
    name = re.compile(rf"payload-{key}(?:-v(\d+\.\d+\.\d+))?-b(\d+)-([a-z]+-[a-z0-9_]+)")
    kept = {}
    for page in range(1, 11):
        batch = fetch(f"actions/artifacts?per_page=100&page={page}")["artifacts"]
        for artifact in batch:
            found = name.fullmatch(artifact["name"])
            if found and not artifact["expired"]:
                build = (found.group(1) or manifest, int(found.group(2)))
                kept.setdefault(build, {}).setdefault(found.group(3), (artifact["workflow_run"]["id"], artifact["name"]))
        if len(batch) < 100:
            break
    return kept


def decide(mode, event):
    pr = event["pull_request"]
    labels = {label["name"] for label in pr.get("labels", [])}
    base, head = pr["base"]["ref"], pr["head"]["ref"]
    action = event.get("action", "")
    if mode == "build":
        if action in ("opened", "reopened", "ready_for_review") and base in FLOW:
            labels.add("build")
        added = (event.get("label") or {}).get("name")
        if action in ("labeled", "unlabeled") and added not in REBUILD_LABELS:
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
    if base == "staging" and not staging_ships():
        return None, "staging releases are switched off, the merge into main ships", labels
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
    placement = runners(labels, head_repo == pr["base"]["repo"]["full_name"])
    out = {
        "run": "false",
        "reason": reason,
        "linux_archs": json.dumps(archs("linux", labels)),
        "windows_archs": json.dumps(archs("windows", labels)),
        "macos_archs": json.dumps(archs("macos", labels)),
        **placement,
    }
    if kind:
        env = ci_env(root)
        picked = [p for p in PLATFORMS if LABELS_OF[p] & labels] or list(PLATFORMS)
        environment = environment_of(kind, pr["base"]["ref"], labels)
        manifest = workspace_version(root)
        version = manifest
        build_number = os.environ.get("GITHUB_RUN_NUMBER", "1")
        sha = pr.get("merge_commit_sha") if kind == "release" else pr["head"]["sha"]
        tree, payloads, payload_names, payload_tag, payload_assets = "", {}, {}, "", {}
        if kind == "release":
            kept = None
            if os.environ.get("GH_TOKEN"):
                top = github(f"git/commits/{sha}")["tree"]["sha"]
                listing = github(f"git/trees/{top}?recursive=1")
                if listing.get("truncated"):
                    tree = top[:12]
                else:
                    tree = source_key((e["path"], e["sha"]) for e in listing["tree"] if e["type"] != "tree")
                by_hand = stored_by_hand(tree, github, manifest)
                by_run = compiled_before(tree, github, manifest)
                builds = sorted(set(by_hand) | set(by_run), key=lambda build: (parse_version(build[0]), build[1]))
                kept = builds[-1] if builds else None
            if kept:
                version, build_number = kept[0], str(kept[1])
                payload_tag, payload_assets = by_hand.get(kept, ("", {}))
                for target, (run, name) in by_run.get(kept, {}).items():
                    if target not in payload_assets:
                        payloads[target], payload_names[target] = run, name
            else:
                version = release_version(manifest, labels)
        if kind == "release" and environment == "production":
            respin, targets = production_targets(labels, manifest, history_of("production"), respin=not kept)
            if respin:
                version = respin
                print(f"::notice::{', '.join(targets)} join {respin}, which shipped without them")
            else:
                print(f"::notice::{version} ships every target: {', '.join(targets)}")
            picked = [p for p in PLATFORMS if any(t.startswith(f"{p}-") for t in targets)]
            for p in PLATFORMS:
                out[f"{p}_archs"] = json.dumps([t.split("-", 1)[1] for t in targets if t.startswith(f"{p}-")] or archs(p, labels))
            shipped = already_shipped(version, targets)
            if shipped and not kept:
                sys.exit(f"::error::{version} is already in production for {', '.join(shipped)}: pick other platforms or change the sources")
            if shipped:
                # why: the same sources rerun after one platform failed, so only the missing ones build and join the version
                targets = [t for t in targets if t not in shipped]
                if not targets:
                    sys.exit(f"::error::{version} of these sources is already in production for every target")
                print(f"::notice::{', '.join(shipped)} already ship {version}, building {', '.join(targets)}")
                picked = [p for p in PLATFORMS if any(t.startswith(f"{p}-") for t in targets)]
                for p in PLATFORMS:
                    out[f"{p}_archs"] = json.dumps([t.split("-", 1)[1] for t in targets if t.startswith(f"{p}-")])
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
            "payload_names": json.dumps(payload_names),
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
