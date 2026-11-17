#!/usr/bin/env python3
import datetime
import hashlib
import json
import os
import re
import sys

NAME = re.compile(r"^(?P<slug>.+)-(?P<version>\d+\.\d+\.\d+)-(?P<target>(macos|linux|windows)-(x86_64|aarch64))\.(?P<ext>zip|dmg|AppImage)$")
UPDATER_EXT = {"macos": "zip", "windows": "zip", "linux": "AppImage"}


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for chunk in iter(lambda: source.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def version_key(raw):
    return tuple(int(part) for part in raw.split("."))


def collect(dist, version, base_url):
    artifacts, downloads = {}, {}
    for directory, _, files in os.walk(dist):
        for file_name in sorted(files):
            found = NAME.match(file_name)
            if not found:
                continue
            if found["version"] != version:
                sys.exit(f"{file_name} is not version {version}")
            path = os.path.join(directory, file_name)
            entry = {"url": f"{base_url.rstrip('/')}/{file_name}", "size": os.path.getsize(path), "sha256": sha256(path)}
            target, ext = found["target"], found["ext"]
            downloads.setdefault(target, {})[ext] = entry
            if UPDATER_EXT[target.split("-")[0]] == ext:
                artifacts[target] = entry
    if not artifacts:
        sys.exit(f"no build files for {version} under {dist}")
    return artifacts, downloads


def main():
    if len(sys.argv) != 4:
        sys.exit("usage: feed.py <dist dir> <current feed or -> <out feed>")
    dist, current, out = sys.argv[1:]
    version = os.environ["VERSION"]
    artifacts, downloads = collect(dist, version, os.environ["DOWNLOADS_URL"])
    feed = {"releases": []}
    if current != "-" and os.path.isfile(current) and os.path.getsize(current):
        feed = json.load(open(current, encoding="utf-8"))
        if not isinstance(feed.get("releases"), list):
            sys.exit(f"{current} is not a feed")
    releases = [r for r in feed["releases"] if r.get("version") != version]
    previous = next((r for r in feed["releases"] if r.get("version") == version), None)
    if previous:
        artifacts = {**previous.get("artifacts", {}), **artifacts}
        downloads = {**previous.get("downloads", {}), **downloads}
    releases.append({
        "version": version,
        "channel": os.environ.get("CHANNEL", "stable"),
        "build": int(os.environ.get("BUILD_NUMBER", "0")),
        "published_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "artifacts": artifacts,
        "downloads": downloads,
    })
    releases.sort(key=lambda r: version_key(r["version"]), reverse=True)
    feed["releases"] = releases[: int(os.environ.get("KEEP", "12"))]
    feed["generated_at"] = releases[0]["published_at"] if releases else None
    with open(out, "w", encoding="utf-8") as sink:
        json.dump(feed, sink, indent=2, sort_keys=True)
        sink.write("\n")
    for target, entry in sorted(artifacts.items()):
        print(f"{target}: {entry['url']} {entry['size']} bytes")


if __name__ == "__main__":
    main()
