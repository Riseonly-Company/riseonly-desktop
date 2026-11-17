#!/usr/bin/env bash
set -euo pipefail

[ $# -ge 2 ] || { echo "usage: $0 <cache> <command...>" >&2; exit 2; }
cache=$1
shift

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
desktop="$(cd "$here/../.." && pwd)"
workspace="$(cd "$desktop/.." && pwd)"
[ -d "$workspace/riseonly-tools" ] || { echo "riseonly-tools is not checked out beside riseonly-desktop" >&2; exit 2; }

root=${DESKTOP_CI_CACHE:-/srv/ci/desktop}
mkdir -p "$root/$cache" "$root/cargo" "$root/xwin" "$root/home"

min_free=${DESKTOP_CI_MIN_FREE_GB:-25}
free_gb() { df -Pk "$root" | awk 'NR == 2 { print int($4 / 1048576) }'; }
if [ "$(free_gb)" -lt "$min_free" ]; then
    echo "only $(free_gb) GB free, dropping other target dirs"
    for dir in "$root"/target-*; do
        [ -d "$dir" ] && [ "$dir" != "$root/$cache" ] || continue
        echo "  removing $dir"
        rm -rf "$dir"
        [ "$(free_gb)" -ge "$min_free" ] && break
    done
fi

dockerfile="$desktop/scripts/ci/toolchain/Dockerfile"
rust=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$desktop/rust-toolchain.toml")
image="riseonly-desktop-toolchain:$( (cat "$dockerfile"; echo "$rust") | sha256sum | cut -c1-12)"
if ! docker image inspect "$image" >/dev/null 2>&1; then
    docker build --quiet --build-arg "RUST_TOOLCHAIN=$rust" --tag "$image" "$(dirname "$dockerfile")" >/dev/null
fi

forward=()
for name in RISE_ENV RISE_BUILD_NUMBER RISE_UPDATE_FEED_PUBLIC_KEY PROFILE VERSION SLUG CARGO_TERM_COLOR WIN_ARCH; do
    if [ -n "${!name+x}" ]; then
        forward+=(-e "$name=${!name}")
    fi
done

exec docker run --rm --init \
    --name "desktop-ci-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-0}-$cache" \
    --user "$(id -u):$(id -g)" \
    --cpu-shares "${DESKTOP_CI_CPU_SHARES:-512}" \
    --memory "${DESKTOP_CI_MEMORY:-9g}" --memory-swap "${DESKTOP_CI_MEMORY:-9g}" \
    -v "$workspace/riseonly-desktop:/w/riseonly-desktop" \
    -v "$workspace/riseonly-tools:/w/riseonly-tools" \
    -v "$root/$cache:/target" \
    -v "$root/cargo:/cargo" \
    -v "$root/xwin:/xwin" \
    -v "$root/home:/home/ci" \
    -e HOME=/home/ci \
    -e CARGO_HOME=/cargo \
    -e CARGO_TARGET_DIR=/target \
    -e CARGO_BUILD_JOBS="$(nproc)" \
    "${forward[@]}" \
    -w /w/riseonly-desktop \
    "$image" "$@"
