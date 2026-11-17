#!/usr/bin/env bash
set -euo pipefail

FFMPEG_VERSION="${FFMPEG_VERSION:-7.1.1}"
FFMPEG_SHA256="${FFMPEG_SHA256:-733984395e0dbbe5c046abda2dc49a5544e7e0e1e2366bba849222ae9e3a03b1}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

case "$(uname -s)" in
    Darwin) HOST_OS=macos ;;
    Linux) HOST_OS=linux ;;
    MINGW* | MSYS* | CYGWIN*) HOST_OS=windows ;;
    *)
        echo "unsupported host: $(uname -s)" >&2
        exit 1
        ;;
esac

OS="${FFMPEG_TARGET_OS:-${HOST_OS}}"
ARCH="${FFMPEG_TARGET_ARCH:-$(uname -m)}"
case "${ARCH}" in
    aarch64 | arm64) ARCH=arm64 ;;
    amd64 | x86_64) ARCH=x86_64 ;;
esac
case "${OS}" in
    macos | linux | windows) ;;
    *)
        echo "unsupported target os: ${OS}" >&2
        exit 1
        ;;
esac

VENDOR="${ROOT}/vendor/ffmpeg"
PREFIX="${VENDOR}/${OS}-${ARCH}"
WORK="${VENDOR}/build"
TARBALL="${VENDOR}/ffmpeg-${FFMPEG_VERSION}.tar.xz"
SRC="${WORK}/ffmpeg-${FFMPEG_VERSION}"

if [ -f "${PREFIX}/lib/pkgconfig/libavcodec.pc" ] && [ "${FORCE:-0}" != "1" ]; then
    echo "ffmpeg already built at ${PREFIX} (FORCE=1 to rebuild)"
    exit 0
fi

mkdir -p "${VENDOR}" "${WORK}"

if [ ! -f "${TARBALL}" ]; then
    echo "fetching ffmpeg ${FFMPEG_VERSION} source"
    curl -fsSL -o "${TARBALL}" \
        "https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz"
fi

if command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "${TARBALL}" | cut -d' ' -f1)"
else
    ACTUAL="$(sha256sum "${TARBALL}" | cut -d' ' -f1)"
fi

if [ "${ACTUAL}" != "${FFMPEG_SHA256}" ]; then
    echo "ffmpeg tarball hash mismatch" >&2
    echo "  expected ${FFMPEG_SHA256}" >&2
    echo "  actual   ${ACTUAL}" >&2
    echo "Refusing to build. Delete ${TARBALL} and retry, or update the pin" >&2
    echo "deliberately after reading the diff." >&2
    exit 1
fi

rm -rf "${SRC}" "${PREFIX}"
tar -xf "${TARBALL}" -C "${WORK}"

COMMON_FLAGS=(
    --prefix="${PREFIX}"
    --disable-gpl
    --disable-nonfree
    --disable-version3
    --enable-shared
    --disable-static
    --disable-programs
    --disable-doc
    --disable-debug
    --disable-everything
    --disable-encoders
    --disable-muxers
    --disable-filters
    --disable-devices
    --disable-network
    --enable-decoder=h264,hevc,vp8,vp9,av1,mjpeg,aac,mp3,opus,vorbis,flac,pcm_s16le
    --enable-parser=h264,hevc,vp8,vp9,av1,mjpeg,aac,mpegaudio,opus,vorbis,flac
    --enable-demuxer=mov,matroska,mp3,ogg,flac,wav,aac
    --enable-protocol=file,pipe
    --enable-swscale
    --enable-swresample
    --disable-autodetect
    --enable-zlib
)

if [ "${OS}" = macos ]; then
    COMMON_FLAGS+=(--install-name-dir=@rpath)
fi

