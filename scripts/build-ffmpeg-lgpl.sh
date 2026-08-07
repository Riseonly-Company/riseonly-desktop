#!/usr/bin/env bash
#
# Build an LGPL-only FFmpeg into vendor/ffmpeg/<os>-<arch>/.
#
# WHY THIS EXISTS AT ALL, WHEN EVERY MACHINE ALREADY HAS AN FFMPEG
#
# Homebrew's ffmpeg is configured --enable-gpl --enable-version3 with libx264
# and libx265. Linking a proprietary product against that build relicenses the
# product under the GPL. Every distribution's default ffmpeg has the same
# problem. There is no way to "use the system ffmpeg carefully"; the flags are
# baked into the shared objects we would link.
#
# So we build our own, with the licence-bearing flags off, and CI re-derives the
# licence from the artefact rather than trusting this script (see
# scripts/check-ffmpeg-license.py). A single --enable-gpl slipping in here would
# otherwise be invisible until someone read the About box.
#
# LGPL also requires dynamic linking (so a user can substitute their own
# FFmpeg) and publication of the matching source. Hence --enable-shared
# --disable-static, and the tarball is kept next to the prefix.

set -euo pipefail

FFMPEG_VERSION="${FFMPEG_VERSION:-7.1.1}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

case "$(uname -s)" in
    Darwin) OS=macos ;;
    Linux) OS=linux ;;
    MINGW* | MSYS* | CYGWIN*) OS=windows ;;
    *)
        echo "unsupported host: $(uname -s)" >&2
        exit 1
        ;;
esac
ARCH="$(uname -m)"

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

rm -rf "${SRC}"
tar -xf "${TARBALL}" -C "${WORK}"

# The decoder set is deliberately narrow. Every codec enabled here is a C parser
# fed attacker-supplied bytes off a socket; the ones we do not ship are the
# cheapest attack surface reduction available.
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
)

case "${OS}" in
    macos)
        COMMON_FLAGS+=(--enable-videotoolbox --enable-hwaccel=h264_videotoolbox,hevc_videotoolbox,vp9_videotoolbox,av1_videotoolbox)
        ;;
    linux)
        COMMON_FLAGS+=(--enable-vaapi --enable-hwaccel=h264_vaapi,hevc_vaapi,vp9_vaapi,av1_vaapi)
        ;;
    windows)
        COMMON_FLAGS+=(--enable-d3d11va --enable-hwaccel=h264_d3d11va2,hevc_d3d11va2,vp9_d3d11va2,av1_d3d11va2)
        ;;
esac

(
    cd "${SRC}"
    ./configure "${COMMON_FLAGS[@]}"
    make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
    make install
)

# The configure line is recorded next to the artefact so the licence check does
# not have to trust this script's contents at the time CI happens to run.
"${PREFIX}/lib"/../bin/ffmpeg -version 2>/dev/null | head -20 >"${PREFIX}/BUILD_INFO.txt" || true
{
    echo "ffmpeg_version=${FFMPEG_VERSION}"
    echo "source_tarball=$(basename "${TARBALL}")"
    echo "configure=${COMMON_FLAGS[*]}"
} >>"${PREFIX}/BUILD_INFO.txt"

echo "ffmpeg (LGPL) installed to ${PREFIX}"
echo "point rsmpeg at it with:"
echo "  export FFMPEG_PKG_CONFIG_PATH=${PREFIX}/lib/pkgconfig"
