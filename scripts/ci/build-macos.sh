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
export FFMPEG_PKG_CONFIG_PATH="$PWD/vendor/ffmpeg/macos-$ffmpeg_arch/lib/pkgconfig"
if [ "$arch" = x86_64 ]; then
    export PKG_CONFIG_ALLOW_CROSS_x86_64_apple_darwin=1
    export PKG_CONFIG_LIBDIR_x86_64_apple_darwin="$FFMPEG_PKG_CONFIG_PATH"
fi

./scripts/vendor-rlottie.sh
if [ ! -d "vendor/ffmpeg/macos-$ffmpeg_arch/lib" ]; then
    if [ "$arch" = x86_64 ]; then
        arch -x86_64 /bin/bash ./scripts/build-ffmpeg-lgpl.sh
    else
        ./scripts/build-ffmpeg-lgpl.sh
    fi
fi
python3 scripts/check-ffmpeg-license.py "vendor/ffmpeg/macos-$ffmpeg_arch"

bin="${CARGO_TARGET_DIR:-target}/${RISE_MACOS_TARGET:+$RISE_MACOS_TARGET/}$profile/riseonly"
if [ -n "${PAYLOAD:-}" ]; then
    mkdir -p "$(dirname "$bin")"
    cp "$PAYLOAD/riseonly" "$bin"
    chmod +x "$bin"
else
    cores=$(sysctl -n hw.ncpu)
    memory_gb=$(($(sysctl -n hw.memsize) / 1073741824))
    if [ -z "${CARGO_BUILD_JOBS:-}" ] && [ "$memory_gb" -lt 12 ]; then
        export CARGO_BUILD_JOBS=2
    fi
    echo "host: $cores cores, $memory_gb GB, cargo jobs ${CARGO_BUILD_JOBS:-$cores}"
    env -u RISE_ENV cargo build --locked --profile "$profile" ${target_args[@]+"${target_args[@]}"} -p riseonly
    echo "swap after the build: $(sysctl -n vm.swapusage)"
    rm -rf "payload/macos-$arch"
    mkdir -p "payload/macos-$arch"
    cp "$bin" "payload/macos-$arch/riseonly"
fi
app=$(bash scripts/bundle-macos.sh "$profile" | tee /dev/stderr | sed -n 's/^bundled: \(.*\)  (.*)$/\1/p')
[ -d "$app" ] || { echo "no bundle under build/macos/$profile" >&2; exit 1; }
lipo -archs "$app/Contents/MacOS/riseonly" | grep -qw "$([ "$arch" = aarch64 ] && echo arm64 || echo x86_64)" ||
    { echo "the bundle is not $arch" >&2; exit 1; }

mkdir -p dist/macos
base="dist/macos/$slug-$version-macos-$arch"
rm -f "$base.zip" "$base.dmg"
bash scripts/ci/macos-sign.sh "$app" "$base"
ls -l dist/macos
