#!/usr/bin/env bash
set -euo pipefail

[ $# -eq 3 ] || { echo "usage: $0 linux|windows <arch> <on the team mac: true|false>" >&2; exit 2; }
platform=$1
arch=$2
on_mac=$3

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
desktop="$(cd "$here/../.." && pwd)"
profile=${PROFILE:-fast}
mac_cache="$HOME/.cache/riseonly-ci/desktop"

rm -rf "$desktop/dist" "$desktop/timings"

case "$platform:$on_mac" in
    windows:true)
        cd "$desktop"
        WIN_ARCH="$arch" CARGO_TARGET_DIR="$mac_cache/target-windows-$profile-$arch" \
            bash scripts/ci/mac-cross.sh bash scripts/ci/build-windows.sh
        ;;
    windows:false)
        WIN_ARCH="$arch" bash "$here/in-toolchain.sh" "target-windows-$profile" bash scripts/ci/build-windows.sh
        ;;
    linux:true)
        case "$arch" in x86_64) docker_arch=amd64 ;; *) docker_arch=arm64 ;; esac
        # why: the Docker VM of the Mac has ~11 GB beside the local backend stack, zram swap covers the LTO peak
        DESKTOP_CI_CACHE="$mac_cache" DESKTOP_CI_PLATFORM="linux/$docker_arch" DESKTOP_CI_CPU_SHARES=1024 \
            DESKTOP_CI_MEMORY="${DESKTOP_CI_MAC_MEMORY:-8g}" DESKTOP_CI_MEMORY_SWAP="${DESKTOP_CI_MAC_MEMORY_SWAP:-16g}" \
            DESKTOP_CI_JOBS="${DESKTOP_CI_MAC_JOBS:-6}" \
            caffeinate -i /usr/bin/lockf -k /tmp/riseonly-desktop-build.lock \
            bash "$here/in-toolchain.sh" "target-linux-$profile-$arch" bash scripts/ci/build-linux.sh
        ;;
    linux:false)
        bash "$here/in-toolchain.sh" "target-linux-$profile" bash scripts/ci/build-linux.sh
        ;;
    *) echo "unknown platform $platform" >&2; exit 2 ;;
esac
