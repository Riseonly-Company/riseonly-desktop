#!/usr/bin/env bash
set -euo pipefail

[ $# -ge 1 ] || { echo "usage: $0 <command...>" >&2; exit 2; }

export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:/opt/homebrew/bin:$PATH"
# why: bindgen 0.71 (rusty_ffmpeg) loses FFmpeg struct bodies with Homebrew libclang 22+, Xcode's parses them whole
xcode_libclang=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib
if [ -z "${LIBCLANG_PATH:-}" ] && [ -f "$xcode_libclang/libclang.dylib" ]; then
    export LIBCLANG_PATH="$xcode_libclang"
fi
export XWIN_ACCEPT_LICENSE=1
export XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-$HOME/.cache/riseonly-ci/xwin}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target-windows}"

case "${WIN_ARCH:-x86_64}" in
    aarch64) rustup target add aarch64-pc-windows-msvc >/dev/null ;;
    *) rustup target add x86_64-pc-windows-msvc >/dev/null ;;
esac

exec caffeinate -i /usr/bin/lockf -k /tmp/riseonly-desktop-build.lock "$@"
