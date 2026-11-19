#!/bin/bash
# Cross-builds the MapLibre Native core for x86_64-pc-windows-msvc on macOS (or Linux) with
# clang-cl, lld-link and the cargo-xwin sysroot: static CRT (/MT, as libwebrtc), Vulkan renderer,
# every dependency static, one amalgam .lib for MLN_CORE_LIBRARY_PATH.
#
#   WIN_ARCH=x86_64|aarch64 build-maplibre-core-windows.sh <maplibre-native checkout> <work dir>
#
# A missing checkout is cloned at the MLN_COMMIT of vendor/maplibre_native-0.10.0/build.rs. The
# cmake toolchain comes from cargo-xwin (XWIN_CACHE_DIR, run `cargo xwin env --target <triple>`
# once first). Output: <work dir>/out/maplibre-native-core-amalgam-windows-<x64|arm64>-vulkan.lib
# and the headers tarball of the same release beside it.
#
# The released maplibre-native-core-windows-x64-*.lib cannot be used: it is built with /MD
# (RuntimeLibrary=MD_DynamicRelease) and leaves curl, libuv, png, jpeg, webp, sqlite and the
# vendored mbgl libraries unresolved.
set -eu
MLN=${1:?maplibre-native checkout}
W=${2:?work dir}
P=$W/prefix
ROOT=$(cd "$(dirname "$0")/.." && pwd)
case "${WIN_ARCH:-x86_64}" in
    x86_64) TRIPLE=x86_64-pc-windows-msvc LIB_ARCH=x64 ;;
    aarch64) TRIPLE=aarch64-pc-windows-msvc LIB_ARCH=arm64 ;;
    *) echo "WIN_ARCH is x86_64 or aarch64" >&2; exit 2 ;;
esac
XC=${XWIN_CACHE_DIR:-$HOME/.cache/cargo-xwin}
mkdir -p "$W/src" "$W/build" "$W/out" "$P/lib" "$P/include"
export PATH=$XC:/opt/homebrew/opt/llvm/bin:/opt/homebrew/bin:$PATH
TC=$XC/cmake/clang-cl/$TRIPLE-toolchain.cmake
X=$XC/xwin
[ -f "$TC" ] || { echo "no cargo-xwin cmake toolchain at $TC: run cargo xwin env --target $TRIPLE" >&2; exit 2; }
COMMIT=$(sed -n 's/^const MLN_COMMIT: &str = "\(.*\)";/\1/p' "$ROOT/vendor/maplibre_native-0.10.0/build.rs")
if [ ! -d "$MLN/.git" ]; then
    mkdir -p "$MLN"
    git -C "$MLN" init -q
    git -C "$MLN" remote add origin https://github.com/maplibre/maplibre-native.git
    git -C "$MLN" fetch -q --depth 1 origin "${COMMIT#core-}"
    git -C "$MLN" checkout -q FETCH_HEAD
    git -C "$MLN" submodule update -q --init --recursive --depth 1
fi

cat > "$W/clang-cl-wrap" <<'WRAP'
#!/bin/bash
args=(); done_flags=0
for a in "$@"; do
  if [ "$a" = "--" ] && [ $done_flags = 0 ]; then args+=("-Wno-error" "-Wno-everything" "--"); done_flags=1; else args+=("$a"); fi
done
[ $done_flags = 0 ] && args+=("-Wno-error")
exec clang-cl "${args[@]}"
WRAP
chmod +x "$W/clang-cl-wrap"

fetch() { [ -d "$W/src/$2" ] || (cd "$W/src" && curl -sSL "$1" | tar xz); }
fetch https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz zlib-1.3.1
fetch https://download.sourceforge.net/libpng/libpng-1.6.44.tar.gz libpng-1.6.44
fetch https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/3.0.4/libjpeg-turbo-3.0.4.tar.gz libjpeg-turbo-3.0.4
fetch https://storage.googleapis.com/downloads.webmproject.org/releases/webp/libwebp-1.4.0.tar.gz libwebp-1.4.0
fetch https://github.com/libuv/libuv/archive/refs/tags/v1.49.2.tar.gz libuv-1.49.2
fetch https://curl.se/download/curl-8.10.1.tar.gz curl-8.10.1

