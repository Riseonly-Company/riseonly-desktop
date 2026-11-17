#!/usr/bin/env bash
set -euo pipefail

profile=${PROFILE:-fast}
slug=${SLUG:-Riseonly}
version=${VERSION:?VERSION is required}
arch=${MACOS_ARCH:-aarch64}

[ "$(uname -m)" = arm64 ] || { echo "the macOS build runs on Apple silicon only" >&2; exit 1; }

case "$arch" in
    aarch64)
        target_args=()
        ffmpeg_arch=arm64
        export MLN_PRECOMPILE=${MLN_PRECOMPILE:-1}
        ;;
    x86_64)
        target_args=(--target x86_64-apple-darwin)
        ffmpeg_arch=x86_64
        export MLN_PRECOMPILE=0
        export RISE_MACOS_TARGET=x86_64-apple-darwin
        ;;
    *) echo "MACOS_ARCH is aarch64 or x86_64, not $arch" >&2; exit 1 ;;
esac

./scripts/vendor-rlottie.sh
if [ ! -d "vendor/ffmpeg/macos-$ffmpeg_arch/lib" ]; then
    if [ "$arch" = x86_64 ]; then
        arch -x86_64 /bin/bash ./scripts/build-ffmpeg-lgpl.sh
    else
        ./scripts/build-ffmpeg-lgpl.sh
    fi
fi
python3 scripts/check-ffmpeg-license.py "vendor/ffmpeg/macos-$ffmpeg_arch"

cargo build --locked --profile "$profile" ${target_args[@]+"${target_args[@]}"} -p riseonly
bash scripts/bundle-macos.sh "$profile"

app=$(find "build/macos/$profile" -maxdepth 1 -name "*.app" -print -quit)
[ -d "$app" ] || { echo "no bundle under build/macos/$profile" >&2; exit 1; }
lipo -archs "$app/Contents/MacOS/riseonly" | grep -qw "$([ "$arch" = aarch64 ] && echo arm64 || echo x86_64)" ||
    { echo "the bundle is not $arch" >&2; exit 1; }

mkdir -p dist/macos
base="dist/macos/$slug-$version-macos-$arch"
rm -f "$base.zip" "$base.dmg"
bash scripts/ci/macos-sign.sh "$app" "$base"
ls -l dist/macos
