#!/usr/bin/env bash
set -euo pipefail

RLOTTIE_VERSION="${RLOTTIE_VERSION:-0.2}"
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

if [ ! -d "${SRC}" ]; then
    echo "unexpected tarball layout; expected ${SRC}" >&2
    exit 1
fi

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
