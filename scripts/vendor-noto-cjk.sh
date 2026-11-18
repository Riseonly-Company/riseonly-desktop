#!/usr/bin/env bash
set -euo pipefail

NOTO_TAG="${NOTO_TAG:-Sans2.004}"
FONT_SHA256="b76b0433203017ca80401b2ee0dd69350349871c4b19d504c34dbdd80541690a"
LICENSE_SHA256="6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR="${ROOT}/vendor/noto-cjk"
FONT="${VENDOR}/NotoSansCJK-Regular.ttc"
LICENSE="${VENDOR}/OFL.txt"
BASE="https://github.com/notofonts/noto-cjk/raw/${NOTO_TAG}"

sha() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        sha256sum "$1" | cut -d' ' -f1
    fi
}

if [ -f "${FONT}" ] && [ -f "${LICENSE}" ] && [ "$(sha "${FONT}")" = "${FONT_SHA256}" ] &&
    [ "$(sha "${LICENSE}")" = "${LICENSE_SHA256}" ] && [ "${FORCE:-0}" != "1" ]; then
    echo "Noto Sans CJK already vendored at ${VENDOR} (FORCE=1 to re-fetch)"
    exit 0
fi

mkdir -p "${VENDOR}"
curl -fsSL -o "${FONT}.part" "${BASE}/Sans/OTC/NotoSansCJK-Regular.ttc"
curl -fsSL -o "${LICENSE}.part" "${BASE}/LICENSE"
for pair in "${FONT}.part:${FONT_SHA256}" "${LICENSE}.part:${LICENSE_SHA256}"; do
    file="${pair%%:*}"
    expected="${pair##*:}"
    actual="$(sha "${file}")"
    if [ "${actual}" != "${expected}" ]; then
        echo "hash mismatch for ${file}: expected ${expected}, got ${actual}" >&2
        echo "Refusing to vendor. Update the pin deliberately after checking the release." >&2
        rm -f "${file}"
        exit 1
    fi
done
mv "${FONT}.part" "${FONT}"
mv "${LICENSE}.part" "${LICENSE}"
echo "Noto Sans CJK Regular (SIL OFL 1.1) vendored at ${VENDOR}"
