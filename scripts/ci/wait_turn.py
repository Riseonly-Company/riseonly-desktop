#!/usr/bin/env python3
"""holds a linux build that compiles from scratch until macos and windows of this run are done"""
import json
import os
import time
import urllib.request

repo, run = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_RUN_ID"]
windows_planned = os.environ.get("WINDOWS_PLANNED") == "true"


def jobs():
    found, page = [], 1
    while True:
        request = urllib.request.Request(
            f"https://api.github.com/repos/{repo}/actions/runs/{run}/jobs?per_page=100&page={page}",
            headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}", "Accept": "application/vnd.github+json"},
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            batch = json.load(response)["jobs"]
        found += batch
        if len(batch) < 100:
            return found
        page += 1


started = time.time()
while True:
    try:
        listed = jobs()
    except OSError as error:
        print(f"jobs not read ({error}), retrying")
        time.sleep(30)
        continue
    ahead = [j for j in listed if j["name"].split(" ")[0] in ("macos", "windows")]
    busy = [j["name"] for j in ahead if j["status"] != "completed"]
    # why: windows jobs appear only once macos is done, so a planned windows with no job yet is still ahead of us
    if windows_planned and not any(j["name"].startswith("windows") for j in ahead):
        busy.append("windows (not started)")
    if not busy:
        print(f"macos and windows are done after {int(time.time() - started)}s, linux builds now")
        break
    print(f"waiting for {', '.join(busy)}")
    time.sleep(60)
