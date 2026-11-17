#!/usr/bin/env bash
set -euo pipefail

profile=${PROFILE:-fast}
slug=${SLUG:-Riseonly}
version=${VERSION:?VERSION is required}
arch=${WIN_ARCH:-x86_64}
target=${CARGO_TARGET_DIR:-target}

case "$arch" in
    x86_64) triple=x86_64-pc-windows-msvc ffmpeg_arch=x86_64 cross=x86_64-w64-mingw32- ;;
    aarch64) triple=aarch64-pc-windows-msvc ffmpeg_arch=arm64 cross=aarch64-w64-mingw32- ;;
    *) echo "WIN_ARCH is x86_64 or aarch64, not $arch" >&2; exit 1 ;;
esac
triple_env=${triple//-/_}
export XWIN_ARCH="$arch"
export XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-/xwin}/$arch"

case "${RISE_ENV:-production}" in
    staging | stage) app_name="Riseonly Staging" environment=staging ;;
    dev | development) app_name="Riseonly Dev" environment=dev ;;
    *) app_name="Riseonly" environment=production ;;
esac

./scripts/vendor-rlottie.sh

ffmpeg="vendor/ffmpeg/windows-$ffmpeg_arch"
ffmpeg_cache="$target/ffmpeg-windows-$ffmpeg_arch-$(sha256sum scripts/build-ffmpeg-lgpl.sh | cut -c1-12)"
if [ -d "$ffmpeg_cache/lib" ]; then
    rm -rf "$ffmpeg"
    mkdir -p vendor/ffmpeg
    cp -a "$ffmpeg_cache" "$ffmpeg"
else
    FFMPEG_TARGET_OS=windows FFMPEG_TARGET_ARCH="$ffmpeg_arch" FFMPEG_CROSS_PREFIX="$cross" \
        ./scripts/build-ffmpeg-lgpl.sh
    rm -rf "$ffmpeg_cache"
    cp -a "$ffmpeg" "$ffmpeg_cache"
fi
python3 scripts/check-ffmpeg-license.py "$ffmpeg"
export FFMPEG_PKG_CONFIG_PATH="$PWD/$ffmpeg/lib/pkgconfig"
export "PKG_CONFIG_ALLOW_CROSS_$triple_env=1"
export "PKG_CONFIG_LIBDIR_$triple_env=$FFMPEG_PKG_CONFIG_PATH"

exe="$target/$triple/$profile/riseonly.exe"
if [ -n "${PAYLOAD:-}" ]; then
    mkdir -p "$(dirname "$exe")"
    cp "$PAYLOAD/riseonly.exe" "$exe"
else
    export "RC_$triple_env=$PWD/scripts/xwin-llvm-rc.sh"
    env -u RISE_ENV cargo xwin build --locked --profile "$profile" --target "$triple" -p riseonly
    rm -rf "payload/windows-$arch"
    mkdir -p "payload/windows-$arch"
    cp "$exe" "payload/windows-$arch/riseonly.exe"
fi
[ -f "$exe" ] || { echo "missing $exe" >&2; exit 1; }

stage="build/windows/$profile"
out_dir="$stage/$app_name"
rm -rf "$stage"
mkdir -p "$out_dir"
cp "$exe" "$out_dir/riseonly.exe"
printf '%s\n' "$environment" > "$out_dir/environment"
[ -d assets ] && cp -R assets "$out_dir/assets"

for dll in "$ffmpeg"/bin/*.dll; do
    case "$(basename "$dll")" in
        avcodec-*.dll | avdevice-*.dll | avfilter-*.dll | avformat-*.dll | avutil-*.dll | swresample-*.dll | swscale-*.dll)
            cp "$dll" "$out_dir/" ;;
    esac
done
tarball=$(sed -n 's/^source_tarball=//p' "$ffmpeg/BUILD_INFO.txt")
[ -n "$tarball" ] && [ -f "vendor/ffmpeg/$tarball" ] || { echo "FFmpeg source tarball not found; LGPL needs it shipped" >&2; exit 1; }
mkdir -p "$out_dir/licenses/ffmpeg"
cp "vendor/ffmpeg/$tarball" "$ffmpeg/BUILD_INFO.txt" "$out_dir/licenses/ffmpeg/"

for tree in vendor/rlottie/rlottie-*/; do
    [ -d "$tree" ] || continue
    tree="${tree%/}"
    report="$(python3 scripts/check-rlottie-license.py "$tree")" || { printf '%s\n' "$report" >&2; exit 1; }
    licenses="$out_dir/licenses/$(basename "$tree")"
    mkdir -p "$licenses"
    cp "$tree"/licenses/COPYING.* "$licenses/"
    printf '%s\n' "$report" | sed -n 's/^  MPL-2.0, source must ship with the release: //p' |
        while read -r file; do
            mkdir -p "$licenses/source/$(dirname "$file")"
            cp "$tree/$file" "$licenses/source/$file"
        done
done

mkdir -p dist/windows
zip_path="$PWD/dist/windows/$slug-$version-windows-$arch.zip"
rm -f "$zip_path"
(cd "$stage" && zip -qr -9 "$zip_path" "$app_name")
ls -l "$zip_path"
