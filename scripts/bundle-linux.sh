#!/usr/bin/env bash
set -euo pipefail

PROFILE="${1:-debug}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RES="$ROOT/app/resources/linux"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}/$PROFILE"
BIN="$TARGET/riseonly"
HELPER="$TARGET/riseonly-web-view-helper"
CANON="${RISE_CANON_DIR:-$ROOT/../riseonly-tools/canon}/deep_links.json"

[ -f "$CANON" ] || { echo "missing canon: $CANON" >&2; exit 1; }
[ -x "$BIN" ] || { echo "missing binary: $BIN, run cargo build -p riseonly first" >&2; exit 1; }
[ -x "$HELPER" ] || {
    echo "missing web view helper: $HELPER, run cargo build -p riseonly-web-view-helper --features helper first" >&2
    exit 1
}

canon_environment() {
    python3 - "$CANON" "$PROFILE" <<'PY'
import json
import os
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    canon = json.load(source)
environments = canon["environments"]
name = canon["app_env"]["fallback_environment"] if sys.argv[2] == "debug" else "production"
compiled = os.environ.get("RISE_ENV")
if compiled is not None:
    normalized = compiled.strip().lower()
    for environment in environments:
        if normalized in environment["app_env_values"]:
            name = environment["name"]
for environment in environments:
    if environment["name"] == name:
        print(name, environment["url_scheme"], environment["web_host"])
        break
else:
    sys.exit("canon fallback '%s' names no environment" % name)
PY
}
read -r ENVIRONMENT SCHEME WEB_HOST < <(canon_environment)
[ -n "${WEB_HOST:-}" ] || { echo "no environment read from $CANON" >&2; exit 1; }

case "$ENVIRONMENT" in
    dev)        APP_NAME="Riseonly Dev";     BUNDLE_ID="net.riseonly.desktop.dev" ;;
    staging)    APP_NAME="Riseonly Staging"; BUNDLE_ID="net.riseonly.desktop.staging" ;;
    production) APP_NAME="Riseonly";         BUNDLE_ID="net.riseonly.desktop" ;;
    *) echo "canon environment '$ENVIRONMENT' has no bundle identity here" >&2; exit 1 ;;
esac

SHORT_VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml")"
[ -n "$SHORT_VERSION" ] || { echo "no [workspace.package] version in $ROOT/Cargo.toml" >&2; exit 1; }

OUT="$ROOT/build/linux/$PROFILE/usr"
rm -rf "$OUT"
mkdir -p "$OUT/bin" "$OUT/share/applications" "$OUT/share/metainfo" "$OUT/share/riseonly"

cp "$BIN" "$OUT/bin/riseonly"
cp "$HELPER" "$OUT/bin/riseonly-web-view-helper"
EXE="$OUT/bin/riseonly"

fill() {
    sed -e "s|@APP_NAME@|$APP_NAME|g" \
        -e "s|@BUNDLE_ID@|$BUNDLE_ID|g" \
        -e "s|@URL_SCHEME@|$SCHEME|g" \
        -e "s|@WEB_HOST@|$WEB_HOST|g" \
        -e "s|@SHORT_VERSION@|$SHORT_VERSION|g" \
        "$1"
}
fill "$RES/riseonly.desktop.in" > "$OUT/share/applications/$BUNDLE_ID.desktop"
fill "$RES/riseonly.metainfo.xml.in" > "$OUT/share/metainfo/$BUNDLE_ID.metainfo.xml"

if [ -d "$ROOT/assets" ]; then
    cp -R "$ROOT/assets" "$OUT/share/riseonly/assets"
fi
printf '%s\n' "$ENVIRONMENT" > "$OUT/share/riseonly/environment"

FFMPEG_LIB_PATTERN='^lib(avcodec|avdevice|avfilter|avformat|avutil|swresample|swscale)\.so\.[0-9]+$'
case "$(uname -m)" in
    aarch64 | arm64) ARCH=arm64 ;;
    *) ARCH="$(uname -m)" ;;
esac
FFMPEG_PREFIX="$ROOT/vendor/ffmpeg/linux-$ARCH"
LIBS="$OUT/lib/riseonly"

linked_ffmpeg() {
    readelf -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\].*/\1/p' | grep -E "$FFMPEG_LIB_PATTERN" || true
}

