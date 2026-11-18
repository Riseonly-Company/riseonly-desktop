FROM ubuntu:24.04

ARG RUST_TOOLCHAIN=1.95.0
ARG LLVM_VERSION=19
ARG CARGO_XWIN_VERSION=0.23.1
ARG APPIMAGETOOL_VERSION=1.9.1
ARG APPIMAGETOOL_SHA256_X86_64=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
ARG APPIMAGETOOL_SHA256_AARCH64=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
ARG LLVM_MINGW_VERSION=20261104
ARG LLVM_MINGW_SHA256_X86_64=919380403de87dca3c581c2053230cd298af5e891286628ba0613b7fed370926
ARG LLVM_MINGW_SHA256_AARCH64=595e95bdffabfc62559fe26e30bdfbccf277cbaed4902793555015ee47306b94

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:/opt/llvm-mingw/bin \
    XWIN_ACCEPT_LICENSE=1 \
    XWIN_ARCH=x86_64 \
    XWIN_CACHE_DIR=/xwin \
    APPIMAGE_EXTRACT_AND_RUN=1

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl gnupg git rsync python3 build-essential pkg-config cmake ninja-build \
        libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev libxcb1-dev \
        libfontconfig-dev libfreetype-dev libvulkan-dev libvulkan1 libasound2-dev libdbus-1-dev \
        libssl-dev libgbm-dev libdrm-dev libva-dev libegl-dev libgl-dev libuv1-dev libcurl4-openssl-dev \
        libwebkit2gtk-4.1-dev libgtk-3-dev libpipewire-0.3-dev libspa-0.2-dev \
        patchelf imagemagick file desktop-file-utils zip unzip xz-utils binutils squashfs-tools \
        mingw-w64 nasm \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://apt.llvm.org/llvm-snapshot.gpg.key | gpg --dearmor -o /usr/share/keyrings/llvm.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/llvm.gpg] http://apt.llvm.org/noble/ llvm-toolchain-noble-${LLVM_VERSION} main" \
        > /etc/apt/sources.list.d/llvm.list \
    && apt-get update && apt-get install -y --no-install-recommends \
        clang-${LLVM_VERSION} lld-${LLVM_VERSION} llvm-${LLVM_VERSION} libclang-${LLVM_VERSION}-dev \
    && rm -rf /var/lib/apt/lists/* \
    && for tool in clang clang++ clang-cl lld lld-link ld.lld llvm-lib llvm-ar llvm-rc llvm-ranlib llvm-dlltool; do \
        ln -sf "/usr/lib/llvm-${LLVM_VERSION}/bin/${tool}" "/usr/local/bin/${tool}"; \
    done

RUN curl -fsSL https://sh.rustup.rs -o /tmp/rustup-init.sh \
    && CARGO_HOME=/usr/local/cargo sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_TOOLCHAIN}" -c rustfmt -c clippy -t x86_64-pc-windows-msvc -t aarch64-pc-windows-msvc \
    && rm /tmp/rustup-init.sh \
    && CARGO_HOME=/usr/local/cargo cargo install cargo-xwin --version "${CARGO_XWIN_VERSION}" --locked --root /usr/local \
    && rm -rf /usr/local/cargo/registry /usr/local/cargo/git \
    && chmod -R a+rwX /usr/local/rustup

RUN arch=$(uname -m) \
    && case "$arch" in x86_64) sha="${APPIMAGETOOL_SHA256_X86_64}" ;; aarch64) sha="${APPIMAGETOOL_SHA256_AARCH64}" ;; *) exit 1 ;; esac \
    && curl -fsSL "https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-${arch}.AppImage" -o /tmp/appimagetool \
    && echo "${sha}  /tmp/appimagetool" | sha256sum -c - \
    && chmod +x /tmp/appimagetool \
    && cd /opt && /tmp/appimagetool --appimage-extract >/dev/null \
    && mv /opt/squashfs-root /opt/appimagetool && rm /tmp/appimagetool \
    && ln -s /opt/appimagetool/AppRun /usr/local/bin/appimagetool

RUN arch=$(uname -m) \
    && case "$arch" in x86_64) sha="${LLVM_MINGW_SHA256_X86_64}" ;; aarch64) sha="${LLVM_MINGW_SHA256_AARCH64}" ;; *) exit 1 ;; esac \
    && curl -fsSL "https://github.com/mstorsjo/llvm-mingw/releases/download/${LLVM_MINGW_VERSION}/llvm-mingw-${LLVM_MINGW_VERSION}-ucrt-ubuntu-22.04-${arch}.tar.xz" -o /tmp/llvm-mingw.tar.xz \
    && echo "${sha}  /tmp/llvm-mingw.tar.xz" | sha256sum -c - \
    && tar -xf /tmp/llvm-mingw.tar.xz -C /opt \
    && mv "/opt/llvm-mingw-${LLVM_MINGW_VERSION}-ucrt-ubuntu-22.04-${arch}" /opt/llvm-mingw \
    && rm /tmp/llvm-mingw.tar.xz

LABEL riseonly.desktop.toolchain="1"
