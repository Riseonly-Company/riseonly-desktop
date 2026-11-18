#!/usr/bin/env python3
import re
import sys


def main():
    if len(sys.argv) != 2 or not re.fullmatch(r"\d+\.\d+\.\d+", sys.argv[1]):
        sys.exit("usage: set-version.py <x.y.z>")
    new = sys.argv[1]
    manifest = open("Cargo.toml", encoding="utf-8").read()
    block = re.search(r"^\[workspace\.package\]\s*\n(.*?)(?=^\[|\Z)", manifest, re.S | re.M)
    found = block and re.search(r'^version\s*=\s*"([^"]+)"', block.group(1), re.M)
    if not found:
        sys.exit("no [workspace.package] version in Cargo.toml")
    old = found.group(1)
    if old == new:
        print(f"version {new}, as in Cargo.toml")
        return
    start = block.start(1) + found.start(1)
    with open("Cargo.toml", "w", encoding="utf-8") as sink:
        sink.write(manifest[:start] + new + manifest[start + len(old):])
    packages = open("Cargo.lock", encoding="utf-8").read().split("\n[[package]]\n")
    moved = 0
    for index, package in enumerate(packages):
        lines = package.split("\n")
        if f'version = "{old}"' in lines and not any(line.startswith("source = ") for line in lines):
            packages[index] = package.replace(f'version = "{old}"', f'version = "{new}"', 1)
            moved += 1
    with open("Cargo.lock", "w", encoding="utf-8") as sink:
        sink.write("\n[[package]]\n".join(packages))
    print(f"version {old} -> {new}, {moved} workspace packages in Cargo.lock")


if __name__ == "__main__":
    main()