common=(-G Ninja -DCMAKE_TOOLCHAIN_FILE=$TC -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$P -DCMAKE_PREFIX_PATH=$P
  -DCMAKE_POLICY_DEFAULT_CMP0091=NEW -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded
  "-DCMAKE_C_FLAGS_RELEASE=/MT /O2 /DNDEBUG" "-DCMAKE_CXX_FLAGS_RELEASE=/MT /O2 /DNDEBUG"
  -DBUILD_SHARED_LIBS=OFF -DCMAKE_POLICY_VERSION_MINIMUM=3.5)
dep() {
  name=$1; src=$2; shift 2
  rm -rf "$W/build/$name"
  cmake -S "$W/src/$src" -B "$W/build/$name" "${common[@]}" "$@" > "$W/build-$name.log" 2>&1
  cmake --build "$W/build/$name" -j4 >> "$W/build-$name.log" 2>&1
  cmake --install "$W/build/$name" >> "$W/build-$name.log" 2>&1
}

rm -rf "$W/build/zlib"
cmake -S "$W/src/zlib-1.3.1" -B "$W/build/zlib" "${common[@]}" -DZLIB_BUILD_EXAMPLES=OFF > "$W/build-zlib.log" 2>&1
cmake --build "$W/build/zlib" -j4 --target zlibstatic >> "$W/build-zlib.log" 2>&1
cp "$W/build/zlib/zlibstatic.lib" "$P/lib/zlib.lib"
cp "$W/build/zlib/zconf.h" "$W/src/zlib-1.3.1/zlib.h" "$P/include/"

rm -rf "$W/build/png" && mkdir -p "$W/build/png"
cp "$W/src/libpng-1.6.44/scripts/pnglibconf.h.prebuilt" "$W/build/png/pnglibconf.h"
for f in png pngerror pngget pngmem pngpread pngread pngrio pngrtran pngrutil pngset pngtrans pngwio pngwrite pngwtran pngwutil; do
  clang-cl --target=$TRIPLE /nologo /MT /O2 /DNDEBUG -DPNG_INTEL_SSE_OPT=0 -DPNG_ARM_NEON_OPT=0 \
    /imsvc "$X/crt/include" /imsvc "$X/sdk/include/ucrt" /imsvc "$X/sdk/include/um" /imsvc "$X/sdk/include/shared" \
    -I"$W/build/png" -I"$P/include" /Fo"$W/build/png/$f.obj" -c -- "$W/src/libpng-1.6.44/$f.c"
