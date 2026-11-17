#!/usr/bin/env bash
set -euo pipefail

usage="usage: payload.sh <build number> <platform-arch>=<riseonly binary> ..."
build=${1:?$usage}
shift
[ $# -gt 0 ] || { echo "$usage" >&2; exit 1; }
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
commit=$(git rev-parse HEAD)
tag="payload-$(python3 "$here/plan.py" key "$commit")-b$build"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

files=()
for pair in "$@"; do
    target=${pair%%=*}
    binary=${pair#*=}
    [ -f "$binary" ] || { echo "no file $binary" >&2; exit 1; }
    mkdir -p "$work/$target"
    cp "$binary" "$work/$target/riseonly"
    chmod +x "$work/$target/riseonly"
    tar -czf "$work/$target.tar.gz" -C "$work/$target" riseonly
    files+=("$work/$target.tar.gz")
done

if gh release view "$tag" >/dev/null 2>&1; then
    gh release upload "$tag" --clobber "${files[@]}"
else
    gh release create "$tag" --prerelease --target "$commit" --title "$tag" --notes "" "${files[@]}"
fi
echo "$tag"
