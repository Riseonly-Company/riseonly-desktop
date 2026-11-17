#!/usr/bin/env bash
set -euo pipefail

PROFILE="${1:-debug}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RES="$ROOT/app/resources/macos"
BIN="${CARGO_TARGET_DIR:-$ROOT/target}/${RISE_MACOS_TARGET:+$RISE_MACOS_TARGET/}$PROFILE/riseonly"
CANON="${RISE_CANON_DIR:-$ROOT/../riseonly-tools/canon}/deep_links.json"

[ -f "$CANON" ] || { echo "missing canon: $CANON" >&2; exit 1; }

canon() { plutil -extract "$1" raw -o - "$CANON"; }

if [ "$PROFILE" = "debug" ]; then
    ENVIRONMENT="dev"
else
    ENVIRONMENT="production"
fi
if [ -n "${RISE_ENV+set}" ]; then
    compiled="$RISE_ENV"
    compiled="${compiled#"${compiled%%[![:space:]]*}"}"
    compiled="${compiled%"${compiled##*[![:space:]]}"}"
    compiled="$(printf '%s' "$compiled" | tr '[:upper:]' '[:lower:]')"
    count="$(canon environments)"
    i=0
    while [ "$i" -lt "$count" ]; do
        values="$(canon "environments.$i.app_env_values")"
        j=0
        while [ "$j" -lt "$values" ]; do
            if [ "$(canon "environments.$i.app_env_values.$j")" = "$compiled" ]; then
                ENVIRONMENT="$(canon "environments.$i.name")"
            fi
            j=$((j + 1))
        done
        i=$((i + 1))
    done
fi

index=""
count="$(canon environments)"
i=0
while [ "$i" -lt "$count" ]; do
    [ "$(canon "environments.$i.name")" = "$ENVIRONMENT" ] && index="$i"
    i=$((i + 1))
done
[ -n "$index" ] || { echo "'$ENVIRONMENT' names no canon environment" >&2; exit 1; }
SCHEME="$(canon "environments.$index.url_scheme")"
WEB_HOST="$(canon "environments.$index.web_host")"

case "$ENVIRONMENT" in
    dev)        APP_NAME="Riseonly Dev";     BUNDLE_ID="net.riseonly.desktop.dev" ;;
    staging)    APP_NAME="Riseonly Staging"; BUNDLE_ID="net.riseonly.desktop.staging" ;;
    production) APP_NAME="Riseonly";         BUNDLE_ID="net.riseonly.desktop" ;;
    *) echo "canon environment '$ENVIRONMENT' has no bundle identity here" >&2; exit 1 ;;
esac

SHORT_VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml")"
[ -n "$SHORT_VERSION" ] || { echo "no [workspace.package] version in $ROOT/Cargo.toml" >&2; exit 1; }
BUILD_NUMBER="${RISE_BUILD_NUMBER:-1}"
case "$BUILD_NUMBER" in
    ''|.*|*.|*..*|*[!0-9.]*) BUILD_NUMBER=1 ;;
esac

OUT="$ROOT/build/macos/$PROFILE/$APP_NAME.app"

[ -x "$BIN" ] || { echo "missing binary: $BIN — run cargo build -p riseonly first" >&2; exit 1; }

rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"

cp "$BIN" "$OUT/Contents/MacOS/riseonly"

sed -e "s|@APP_NAME@|$APP_NAME|g" \
    -e "s|@BUNDLE_ID@|$BUNDLE_ID|g" \
    -e "s|@URL_SCHEME@|$SCHEME|g" \
    -e "s|@SHORT_VERSION@|$SHORT_VERSION|g" \
    -e "s|@BUILD_NUMBER@|$BUILD_NUMBER|g" \
    "$RES/Info.plist.in" > "$OUT/Contents/Info.plist"
printf 'APPL????' > "$OUT/Contents/PkgInfo"

