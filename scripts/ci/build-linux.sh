#!/usr/bin/env bash
set -euo pipefail

profile=${PROFILE:-fast}
slug=${SLUG:-Riseonly}
version=${VERSION:?VERSION is required}
target=${CARGO_TARGET_DIR:-target}

export MLN_PRECOMPILE=${MLN_PRECOMPILE:-1}

./scripts/vendor-rlottie.sh
cargo build --locked --profile "$profile" -p riseonly
cargo build --locked --profile "$profile" -p riseonly-web-view-helper --features helper

bash scripts/bundle-linux.sh "$profile"
usr="build/linux/$profile/usr"
desktop_file=$(ls "$usr"/share/applications/*.desktop)
bundle_id=$(basename "$desktop_file" .desktop)

appdir="build/linux/$profile/AppDir"
rm -rf "$appdir"
mkdir -p "$appdir"
cp -a "$usr" "$appdir/usr"

lib=$(ldconfig -p | awk '/libpipewire-0\.3\.so\.0 / && /x86-64/ { print $NF; exit }')
[ -n "$lib" ] || { echo "libpipewire-0.3.so.0 not found in the toolchain image" >&2; exit 1; }
mkdir -p "$appdir/usr/lib"
cp -L "$lib" "$appdir/usr/lib/"

cp "$desktop_file" "$appdir/$bundle_id.desktop"
cp "$usr/share/icons/hicolor/256x256/apps/$bundle_id.png" "$appdir/$bundle_id.png"
ln -sf "$bundle_id.png" "$appdir/.DirIcon"
cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
export LD_LIBRARY_PATH="$here/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/usr/bin/riseonly" "$@"
RUN
chmod +x "$appdir/AppRun"

mkdir -p dist/linux
out="dist/linux/$slug-$version-linux-x86_64.AppImage"
rm -f "$out"
ARCH=x86_64 appimagetool --no-appstream "$appdir" "$out"
chmod +x "$out"
ls -l "$out"
echo "target dir: $target"
