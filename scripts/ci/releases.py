#!/usr/bin/env python3
import html
import json
import os
import re
import sys

LATEST = re.compile(r"-\d+\.\d+\.\d+-")
TARGET_TITLES = {
    "macos-aarch64": "macOS (Apple silicon)",
    "macos-x86_64": "macOS (Intel)",
    "linux-x86_64": "Linux x86_64",
    "linux-aarch64": "Linux aarch64",
    "windows-x86_64": "Windows x86_64",
    "windows-aarch64": "Windows arm64",
}
EXT_TITLES = {"dmg": "dmg", "zip": "zip", "AppImage": "AppImage"}


def version_key(raw):
    return tuple(int(part) for part in raw.split("."))


def latest_name(url):
    return LATEST.sub("-", url.rsplit("/", 1)[1], count=1)


def merge(feed_path, current_path, version):
    feed = json.load(open(feed_path, encoding="utf-8"))
    release = next(r for r in feed["releases"] if r["version"] == version)
    history = {"releases": []}
    if os.path.isfile(current_path) and os.path.getsize(current_path):
        history = json.load(open(current_path, encoding="utf-8"))
        if not isinstance(history.get("releases"), list):
            sys.exit(f"{current_path} is not a release history")
    releases = [r for r in history["releases"] if r.get("version") != version]
    releases.append({
        "version": release["version"],
        "build": release["build"],
        "channel": release["channel"],
        "published_at": release["published_at"],
        "downloads": release["downloads"],
    })
    releases.sort(key=lambda r: version_key(r["version"]), reverse=True)
    return releases


def latest_of(releases, latest_base):
    latest = {}
    for release in sorted(releases, key=lambda r: version_key(r["version"])):
        for target, files in release["downloads"].items():
            for ext, entry in files.items():
                latest.setdefault(target, {})[ext] = {
                    "url": entry["url"],
                    "stable_url": f"{latest_base.rstrip('/')}/{latest_name(entry['url'])}",
                    "version": release["version"],
                    "build": release["build"],
                    "size": entry["size"],
                    "sha256": entry["sha256"],
                }
    return latest


def size_text(size):
    return f"{size / (1 << 20):.1f} MB"


def links(files, show_version):
    parts = []
    for ext in sorted(files):
        entry = files[ext]
        label = EXT_TITLES.get(ext, ext)
        if show_version:
            label = f"{label} {entry['version']}"
        parts.append(
            f'<a href="{html.escape(entry["url"])}" title="sha256 {entry["sha256"]}">{html.escape(label)}</a>'
            f' <span class="size">{size_text(entry["size"])}</span>'
        )
    return " &middot; ".join(parts)


def page(environment, slug, releases, latest):
    title = f"{slug} desktop downloads"
    rows = []
    for target in sorted(latest):
        name = TARGET_TITLES.get(target, target)
        rows.append(f"<tr><th>{html.escape(name)}</th><td>{links(latest[target], True)}</td></tr>")
    history = []
    for release in releases:
        cells = []
        for target in sorted(release["downloads"]):
            files = {
                ext: {**entry, "version": release["version"]}
                for ext, entry in release["downloads"][target].items()
            }
            cells.append(f"{html.escape(TARGET_TITLES.get(target, target))}: {links(files, False)}")
        history.append(
            f"<tr><th>{html.escape(release['version'])}</th><td>{release['build']}</td>"
            f"<td>{html.escape(release['published_at'][:10])}</td><td>{'<br>'.join(cells)}</td></tr>"
        )
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="{'noindex' if environment != 'production' else 'index'}">
<title>{html.escape(title)}</title>
<style>
:root {{ color-scheme: light dark; --fg: #111; --bg: #fff; --muted: #666; --line: #e5e5e5; --link: #0a66d8; }}
@media (prefers-color-scheme: dark) {{ :root {{ --fg: #eee; --bg: #111; --muted: #999; --line: #2a2a2a; --link: #6aa8ff; }} }}
body {{ margin: 0; padding: 32px 16px; font: 15px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; color: var(--fg); background: var(--bg); }}
main {{ max-width: 860px; margin: 0 auto; }}
h1 {{ font-size: 24px; margin: 0 0 4px; }}
h2 {{ font-size: 17px; margin: 32px 0 8px; }}
p {{ color: var(--muted); margin: 0; }}
table {{ width: 100%; border-collapse: collapse; }}
th, td {{ text-align: left; vertical-align: top; padding: 8px 12px 8px 0; border-top: 1px solid var(--line); }}
a {{ color: var(--link); text-decoration: none; }}
a:hover {{ text-decoration: underline; }}
.size {{ color: var(--muted); font-size: 13px; }}
.wrap {{ overflow-x: auto; }}
</style>
</head>
<body>
<main>
<h1>{html.escape(title)}</h1>
<p>{html.escape(environment)} builds. The links under Latest always point at the newest build.</p>
<h2>Latest</h2>
<div class="wrap"><table>{''.join(rows)}</table></div>
<h2>All versions</h2>
<div class="wrap"><table><tr><th>Version</th><th>Build</th><th>Date</th><th>Files</th></tr>{''.join(history)}</table></div>
</main>
</body>
</html>
"""


def main():
    if len(sys.argv) != 6:
        sys.exit("usage: releases.py <feed> <current releases or empty> <out releases> <out index.html> <out copies>")
    feed_path, current_path, out_json, out_html, out_copies = sys.argv[1:]
    environment = os.environ["ENVIRONMENT"]
    slug = os.environ["SLUG"]
    version = os.environ["VERSION"]
    releases = merge(feed_path, current_path, version)
    latest = latest_of(releases, os.environ["LATEST_URL"])
    current = next(r for r in releases if r["version"] == version)
    with open(out_copies, "w", encoding="utf-8") as sink:
        for target, files in sorted(current["downloads"].items()):
            for ext, entry in sorted(files.items()):
                if latest[target][ext]["version"] == version:
                    name = entry["url"].rsplit("/", 1)[1]
                    sink.write(f"{name}\t{latest_name(entry['url'])}\n")
    with open(out_json, "w", encoding="utf-8") as sink:
        json.dump({"environment": environment, "latest": latest, "releases": releases}, sink, indent=2, sort_keys=True)
        sink.write("\n")
    with open(out_html, "w", encoding="utf-8") as sink:
        sink.write(page(environment, slug, releases, latest))
    for target, files in sorted(latest.items()):
        for ext, entry in sorted(files.items()):
            print(f"latest {target} {ext}: {entry['url']} ({entry['version']})")


if __name__ == "__main__":
    main()
