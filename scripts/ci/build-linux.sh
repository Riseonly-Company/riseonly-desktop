#!/usr/bin/env bash
set -euo pipefail

profile=${PROFILE:-fast}
slug=${SLUG:-Riseonly}
version=${VERSION:?VERSION is required}
target=${CARGO_TARGET_DIR:-target}
arch=$(uname -m)

export MLN_PRECOMPILE=${MLN_PRECOMPILE:-1}

./scripts/vendor-rlottie.sh
./scripts/vendor-noto-cjk.sh
./scripts/vendor-noto-fallback.sh
case "$arch" in
    aarch64 | arm64) ffmpeg_arch=arm64 ;;
    *) ffmpeg_arch=x86_64 ;;
esac
[ -f "vendor/ffmpeg/linux-$ffmpeg_arch/lib/pkgconfig/libavcodec.pc" ] || ./scripts/build-ffmpeg-lgpl.sh
python3 scripts/check-ffmpeg-license.py "vendor/ffmpeg/linux-$ffmpeg_arch"
export FFMPEG_PKG_CONFIG_PATH="$PWD/vendor/ffmpeg/linux-$ffmpeg_arch/lib/pkgconfig"
if [ -n "${PAYLOAD:-}" ]; then
    mkdir -p "$target/$profile"
    cp "$PAYLOAD/riseonly" "$PAYLOAD/riseonly-web-view-helper" "$target/$profile/"
    chmod +x "$target/$profile/riseonly" "$target/$profile/riseonly-web-view-helper"
else
    env -u RISE_ENV cargo build --locked --profile "$profile" -p riseonly
    env -u RISE_ENV cargo build --locked --profile "$profile" -p riseonly-web-view-helper --features helper
    rm -rf "payload/linux-$arch"
    mkdir -p "payload/linux-$arch"
    cp "$target/$profile/riseonly" "$target/$profile/riseonly-web-view-helper" "payload/linux-$arch/"
fi

bash scripts/bundle-linux.sh "$profile"
usr="build/linux/$profile/usr"
desktop_file=$(ls "$usr"/share/applications/*.desktop)
bundle_id=$(basename "$desktop_file" .desktop)

appdir="build/linux/$profile/AppDir"
rm -rf "$appdir"
mkdir -p "$appdir"
cp -a "$usr" "$appdir/usr"

exe="$appdir/usr/bin/riseonly"
dynamic_symbols=$(objdump -T "$exe")
for symbol in pidfd_spawnp pidfd_getpid; do
    if grep -q " $symbol\$" <<<"$dynamic_symbols"; then
        patchelf --clear-symbol-version "$symbol" "$exe"
    fi
done
floor=$(objdump -T "$exe" | grep -oE 'GLIBC_2\.[0-9]+' | sort -t. -k2 -n -u | tail -1)
echo "glibc floor of riseonly: $floor"
for symbol in $(objdump -T "$exe" | awk '/CURL_OPENSSL_4/ { print $NF }'); do
    patchelf --clear-symbol-version "$symbol" "$exe"
done

cp "$desktop_file" "$appdir/$bundle_id.desktop"
cp "$usr/share/icons/hicolor/256x256/apps/$bundle_id.png" "$appdir/$bundle_id.png"
ln -sf "$bundle_id.png" "$appdir/.DirIcon"
cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
libs="$here/usr/lib"
fallback="$here/usr/lib/riseonly/fallback"
case "$(uname -m)" in
    x86_64) tag="x86-64" ;;
    aarch64) tag="AArch64" ;;
    *) tag="" ;;
esac
cache=$({ ldconfig -p || /sbin/ldconfig -p || /usr/sbin/ldconfig -p; } 2>/dev/null)
for soname in libpipewire-0.3.so.0 libva.so.2 libva-drm.so.2 libdrm.so.2; do
    [ -d "$fallback/$soname" ] || continue
    printf '%s\n' "$cache" | grep -q "$soname (libc6,$tag" && continue
    libs="$libs:$fallback/$soname"
    if [ "$soname" = libva.so.2 ] && [ -z "${LIBVA_DRIVERS_PATH:-}" ]; then
        drivers=""
        for dir in /usr/lib/x86_64-linux-gnu/dri /usr/lib/aarch64-linux-gnu/dri /usr/lib64/dri /usr/lib/dri /usr/local/lib/dri; do
            [ -d "$dir" ] && drivers="${drivers:+$drivers:}$dir"
        done
        [ -n "$drivers" ] && export LIBVA_DRIVERS_PATH="$drivers"
    fi
done
export LD_LIBRARY_PATH="$libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/usr/bin/riseonly" "$@"
RUN
chmod +x "$appdir/AppRun"

mkdir -p dist/linux
out="dist/linux/$slug-$version-linux-$arch.AppImage"
rm -f "$out"
ARCH="$arch" appimagetool --no-appstream "$appdir" "$out"
chmod +x "$out"
ls -l "$out"
echo "target dir: $target"
