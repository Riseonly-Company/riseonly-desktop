#!/usr/bin/env bash
# Builds a macOS .icns from one square master.
#
# `sips` and `iconutil` ship with macOS, so this needs no dependency. The size
# list is Apple's: omitting a size does not scale the nearest one, it leaves the
# Dock to pick a worse source, which is how an icon ends up soft at 2x.
set -euo pipefail

SRC="${1:?usage: make-icns.sh <master.png> <out.icns>}"
OUT="${2:?usage: make-icns.sh <master.png> <out.icns>}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

SET="$WORK/AppIcon.iconset"
mkdir -p "$SET"

for entry in "16 icon_16x16" "32 icon_16x16@2x" "32 icon_32x32" "64 icon_32x32@2x" \
             "128 icon_128x128" "256 icon_128x128@2x" "256 icon_256x256" \
             "512 icon_256x256@2x" "512 icon_512x512" "1024 icon_512x512@2x"; do
    set -- $entry
    sips -z "$1" "$1" "$SRC" --out "$SET/$2.png" >/dev/null
done

mkdir -p "$(dirname "$OUT")"
iconutil -c icns "$SET" -o "$OUT"
echo "icns: $OUT"
