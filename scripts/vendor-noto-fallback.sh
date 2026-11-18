#!/usr/bin/env bash
set -euo pipefail

NOTO_COMMIT="${NOTO_COMMIT:-15f23a0fd72c87285f50e890af8cf64d82d588a3}"
LICENSE_SHA256="f2095b08bed08b23a6fe26112fcd679a2bee3f002eef077eb05d215ed1051bd8"
FACES=(
    "NotoSansThai:d4303fe9c63ebb72759ca8b6d2040c8ae81689f7d08d7b91c656154382b49313"
    "NotoSansDevanagari:9c7d935139ea6a1e6ad9dbac4f6d27ece1e04bca8123c8888d00a0f9df4724cd"
    "NotoSansBengali:5dceda02816fece18ea6796f474f5d3170f1d862f21b1e809cdc94b1caa4b6ec"
    "NotoSansTamil:6634d9cc97a726e670df41281dd32b167a6b9b71f2036e19671ff08fdde0c292"
    "NotoSansGeorgian:f4b229b126859725b75031dd8c051335d20c85e4eb6d525af28f3e83e471baa4"
    "NotoSansHebrew:04272f5600d0ec816d31d0df73b23aa8d3501ea359ebe820da31c11ffcf00853"
    "NotoSansArabic:bd86ca02f087d7f3c3788ba458fb6b73744c7639ed276b8d870dba6def6c40d0"
)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR="${ROOT}/vendor/noto-fallback"
LICENSE="${VENDOR}/OFL.txt"
BASE="https://github.com/notofonts/notofonts.github.io/raw/${NOTO_COMMIT}/fonts"

sha() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        sha256sum "$1" | cut -d' ' -f1
    fi
}

fetch() {
    local url="$1" target="$2" expected="$3"
    if [ -f "${target}" ] && [ "$(sha "${target}")" = "${expected}" ] && [ "${FORCE:-0}" != "1" ]; then
        return 0
    fi
    curl -fsSL -o "${target}.part" "${url}"
    local actual
    actual="$(sha "${target}.part")"
    if [ "${actual}" != "${expected}" ]; then
        echo "hash mismatch for ${target}: expected ${expected}, got ${actual}" >&2
        echo "Refusing to vendor. Update the pin deliberately after checking the release." >&2
        rm -f "${target}.part"
        exit 1
    fi
    mv "${target}.part" "${target}"
}

mkdir -p "${VENDOR}"
fetch "${BASE}/LICENSE" "${LICENSE}" "${LICENSE_SHA256}"
for pair in "${FACES[@]}"; do
    family="${pair%%:*}"
    fetch "${BASE}/${family}/unhinted/ttf/${family}-Regular.ttf" "${VENDOR}/${family}-Regular.ttf" "${pair##*:}"
done
echo "Noto Sans Thai, Devanagari, Bengali, Tamil, Georgian, Hebrew and Arabic Regular (SIL OFL 1.1) vendored at ${VENDOR}"