done
llvm-lib /nologo /OUT:"$P/lib/png16.lib" "$W"/build/png/*.obj
cp "$W/src/libpng-1.6.44/png.h" "$W/src/libpng-1.6.44/pngconf.h" "$W/build/png/pnglibconf.h" "$P/include/"

dep jpeg libjpeg-turbo-3.0.4 -DENABLE_SHARED=OFF -DENABLE_STATIC=ON -DWITH_TURBOJPEG=OFF -DWITH_SIMD=OFF
# libwebrtc carries Chromium's libjpeg-turbo: its functions are chromium_-prefixed but these two
# tables are not, and lld-link stops on the duplicates.
llvm-objcopy --redefine-sym jpeg_natural_order=rise_jpeg_natural_order \
  --redefine-sym jpeg_nbits_table=rise_jpeg_nbits_table "$P/lib/jpeg-static.lib"
# why: libwebp probes neon with -mfpu=neon, which clang-cl rejects for aarch64-pc-windows-msvc; arm64 has neon anyway
webp_simd=()
[ "$LIB_ARCH" = arm64 ] && webp_simd=(-DWEBP_ENABLE_SIMD=OFF)
dep webp libwebp-1.4.0 ${webp_simd[@]+"${webp_simd[@]}"} -DWEBP_BUILD_ANIM_UTILS=OFF -DWEBP_BUILD_CWEBP=OFF -DWEBP_BUILD_DWEBP=OFF -DWEBP_BUILD_GIF2WEBP=OFF \
  -DWEBP_BUILD_IMG2WEBP=OFF -DWEBP_BUILD_VWEBP=OFF -DWEBP_BUILD_WEBPINFO=OFF -DWEBP_BUILD_WEBPMUX=OFF -DWEBP_BUILD_EXTRAS=OFF
dep uv libuv-1.49.2 -DLIBUV_BUILD_SHARED=OFF -DLIBUV_BUILD_TESTS=OFF -DLIBUV_BUILD_BENCH=OFF
dep curl curl-8.10.1 -DCURL_USE_SCHANNEL=ON -DCURL_USE_LIBPSL=OFF -DUSE_LIBIDN2=OFF -DCURL_ZLIB=ON -DBUILD_CURL_EXE=OFF \
  -DCURL_DISABLE_LDAP=ON -DCURL_USE_LIBSSH2=OFF -DUSE_NGHTTP2=OFF -DCURL_BROTLI=OFF -DCURL_ZSTD=OFF -DBUILD_TESTING=OFF \
  -DBUILD_LIBCURL_DOCS=OFF -DBUILD_MISC_DOCS=OFF -DENABLE_CURL_MANUAL=OFF -DZLIB_ROOT=$P

B=$W/build/mbgl
cmake -S "$MLN" -B "$B" "${common[@]}" -DCMAKE_C_COMPILER="$W/clang-cl-wrap" -DCMAKE_CXX_COMPILER="$W/clang-cl-wrap" \
  -DMLN_WITH_VULKAN=ON -DMLN_WITH_OPENGL=OFF -DMLN_WITH_EGL=OFF -DMLN_WITH_GLFW=OFF -DMLN_WITH_WERROR=OFF \
  -DMLN_USE_BUILTIN_ICU=ON -DMLN_WITH_NODE=OFF > "$W/build-mbgl-configure.log" 2>&1
cmake --build "$B" --target mbgl-core -j5 > "$W/build-mbgl.log" 2>&1

llvm-lib /nologo /OUT:"$W/out/maplibre-native-core-amalgam-windows-$LIB_ARCH-vulkan.lib" \
  "$B/mbgl-core.lib" "$B/mbgl-harfbuzz.lib" "$B/mbgl-vendor-sqlite.lib" "$B/mbgl-vendor-nunicode.lib" \
  "$B/mbgl-vendor-parsedate.lib" "$B/mbgl-freetype.lib" "$B/mbgl-vendor-csscolorparser.lib" "$B/mbgl-vendor-icu.lib" \
  "$B/vendor/maplibre-tile-spec/cpp/mlt-cpp.lib" "$B/vendor/maplibre-tile-spec/cpp/fastpfor-lib.lib" \
  "$B/vendor/glslang/SPIRV/SPIRV.lib" "$B/vendor/glslang/glslang/MachineIndependent.lib" \
  "$B/vendor/glslang/glslang/GenericCodeGen.lib" "$B/vendor/glslang/glslang/glslang-default-resource-limits.lib" \
  "$B/vendor/glslang/glslang/glslang.lib" "$B/vendor/glslang/glslang/OSDependent/Windows/OSDependent.lib" \
  "$P/lib/png16.lib" "$P/lib/jpeg-static.lib" "$P/lib/libwebp.lib" "$P/lib/libsharpyuv.lib" "$P/lib/libuv.lib" \
  "$P/lib/libcurl.lib" "$P/lib/zlib.lib"
curl -sSL -o "$W/out/maplibre-native-headers.tar.gz" \
  "https://github.com/maplibre/maplibre-native/releases/download/$COMMIT/maplibre-native-headers.tar.gz"
echo "$W/out/maplibre-native-core-amalgam-windows-$LIB_ARCH-vulkan.lib"
