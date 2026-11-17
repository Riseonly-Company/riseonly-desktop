#!/usr/bin/env bash
set -euo pipefail

profile=${PROFILE:-fast}
slug=${SLUG:-Riseonly}
version=${VERSION:?VERSION is required}

[ "$(uname -m)" = arm64 ] || { echo "the macOS build runs on Apple silicon only" >&2; exit 1; }

export MLN_PRECOMPILE=${MLN_PRECOMPILE:-1}

./scripts/vendor-rlottie.sh
if [ ! -d vendor/ffmpeg/macos-arm64/lib ]; then
    ./scripts/build-ffmpeg-lgpl.sh
fi
python3 scripts/check-ffmpeg-license.py

cargo build --locked --profile "$profile" -p riseonly
bash scripts/bundle-macos.sh "$profile"

app=$(find "build/macos/$profile" -maxdepth 1 -name "*.app" -print -quit)
[ -d "$app" ] || { echo "no bundle under build/macos/$profile" >&2; exit 1; }

mkdir -p dist/macos
base="dist/macos/$slug-$version-macos-aarch64"
rm -f "$base.zip" "$base.dmg"
bash scripts/ci/macos-sign.sh "$app" "$base"
ls -l dist/macos
