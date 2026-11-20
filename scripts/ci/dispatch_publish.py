#!/usr/bin/env python3
"""starts publish.yml for one finished build so it ships without waiting for the other platforms"""
import json
import os
import urllib.request

plan = json.loads(os.environ["PLAN"])
repo = os.environ["GITHUB_REPOSITORY"]
body = {
    "ref": os.environ["BRANCH"],
    "inputs": {
        "run_id": os.environ["GITHUB_RUN_ID"],
        "target": os.environ["TARGET"],
        "environment": plan["environment"],
        "version": plan["version"],
        "build_number": plan["build_number"],
        "tag": plan["tag"],
        "sha": plan["sha"],
        "slug": plan["slug"],
    },
}
request = urllib.request.Request(
    f"https://api.github.com/repos/{repo}/actions/workflows/publish.yml/dispatches",
    data=json.dumps(body).encode(),
    method="POST",
    headers={
        "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
        "Accept": "application/vnd.github+json",
        "Content-Type": "application/json",
    },
)
with urllib.request.urlopen(request, timeout=30) as response:
    print(f"publish of {os.environ['TARGET']} {plan['version']} started (http {response.status})")