for lproj in "$RES"/*.lproj; do
    [ -d "$lproj" ] || continue
    cp -R "$lproj" "$OUT/Contents/Resources/"
done

if [ -d "$ROOT/assets" ]; then
    cp -R "$ROOT/assets" "$OUT/Contents/Resources/assets"
fi

FFMPEG_LIB_PATTERN='^lib(avcodec|avdevice|avfilter|avformat|avutil|swresample|swscale)\.[0-9]+\.dylib$'
ARCH="$(lipo -archs "$BIN" 2>/dev/null | awk '{print $1}')"
FFMPEG_PREFIX="$ROOT/vendor/ffmpeg/macos-${ARCH:-$(uname -m)}"
FRAMEWORKS="$OUT/Contents/Frameworks"
EXE="$OUT/Contents/MacOS/riseonly"

linked_ffmpeg() {
    otool -L "$1" | awk 'NR > 1 {print $1}' | while read -r path; do
        base="$(basename "$path")"
        if printf '%s' "$base" | grep -Eq "$FFMPEG_LIB_PATTERN"; then
            printf '%s %s\n' "$path" "$base"
        fi
    done
}

if [ -n "$(linked_ffmpeg "$EXE")" ]; then
    [ -d "$FFMPEG_PREFIX/lib" ] || { echo "binary links FFmpeg but $FFMPEG_PREFIX is missing" >&2; exit 1; }
    mkdir -p "$FRAMEWORKS"
    queue=("$EXE")
    next=0
    while [ "$next" -lt "${#queue[@]}" ]; do
        current="${queue[$next]}"
        next=$((next + 1))
        own="$(basename "$current")"
        while read -r path base; do
            [ "$base" = "$own" ] && continue
            install_name_tool -change "$path" "@rpath/$base" "$current"
            [ -f "$FRAMEWORKS/$base" ] && continue
            [ -f "$FFMPEG_PREFIX/lib/$base" ] || { echo "missing $FFMPEG_PREFIX/lib/$base" >&2; exit 1; }
            cp -L "$FFMPEG_PREFIX/lib/$base" "$FRAMEWORKS/$base"
            chmod u+w "$FRAMEWORKS/$base"
            install_name_tool -id "@rpath/$base" "$FRAMEWORKS/$base"
            queue+=("$FRAMEWORKS/$base")
        done < <(linked_ffmpeg "$current")
    done
    otool -l "$EXE" | grep -q '@executable_path/../Frameworks' ||
        install_name_tool -add_rpath "@executable_path/../Frameworks" "$EXE"
    for dylib in "$FRAMEWORKS"/*.dylib; do
        codesign --force --sign - --timestamp=none "$dylib" 2>&1 | sed 's/^/codesign: /'
    done

    LICENSES="$OUT/Contents/Resources/licenses/ffmpeg"
    mkdir -p "$LICENSES"
    TARBALL="$(sed -n 's/^source_tarball=//p' "$FFMPEG_PREFIX/BUILD_INFO.txt" 2>/dev/null || true)"
    [ -n "$TARBALL" ] && [ -f "$ROOT/vendor/ffmpeg/$TARBALL" ] ||
        { echo "FFmpeg source tarball for $FFMPEG_PREFIX not found; LGPL needs it shipped" >&2; exit 1; }
    cp "$ROOT/vendor/ffmpeg/$TARBALL" "$LICENSES/$TARBALL"
    cp "$FFMPEG_PREFIX/BUILD_INFO.txt" "$LICENSES/BUILD_INFO.txt"
    echo "ffmpeg: $(ls "$FRAMEWORKS" | tr '\n' ' ')"
fi

for tree in "$ROOT"/vendor/rlottie/rlottie-*/; do
    [ -d "$tree" ] || continue
    tree="${tree%/}"
    report="$(python3 "$ROOT/scripts/check-rlottie-license.py" "$tree")" ||
        { printf '%s\n' "$report" >&2; exit 1; }
    LICENSES="$OUT/Contents/Resources/licenses/$(basename "$tree")"
    mkdir -p "$LICENSES"
    cp "$tree"/licenses/COPYING.* "$LICENSES/"
    printf '%s\n' "$report" | sed -n 's/^  MPL-2.0, source must ship with the release: //p' |
        while read -r file; do
            mkdir -p "$LICENSES/source/$(dirname "$file")"
            cp "$tree/$file" "$LICENSES/source/$file"
        done
done

ICON_MASTER="$RES/AppIcon.png"
ICON_OUT="$ROOT/build/macos/AppIcon.icns"
if [ -f "$ICON_MASTER" ]; then
    if [ ! -f "$ICON_OUT" ] || [ "$ICON_MASTER" -nt "$ICON_OUT" ]; then
        "$ROOT/scripts/make-icns.sh" "$ICON_MASTER" "$ICON_OUT"
    fi
    cp "$ICON_OUT" "$OUT/Contents/Resources/AppIcon.icns"
else
    echo "warning: no $ICON_MASTER; the bundle will show the generic app icon" >&2
fi

codesign --force --sign - \
    --entitlements "$RES/Riseonly.entitlements" \
    --options runtime \
    --timestamp=none \
    "$OUT" 2>&1 | sed 's/^/codesign: /'

/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
    -f "$OUT" >/dev/null 2>&1 || true
touch "$OUT"

echo "bundled: $OUT  ($ENVIRONMENT, $SCHEME, $WEB_HOST)"