if [ -n "$(linked_ffmpeg "$EXE")" ]; then
    command -v patchelf >/dev/null || { echo "the binary links FFmpeg and patchelf is missing" >&2; exit 1; }
    [ -d "$FFMPEG_PREFIX/lib" ] || { echo "binary links FFmpeg but $FFMPEG_PREFIX is missing" >&2; exit 1; }
    mkdir -p "$LIBS"
    queue=("$EXE")
    next=0
    while [ "$next" -lt "${#queue[@]}" ]; do
        current="${queue[$next]}"
        next=$((next + 1))
        while read -r base; do
            [ -n "$base" ] || continue
            [ -f "$LIBS/$base" ] && continue
            [ -f "$FFMPEG_PREFIX/lib/$base" ] || { echo "missing $FFMPEG_PREFIX/lib/$base" >&2; exit 1; }
            cp -L "$FFMPEG_PREFIX/lib/$base" "$LIBS/$base"
            chmod u+w "$LIBS/$base"
            patchelf --set-rpath '$ORIGIN' "$LIBS/$base"
            queue+=("$LIBS/$base")
        done < <(linked_ffmpeg "$current")
    done
    patchelf --add-rpath '$ORIGIN/../lib/riseonly' "$EXE"

    LICENSES="$OUT/share/licenses/riseonly/ffmpeg"
    mkdir -p "$LICENSES"
    TARBALL="$(sed -n 's/^source_tarball=//p' "$FFMPEG_PREFIX/BUILD_INFO.txt" 2>/dev/null || true)"
    [ -n "$TARBALL" ] && [ -f "$ROOT/vendor/ffmpeg/$TARBALL" ] ||
        { echo "FFmpeg source tarball for $FFMPEG_PREFIX not found; LGPL needs it shipped" >&2; exit 1; }
    cp "$ROOT/vendor/ffmpeg/$TARBALL" "$LICENSES/$TARBALL"
    cp "$FFMPEG_PREFIX/BUILD_INFO.txt" "$LICENSES/BUILD_INFO.txt"
    echo "ffmpeg: $(ls "$LIBS" | tr '\n' ' ')"
fi

for soname in libuv.so.1; do
    readelf -d "$EXE" | grep -q "(NEEDED).*\[$soname\]" || continue
    lib=$(ldconfig -p | awk -v n="$soname" '$1 == n { print $NF; exit }')
    [ -n "$lib" ] || { echo "the binary needs $soname and the build machine has none" >&2; exit 1; }
    mkdir -p "$LIBS"
    cp -L "$lib" "$LIBS/$soname"
    readelf -d "$EXE" | grep -q 'ORIGIN/../lib/riseonly' || patchelf --add-rpath '$ORIGIN/../lib/riseonly' "$EXE"
    echo "bundled $soname"
done

for tree in "$ROOT"/vendor/rlottie/rlottie-*/; do
    [ -d "$tree" ] || continue
    tree="${tree%/}"
    report="$(python3 "$ROOT/scripts/check-rlottie-license.py" "$tree")" ||
        { printf '%s\n' "$report" >&2; exit 1; }
    LICENSES="$OUT/share/licenses/riseonly/$(basename "$tree")"
    mkdir -p "$LICENSES"
    cp "$tree"/licenses/COPYING.* "$LICENSES/"
    printf '%s\n' "$report" | sed -n 's/^  MPL-2.0, source must ship with the release: //p' |
        while read -r file; do
            mkdir -p "$LICENSES/source/$(dirname "$file")"
            cp "$tree/$file" "$LICENSES/source/$file"
        done
done

ICON_MASTER="$ROOT/app/resources/macos/AppIcon.png"
[ -f "$ICON_MASTER" ] || { echo "missing icon master: $ICON_MASTER" >&2; exit 1; }
resize_icon() {
    if command -v magick >/dev/null; then
        magick "$ICON_MASTER" -resize "$1x$1" "$2"
    elif command -v convert >/dev/null; then
        convert "$ICON_MASTER" -resize "$1x$1" "$2"
    elif command -v sips >/dev/null; then
        sips -z "$1" "$1" "$ICON_MASTER" --out "$2" >/dev/null
    else
        echo "no image tool for the icons: install ImageMagick" >&2
        exit 1
    fi
}
for size in 16 24 32 48 64 128 256 512; do
    ICONS="$OUT/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$ICONS"
    resize_icon "$size" "$ICONS/$BUNDLE_ID.png"
    cp "$ICONS/$BUNDLE_ID.png" "$ICONS/riseonly.png"
done

echo "bundled: $OUT  ($ENVIRONMENT, $SCHEME, $WEB_HOST)"
