#!/usr/bin/env bash
set -euo pipefail

profile=${PROFILE:-fast}
slug=${SLUG:-Riseonly}
version=${VERSION:?VERSION is required}
triple=x86_64-pc-windows-msvc
target=${CARGO_TARGET_DIR:-target}

case "${RISE_ENV:-production}" in
    staging | stage) app_name="Riseonly Staging" ;;
    dev | development) app_name="Riseonly Dev" ;;
    *) app_name="Riseonly" ;;
esac

./scripts/vendor-rlottie.sh
export RC_x86_64_pc_windows_msvc="$PWD/scripts/xwin-llvm-rc.sh"
cargo xwin build --locked --profile "$profile" --target "$triple" -p riseonly

exe="$target/$triple/$profile/riseonly.exe"
[ -f "$exe" ] || { echo "missing $exe" >&2; exit 1; }
if grep -aqE '(avcodec|avformat|avutil|swresample|swscale)-[0-9]+\.dll' "$exe"; then
    echo "riseonly.exe imports FFmpeg, the windows package has no FFmpeg prefix yet" >&2
    exit 1
fi

stage="build/windows/$profile"
out_dir="$stage/$app_name"
rm -rf "$stage"
mkdir -p "$out_dir"
cp "$exe" "$out_dir/riseonly.exe"
[ -d assets ] && cp -R assets "$out_dir/assets"

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
zip_path="$PWD/dist/windows/$slug-$version-windows-x86_64.zip"
rm -f "$zip_path"
(cd "$stage" && zip -qr -9 "$zip_path" "$app_name")
ls -l "$zip_path"
