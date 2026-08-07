#!/usr/bin/env bash
#
# Vendor the rlottie source into vendor/rlottie/rlottie-<version>/.
#
# WHY VENDOR AT ALL
#
# rlottie has no crates.io presence worth linking (the `rlottie` bindings crate
# expects a system library, and no distribution ships one), no release channel,
# and no ABI promise. Telegram vendors it for the same reason. What this script
# does NOT do is take it from ../telegram: that repository is GPLv2, and
# copying anything out of it relicenses this product. Upstream Samsung/rlottie
# is MIT and is what we fetch.
#
# LICENSING, CHECKED BY scripts/check-rlottie-license.py
#
#   src/lottie, src/binding, inc   MIT
#   src/vector                     MIT + SKIA (BSD-3)
#   src/vector/freetype            FTL (BSD-style, no advertising clause)
#   src/vector/pixman              PIX (MIT)
#   src/vector/stb                 STB (public domain / MIT)
#   src/lottie/rapidjson           RPD (MIT + JSON licence for the bundled
#                                  msinttypes headers)
#
# None of that is copyleft, so a static link is fine. The check script re-derives
# this from the vendored tree rather than trusting this comment.
#
# There is no configure step and no shared object: crates/rise-media/build.rs
# compiles the sources directly with `cc`, the same way rusqlite bundles SQLite.
# That is what keeps `cargo build` from needing cmake, meson or ninja on three
# operating systems.

set -euo pipefail

RLOTTIE_VERSION="${RLOTTIE_VERSION:-0.2}"
# Pinned. A tarball that fetches a different tree than the one this hash was
# taken from is the supply-chain failure this line exists to catch.
RLOTTIE_SHA256="030ccbc270f144b4f3519fb3b86e20dd79fb48d5d55e57f950f12bab9b65216a"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR="${ROOT}/vendor/rlottie"
SRC="${VENDOR}/rlottie-${RLOTTIE_VERSION}"
TARBALL="${VENDOR}/rlottie-${RLOTTIE_VERSION}.tar.gz"
URL="https://github.com/Samsung/rlottie/archive/refs/tags/v${RLOTTIE_VERSION}.tar.gz"

if [ -f "${SRC}/inc/rlottie_capi.h" ] && [ "${FORCE:-0}" != "1" ]; then
    echo "rlottie already vendored at ${SRC} (FORCE=1 to re-fetch)"
    exit 0
fi

mkdir -p "${VENDOR}"

if [ ! -f "${TARBALL}" ]; then
    echo "fetching rlottie ${RLOTTIE_VERSION} source"
    curl -fsSL -o "${TARBALL}" "${URL}"
fi

if command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "${TARBALL}" | cut -d' ' -f1)"
else
    ACTUAL="$(sha256sum "${TARBALL}" | cut -d' ' -f1)"
fi

if [ "${ACTUAL}" != "${RLOTTIE_SHA256}" ]; then
    echo "rlottie tarball hash mismatch" >&2
    echo "  expected ${RLOTTIE_SHA256}" >&2
    echo "  actual   ${ACTUAL}" >&2
    echo "Refusing to vendor. Delete ${TARBALL} and retry, or update the pin" >&2
    echo "deliberately after reading the diff." >&2
    exit 1
fi

rm -rf "${SRC}"
tar -xzf "${TARBALL}" -C "${VENDOR}"

# GitHub's release tarballs unpack as <repo>-<version-without-v>.
if [ ! -d "${SRC}" ]; then
    echo "unexpected tarball layout; expected ${SRC}" >&2
    exit 1
fi

# The example, test and wasm trees are not compiled and only widen what a
# licence audit has to read.
rm -rf "${SRC}/example" "${SRC}/test" "${SRC}/src/wasm" "${SRC}/.Gifs"

{
    echo "rlottie_version=${RLOTTIE_VERSION}"
    echo "source_url=${URL}"
    echo "source_sha256=${RLOTTIE_SHA256}"
    echo "removed=example test src/wasm .Gifs"
    echo "built_by=crates/rise-media/build.rs (cc, static, LOTTIE_MODULE off)"
} >"${SRC}/VENDOR_INFO.txt"

echo "rlottie ${RLOTTIE_VERSION} vendored to ${SRC}"
python3 "${ROOT}/scripts/check-rlottie-license.py"