if [ "${OS}" != "${HOST_OS}" ] || [ -n "${FFMPEG_CROSS_PREFIX:-}" ]; then
    : "${FFMPEG_CROSS_PREFIX:?cross build to ${OS}-${ARCH} needs FFMPEG_CROSS_PREFIX (toolchain prefix, e.g. x86_64-w64-mingw32-)}"
    case "${OS}" in
        macos) TARGET_OS_FLAG=darwin ;;
        windows) TARGET_OS_FLAG=mingw32 ;;
        *) TARGET_OS_FLAG=linux ;;
    esac
    case "${ARCH}" in
        arm64) TARGET_ARCH_FLAG=aarch64 ;;
        *) TARGET_ARCH_FLAG="${ARCH}" ;;
    esac
    COMMON_FLAGS+=(
        --enable-cross-compile
        --target-os="${TARGET_OS_FLAG}"
        --arch="${TARGET_ARCH_FLAG}"
        --cross-prefix="${FFMPEG_CROSS_PREFIX}"
    )
fi

case "${OS}" in
    macos)
        COMMON_FLAGS+=(--enable-videotoolbox --enable-hwaccel=h264_videotoolbox,hevc_videotoolbox,vp9_videotoolbox,av1_videotoolbox)
        ;;
    linux)
        COMMON_FLAGS+=(--enable-vaapi --enable-hwaccel=h264_vaapi,hevc_vaapi,vp9_vaapi,av1_vaapi)
        COMMON_FLAGS+=(--enable-libdrm)
        COMMON_FLAGS+=(--enable-encoder=aac,h264_vaapi --enable-muxer=mp4,ipod)
        COMMON_FLAGS+=(--enable-filter=buffer,abuffer,buffersink,abuffersink,trim,atrim,setpts,asetpts,fps,scale,format,aformat,hflip,vflip,transpose,rotate,overlay,color,amix,adelay,volume,anull,null,hwupload,aresample,atempo,normalize,hue,curves,colortemperature,vignette)
        ;;
    windows)
        COMMON_FLAGS+=(--enable-d3d11va --enable-hwaccel=h264_d3d11va2,hevc_d3d11va2,vp9_d3d11va2,av1_d3d11va2)
        COMMON_FLAGS+=(--enable-mediafoundation --enable-encoder=aac,h264_mf --enable-muxer=mp4,ipod)
        COMMON_FLAGS+=(--enable-filter=buffer,abuffer,buffersink,abuffersink,trim,atrim,setpts,asetpts,fps,scale,format,aformat,hflip,vflip,transpose,rotate,overlay,color,amix,adelay,volume,anull,null,aresample,normalize,hue,curves,colortemperature,vignette)
        ;;
esac

(
    cd "${SRC}"
    ./configure "${COMMON_FLAGS[@]}"
    make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
    make install
)

for pc in "${PREFIX}/lib/pkgconfig"/*.pc; do
    sed -e 's|^prefix=.*|prefix=${pcfiledir}/../..|' \
        -e 's|^exec_prefix=.*|exec_prefix=${prefix}|' \
        -e 's|^libdir=.*|libdir=${prefix}/lib|' \
        -e 's|^includedir=.*|includedir=${prefix}/include|' \
        "${pc}" >"${pc}.tmp"
    mv "${pc}.tmp" "${pc}"
done

if [ "${OS}" = windows ]; then
    for import_lib in "${PREFIX}/bin"/*.lib; do
        [ -e "${import_lib}" ] || continue
        cp "${import_lib}" "${PREFIX}/lib/"
    done
fi

{
    echo "ffmpeg_version=${FFMPEG_VERSION}"
    echo "source_tarball=$(basename "${TARBALL}")"
    echo "source_sha256=${FFMPEG_SHA256}"
    printf 'configure='
    printf '%q ' "${COMMON_FLAGS[@]}"
    echo
} >"${PREFIX}/BUILD_INFO.txt"

echo "ffmpeg (LGPL) installed to ${PREFIX}"
echo "point rsmpeg at it with:"
echo "  export FFMPEG_PKG_CONFIG_PATH=${PREFIX}/lib/pkgconfig"
