# Dependencies

The reference keeps external dependencies to a minimum and so do we. Every crate
below has a reason and an exit. Adding one without a row here is a review reject.

A new crate, a new dependency between crates or a new external crate gets a row
here and a pinned version.

| Crate | Why | If it dies |
|---|---|---|
| `gpui`, `gpui_platform` | The UI toolkit. Apache-2.0, three OSes in production via Zed. | Nothing equivalent exists; this is the project's central bet. Pinned by exact rev. |
| `tokio` | Async runtime. Forced anyway by the LiveKit SDK later. | `smol`, but the media stack would have to follow. |
| `rusqlite` | RiseStore is SQLite, same as the reference. Bundled build, no system dependency. | `libsql`. |
| `aes-gcm` | Value encryption, matching the reference's sealed-value scheme. | `ring`. |
| `keyring` | Keychain / Credential Manager / Secret Service behind one API. | Per-OS code in `rise-platform`; the APIs are stable even if the wrapper is not. |
| `serde`, `serde_json` | Wire encoding. The gateway speaks JSON. | None needed. |
| `reqwest` | HTTP for uploads and media. rustls, no OpenSSL. | `hyper` directly. |
| `tokio-tungstenite` | The one logical socket. | `fastwebsockets`. |
| `rustls` =0.23 line, `rustls-platform-verifier` 0.7.0 | The trust decision of the OS (SecTrust, CryptoAPI, the distribution store) for reqwest (`use_preconfigured_tls`), including the remote poster of `rise-media` under its feature `ffmpeg` (P256), and tokio-tungstenite (`Connector::Rustls`), as URLSession has it; built in one place, `core/engine_bridge/network_transport_config.rs` (X24). The `socks` feature of reqwest serves a SOCKS system proxy. | webpki roots, which do not see corporate certificates. |
| `parking_lot` | Cheaper mutexes than std for hot paths; also `rise-core` for the `RiseAppEvents` observer registry (`NotificationCenter` twin), so the kit and the app share one center (A1-core-002). | std. |
| `smallvec` | Avoids allocating for the small collections the shell holds per frame. | `Vec`. |
| `tracing` | Structured logging, compiled out in release paths that need it. | `log`. |
| `directories` | Per-OS data directory, outside cloud-synced folders. | Hand-rolled per OS. |
| `base64` | Encodes secret bytes for the OS credential stores, which disagree about bytes vs strings (Secret Service is text-oriented). One encoding beats three byte paths. | Hand-rolled; it is 40 lines. |
| `fs4` | Advisory file lock on the account database. A desktop app can be launched twice; an advisory lock is released by the OS when the process dies, which a lock file is not. | Per-OS `flock`/`LockFileEx` in `rise-platform`. |
| `anyhow`, `thiserror` | Error plumbing: typed per subsystem, boxed at boundaries. | std. |
| `sha2` | Verifying a downloaded update against the hash the manifest declares, and hashing an account id before it becomes a credential-store entry name (that name is visible in Keychain Access and `secret-tool search`). An update you cannot verify is worse than no update, so this is not optional. | `ring`, or the `Security.framework`/CNG digests per OS. |
| `ring` =0.17.14 (rise-platform) | Checking the ed25519 signature of the update feed before the app trusts it. The same release rustls already links, so nothing new is built. | `ed25519-dalek`. |
| `unicode-segmentation`, `unicode-normalization`, `regex` | Foundation text twins in `rise-core::foundation` (COMMON-TWINS 1-5, 7): Swift `String.count`, `prefix` and `Character` are extended grapheme clusters, `.diacriticInsensitive` and `==` need canonical decomposition, `NSRegularExpression` patterns run 1 to 1; the regex tables also give `Character` its General Category and emoji properties, which std lacks. `unicode-segmentation` and `regex` are already in the graph through gpui; `unicode-normalization` is new (needs only `tinyvec`, also already present). | Grapheme and normalization tables generated from the UCD in a build step; `fancy-regex` or ICU bindings for patterns. |
| `futures` | Stream and future combinators (`select`, `join`, `StreamExt`) used by the engine wire, the image cache and the media HTTP path; gpui already pulls it. | Hand-written poll loops over `std::future`. |
| `tokio-util` | `CancellationToken` and codecs on top of `tokio`, for cancellation that mirrors Swift `Task` cancellation. | Own token over `tokio::sync::watch`. |
| `tracing-subscriber` | The one subscriber the binary installs for `tracing` (env filter). Binary only, never a library crate. | A tiny hand-written subscriber; `env_logger` with `log`. |
| `rand` | Random bytes for nonces and identifiers (RiseStore cipher, `foundation_uuid`). | `getrandom` directly, which is what `rand` uses underneath. |
| `x25519-dalek` | X25519 key agreement for call end-to-end keys, the twin of CryptoKit `Curve25519.KeyAgreement` (`rise_rtc_call_crypto`). BSD-3-Clause, `static_secrets` feature. HKDF runs on `rise_core` HmacSha256 (RFC 5869). | `curve25519-dalek` scalar multiplication directly. |
| `image` (`rise-media`, no default features) | Frame and delay types for rasterised Lottie and decoded raster images handed to gpui. | gpui's own `RenderImage` buffers plus hand-rolled frame structs. |
| `qrcode` 0.14.1 (`rise-ui`, no default features) | `RiseQRCode` draws the code itself with error correction H (the reference uses CoreImage `CIQRCodeGenerator` with `inputCorrectionLevel` H); the encoder returns the module matrix and `rise-ui` paints it, so the `image`/`svg` features stay off. No dependencies of its own. MIT/Apache-2.0 (MAP1 A2-core-ui-037). | The ISO 18004 encoder by hand (Reed-Solomon over GF(256), masks, version table), a few hundred lines. |
| `rqrr` 0.11.0 (`riseonly`, no default features) | The QR scanner page reads codes from camera frames and from picked pictures, which the reference does with AVFoundation `AVMetadataObject.ObjectType.qr` and `CIDetector`; gpui and the OSes give no cross-platform detector. Fed greyscale through `PreparedImage::prepare_from_greyscale`, so the `img` feature stays off. Pulls `g2p` 1.2.2 (with `g2gen`, `g2poly`) and `lru` 0.18.5; `hashbrown` 0.17.1 is already in the graph. (MIT OR Apache-2.0) AND ISC (MAP1 A2-core-ui-038). | `bardecoder`, or the per-OS detectors (Vision `VNDetectBarcodesRequest`, ZXing-C++ on Linux and Windows). |
| `ttf-parser` 0.25.1 (`riseonly`, `std` only) | The resume PDF embeds the bundled Inter bytes and draws Identity-H glyph ids; when the shaper resolved a host Inter of another release under the same family name (cosmic-text on Linux loads system faces first; a user-installed Inter on macOS or Windows), `ResumePdfTypesetter` reads each id from the embedded file's `cmap` instead, keeping the shaper's positions. Already in the lock through `fontdb` and `usvg`, no new crates. MIT OR Apache-2.0 (P186). | Register the bundled faces under a private family name so no host face can win it, at the cost of a renamed `name` table in every shipped file. |

## Native platform seams

These four are target-gated, so a macOS build never compiles the other two, and
none of them is reachable from `rise-engine`. They are used only from the
`macos.rs`, `linux.rs`, `windows.rs` branches of a seam. Each exists because gpui has no
equivalent, not because it was more convenient.

| Crate | Target | Why | If it dies |
|---|---|---|---|
| `objc2`, `objc2-foundation`, `objc2-app-kit` | macOS | The tray, the Dock badge, launch-at-login and the macOS version probe are AppKit and ServiceManagement, and gpui exposes none of them. Pinned to the same 0.6/0.3 family gpui already links, so there is no second ObjC runtime in the binary. | Nothing else is viable for AppKit from Rust; the fallback is hand-written `msg_send` against the ObjC runtime, which is what `objc2` already is. |
| `core-video` 0.5.2, `core-foundation` 0.10 (rise-media) | macOS | VideoToolbox decodes into a CVPixelBuffer and gpui draws it only through `gpui::surface(CVPixelBuffer)`; building that type from FFmpeg's raw `CVPixelBufferRef` needs `TCFType::wrap_under_get_rule` from core-foundation. Same versions gpui 6b2aa1c links, so no second copy ships (P27). | Frames fall back to `TextureImport::CpuUpload`; the presentation seam reports the platform surface as unavailable. |
| `raw-window-handle` | macOS, Windows, Linux | Reads the public AppKit `NSView` handle from `gpui::Window` so `rise-platform` can reach the owning `NSWindow` and GPUI's Metal layer and give both the same system corner; on Windows the `HWND` the share sheet is opened for (P34). The version matches GPUI's own resolved 0.6 trait. MIT/Apache-2.0. | Add an equivalent native-view accessor to GPUI or maintain the one-line accessor in the pinned GPUI fork. |
| `windows` | Windows | `Shell_NotifyIconW` for the tray and the `HKCU\...\Run` value for autostart. Microsoft's own bindings, generated from the platform metadata. | `winapi`, which is unmaintained, or hand-declared `extern "system"` items. |
| `windows-collections` 0.3 | Windows | `IIterable<IStorageItem>` from a `Vec` for `DataPackage.SetStorageItems` (system_share, P34). The `windows` crate uses this crate of its own family (windows-rs, Microsoft) in its signatures without re-exporting it; the same 0.3.2 the `windows` 0.62.2 in Cargo.lock already pulls, so nothing new ships. MIT/Apache-2.0. | Implement `IIterable` with `windows::core::implement`, which still needs the trait from this crate, or share links and text only. |
| `windows-core` 0.62 | Windows | `#[implement(IMFMediaEngineNotify)]` for the Media Engine event callback of the clip player (av_player, P105): the `windows` macro expands to paths of this crate, so it must be a direct dependency. The same 0.62.2 the `windows` 0.62.2 in Cargo.lock already pulls, so nothing new ships. MIT/Apache-2.0. | Hand-written COM vtable for the callback (unsafe, more code), or poll the engine's state instead of events. |
| `webview2-com` =0.39.1 | Windows | The bot mini app page (`BotMiniAppScreen`, `WKWebView` on iOS) as a WebView2 child window of the gpui `HWND`: `ICoreWebView2*` COM interfaces, `CreateCoreWebView2EnvironmentWithOptions` and the completed-handler and event-handler callback types (web_view windows.rs, P109, X28). The `windows` crate has no WebView2 bindings. 0.39 is the line built on `windows`/`windows-core` ^0.62, so no second copy of that family ships; it pulls `webview2-com-sys` 0.39.1 and `webview2-com-macros` 0.8.1 (rust-version 1.82). The same crate wry uses on Windows. MIT. | Hand-declared WebView2 COM vtables with `windows::core::interface` (the IDL is public), or open the mini app in the default browser. |
| `zbus` | Linux | The tray is `StatusNotifierItem` and the now-playing card is MPRIS; both are D-Bus interfaces with no C library worth linking. | Hand-rolled D-Bus over a Unix socket, which is a protocol implementation, not a weekend. |
| `x11rb`, `wayland-client`, `wayland-backend` | Linux | rise-platform and, since P135, rise-media (screen_capture X11 arm: RandR monitor sizes by the name atom libwebrtc uses as the screen id, window GetGeometry, on an own `RustConnection`). The display cadence (P100) reads the refresh of the monitor under the window on gpui's own display connection: XRandR CRTC mode lines through x11rb (`randr`, `allow-unsafe-code` to wrap gpui's `xcb_connection_t`) and `wl_output` current modes through a guest event queue on gpui's `wl_display` (`client_system`). The same versions gpui_linux already resolves, so Cargo.lock gains no crate. MIT. | The hub answers 60 Hz, the rate every panel shows. |
| `wayland-protocols` (`client`, `staging`), `wayland-protocols-wlr` (`client`) | Linux | pasteboard_write (P116) owns the Wayland selection through `ext_data_control_v1`, else `zwlr_data_control_v1`, to offer `x-kde-passwordManagerHint` and `text/uri-list` that gpui cannot offer; the X11 arm uses the x11rb above on its own connection. The same versions and features gpui_linux already resolves, so Cargo.lock gains no crate. MIT. | The copy goes through gpui as plain text. |
| `cosmic-text` =0.19.0, `swash` =0.2.10 | Linux | emoji_raster (P199): the guide sandbox stickers draw one emoji off screen into an RGBA raster, which iOS does with `NSString.draw` in `UIGraphicsImageRenderer`; gpui keeps its glyph rasteriser pub(crate). cosmic-text (fontdb over the fontconfig set, per character fallback to Noto Color Emoji) shapes and measures, swash rasterises colour outlines, colour bitmaps and masks; `swash` is named only to tell a premultiplied COLR image from a straight PNG strike (`swash::scale::Source`). Both are already in the lock through `gpui_wgpu`, same versions, no new crates. MIT OR Apache-2.0. | A sticker plate without its emoji (the seam answers Err), or rasterise through gpui once it exposes `rasterize_glyph`. |

Deliberately **not** added, and why, so nobody re-proposes them:

- `souvlaki` for media keys — 3.5k downloads/month and 7 dependents is a bus
  factor, not a dependency. `media_keys.rs` talks to MediaPlayer.framework and
  MPRIS directly instead.
- `tray-icon`, `global-hotkey` — better maintained than souvlaki, but they each
  want to own an event loop, and gpui already owns ours.
- `rfd` for file dialogs — gpui already has `prompt_for_paths` and
  `prompt_for_new_path` on all three platforms. The only thing it lacks is a
  file-type filter, and that is thirty lines of our own.
- a semver crate — the updater compares `major.minor.patch` and nothing else.

## The gpui pin

`gpui` on crates.io is stuck at 0.2.2 (October 2025) and predates the
`gpui_platform` split; `gpui_platform` is not published at all. Every serious
third-party app pins an exact git rev of the Zed monorepo, and so do we.

Bumping the rev is scheduled work with a test gate and a rollback, never an
incidental change. `[profile.dev.package.gpui] opt-level = 3` is mandatory — an
unoptimised gpui is unusable for interactive development.

### Vendored patches

`vendor/gpui-6b2aa1c/` holds `gpui`, `gpui_wgpu` and `gpui_windows` of the same rev with the
patches below, written by `scripts/vendor-gpui.py` and used through `[patch]` in the root
`Cargo.toml`; every other zed crate stays the git dependency. Each patch is minimal and listed here.

| Patch | Crates | Why | If it dies |
|---|---|---|---|
| `0001-owned-nv12-surface.diff` (P41) | `gpui`, `gpui_wgpu` | Off macOS `PaintSurface` had no payload and the wgpu renderer skipped surfaces; video frames get an owned NV12 frame and Linux draws it. | Drop it: off macOS the presentation seam (`rise-media` `texture/native_surface`) answers `NotOnThisHost` again. |
| `0002-windows-owned-surface.diff` (P42) | `gpui_windows` | The DirectX 11 renderer left `draw_surfaces` empty; it now uploads the 0001 frame into per-stream R8/R8G8 textures and draws it with a new surface shader. | Drop it: the Windows branch of `texture/native_surface` returns to the default `NotOnThisHost` answer. |
| `0005-macos-bgra-surface.diff` (P148) | `gpui_macos` | The Metal renderer drew a surface only from a `420f` CVPixelBuffer and asserted on any other format; a `32BGRA` buffer with premultiplied alpha is now one BGRA8Unorm texture drawn by `bgra_surface_fragment` with premultiplied blending (WebM stickers over the bubble). Needs `gpui_macos` in the root `[patch]`. | Drop it: the macOS branch of `texture/native_surface` must stop overriding `prepare_host_bgra_frame`, and stickers fall back to the poster frame (`NotOnThisHost`, P148). |

## Licensing

`gpui` and the whole `gpui_*` family are Apache-2.0 and safe to link into a
closed-source product. The rest of the Zed repository — `crates/ui`, `theme`,
`icons`, `editor`, `markdown`, `picker`, `workspace` — is GPL-3.0-or-later.
Read it as reference; never copy it in.

Not yet added, and each carries a condition:
- `gpui-component` is not taken: text input is our own `rise-ui` `input_ui`
  built on gpui's IME (`EntityInputHandler`).
- `ffmpeg` must be built LGPL-only: no `--enable-gpl`, no libx264/libx265,
  dynamically linked, sources published. Checked by `scripts/check-ffmpeg-license.py`.

## Media

| Crate | Why | If it dies |
|---|---|---|
| `cpal` | The sound device on three operating systems. CoreAudio, WASAPI and ALSA have nothing in common beyond the idea of a callback, so a hand-written seam here would *be* cpal. Apache-2.0. | `rodio` is a layer on top of it, not an alternative; below it the fallback is three per-OS bindings in `rise-platform`. |
| `symphonia` | Audio decode: MP3, AAC-LC, ALAC, FLAC, Vorbis, PCM. Pure Rust, so the music player carries no FFmpeg licensing burden and no C parser is fed attacker-supplied bytes. MPL-2.0, which is file-level copyleft: linking is fine, and modifying a symphonia file would oblige us to publish that file. We do not modify it. | The LGPL FFmpeg is already vendored for video and could decode audio too — at the cost of putting FFmpeg on the music player's critical path, which is exactly what this avoids. |
| LiveKit Rust SDK: `livekit` =0.7.32 in `rise-media` (no default features; `tokio`, `rustls-tls-native-roots`; linked by P43, the `rtc_media` seam), `libwebrtc` =0.3.26, `webrtc-sys` =0.3.23 (the versions gpui's Zed rev 6b2aa1c pins in its Cargo.toml; Zed patches them to its own fork, we take crates.io). `webrtc-sys-build` 0.3.21 resolves with them and pins prebuilt libwebrtc `webrtc-a545de4` (github.com/livekit/rust-sdks releases), taken from a local copy through `LK_CUSTOM_WEBRTC` when it is set. `livekit-api` =0.4.14 is also a direct dependency of `rise-media` and comes from `vendor/livekit-api-0.4.14` through `[patch.crates-io]` (P256): one added function, `set_signal_stream_opener`, because upstream reads the signal proxy only from `HTTPS_PROXY` and only as HTTP CONNECT; the tree is generated by `scripts/vendor-livekit-api.py`. Apache-2.0; libwebrtc BSD-3-Clause. | Calls and streams, the same SFU as iOS. | Raw `libwebrtc` bindings plus our own signalling over the LiveKit protocol. |
| `maplibre_native` =0.10.0 (crates.io, maplibre/maplibre-native-rs; rust-version 1.90) in `rise-media` for every target but Windows (no default features; `geojson`, `json`; linked by P38, the `map_view` seam, module `map_view/maplibre_map.rs`). Its build.rs takes the prebuilt core `core-a33d3f00fb9dbf9117ce18edd799aa0ca5bcce5b` (github.com/maplibre/maplibre-native releases): amalgam macos-arm64 metal, linux-arm64 and linux-x64 vulkan plus the headers archive, taken from a local copy through `MLN_CORE_LIBRARY_PATH` and `MLN_CORE_LIBRARY_HEADERS_PATH` when they are set. No prebuilt amalgam for Windows (build.rs panics there): the Windows map answers `NoNativeCore` with that reason (map_view/windows.rs). MIT/Apache-2.0; core BSD-2-Clause (LICENSES.core.md). Patched through `[patch.crates-io]` (X148): `vendor/maplibre_native-0.10.0/` is generated by `scripts/vendor-maplibre-native.py`, it adds `Responder::send` (one request answers many times) and moves the checkout of a from-source core build to `<target dir>/maplibre-native-checkout`. | The maps section, the same renderer family as iOS MapLibre. | Server-rendered raster tiles in a gpui view. |
| `lz4_flex` | Compresses rasterised sticker frames. The reference uses LZFSE, which is Apple-only; LZ4 is the portable equivalent at this ratio and decodes faster, which matters because a decode happens once per visible sticker per presentation. MIT. | `zstd` at level 1, at the cost of a C dependency; or the frames stay uncompressed and the sticker cache holds roughly a tenth as many. |
| `flate2` (`rust_backend`) | Inflates `.ros` and `.tgs` stickers, which are gzipped Lottie JSON. `rust_backend` so there is no system zlib on any of the three. MIT/Apache-2.0. | Hand-rolled DEFLATE, which is a weekend nobody should spend. |
| `cc` (build) | Compiles the vendored rlottie. Already in the tree transitively. | `cmake`, at the cost of requiring cmake and ninja on every machine and CI runner. |

## rlottie (vendored source, `rise-media/rlottie`)

Not a crate. `scripts/vendor-rlottie.sh` fetches a pinned, SHA-256-checked
upstream tarball into `vendor/rlottie/`, and `crates/rise-media/build.rs`
compiles its 35 translation units directly with `cc`, the way `rusqlite` bundles
SQLite. No cmake, no meson, no system library — no operating system ships one.

Licensing is not one licence: MIT for rlottie itself, BSD-3 for the Skia-derived
rasteriser, FTL for the FreeType fork, MIT for pixman and rapidjson, public
domain for stb, and **MPL-2.0 for one file** (`src/vector/vinterpolator.cpp`,
lifted from Firefox). All are link-safe for a proprietary product; the MPL file
obliges us to ship its source with the release.
`scripts/check-rlottie-license.py` re-derives all of that from the vendored tree
and prints the MPL file by name, so a version bump that changes it fails rather
than passing quietly.

Deliberately **not** taken from `../telegram`, which vendors its own fork: that
repository is GPLv2 and copying anything out of it would relicense this product.

If it dies: ThorVG is the successor project and is why `LottieRasterizer` is a
trait — everything above it is pure and would not change.

## rsmpeg (optional, `rise-media/ffmpeg`)

FFmpeg bindings. Chosen over ffmpeg-next because it exposes `AVHWDeviceContext`
and lets `hw_device_ctx` be set on an `AVCodecContext` — without which
libavcodec returns every frame on the CPU and the whole zero-copy texture path
is pointless.

Links only against the LGPL prefix from `scripts/build-ffmpeg-lgpl.sh`; the
crate's `build.rs` refuses to fall back to a system FFmpeg, because every
distribution's default is `--enable-gpl`.

If it dies: the alternatives are ffmpeg-next (loses hardware frames, so a
rewrite of the decoder rather than a swap) or hand-rolled bindings over
rusty_ffmpeg, which rsmpeg is a thin layer over and which we already build
against.

## swiftc (build prerequisite, macOS only)

Not a crate, and not optional. `crates/rise-platform/build.rs` compiles
`src/materials/macos/RiseGlass.swift` into a static archive on every macOS build and
**panics** when it can find no compiler, carrying the `xcode-select --install`
line that fixes it. A Mac without the Xcode command line tools cannot build this
workspace at all — not the glass bridge, the workspace. The failure is loud on
purpose: a silent skip would ship an app whose rail and sheets are missing their
material with nothing in a log. The Linux and Windows arms of that build script
return before touching Swift, so nothing outside macOS needs a Swift toolchain.

Why Swift rather than `objc2`, which the crate already links: `NSGlassEffectView`
is a Swift-first API whose configuration surface shifts between OS versions, and
the whole tier-1 design turns on `NSClassFromString` behind `#available` — which
the Swift compiler checks. The archive is compiled against the product's floor
(deployment target 13.0), never the build machine's SDK, so that `#available`
stays a real runtime check instead of being folded away.

If it dies: hand-rolled `objc2` message sends for the same view tree, with a
hand-maintained version comparison standing in for `#available`. The cost is the
availability checking — every configuration property becomes a raw selector that
fails on a user's machine rather than on ours.

## Assets

| Asset | Why | If it dies |
|---|---|---|
| `Inter` 4.1 (`assets/fonts`, SIL OFL 1.1) | The UI typeface, vendored as six static TTFs (300/400/500/600/700/900) rather than resolved from the host. Six because the reference draws with six: `.light`, `.regular`, `.medium`, `.semibold`, `.bold` and `.heavy`. `rise_theme::Typography` snaps any other weight to the nearest shipped face, because a weight with no face makes the text stack substitute a different family per OS — the exact failure vendoring a font is meant to remove. CoreText, cosmic-text and DirectWrite each map a system family name to a different file with different metrics, so a layout measured against the host font is correct on one OS only — and on Linux `system-ui`/`sans-serif` need not resolve at all. Inter is the neutral UI face closest to the SF Pro the iOS reference draws with, and the OFL permits redistribution inside a closed-source bundle provided `OFL.txt` ships with it. Static faces, not `InterVariable.ttf`, because font-kit's named-instance and `wght`-axis handling is not verifiable across all three stacks here. | Any OFL/Apache text face with the same six weights and a similar x-height — Public Sans, Source Sans 3, IBM Plex Sans. Swapping it re-measures every fixed layout in the storybook, which is why the release is pinned by SHA-256. |

## flag-icons (vendored assets, `assets/icons/flags`)

Not a crate. 256 SVGs, one per region code, 4:3, copied verbatim from the
`flags/4x3/` directory of flag-icons **7.5.0** (MIT, Copyright (c) 2013
Panayiotis Lipiridis). Pinned by npm tarball and its sha512
`kd+MNXviFIg5hijH766tt+3x76ele1AXlo4zDdCxIvqWZhKt4T83bOtxUOOMlTx/EcFdUMH5yvQgYlFh1EqqFg==`.
1.62 MiB total.

Why it exists: the reference draws a flag by adding 127397 to each letter of a
region code, which yields a regional-indicator pair that only Apple's system
font ligates. On Linux and Windows that renders as two letters in boxes, and no
font choice fixes it — the flag has to stop being a glyph and become an asset.

If it dies: the files are static SVGs under MIT and already in the tree, so
nothing breaks; a successor set (`circle-flags`, or Wikimedia's public-domain
SVGs) would be a swap of the directory. The lookup contract deliberately allows
a miss — the fallback is a theme-painted chip carrying the two-letter code — so
an incomplete replacement degrades instead of failing.

Not a routine upgrade: which flags exist and how their borders are drawn is a
product decision, not a version bump.

## Lucide icons (vendored SVG, `assets/icons/lucide`)

Not a crate. 169 SVG files, 60 KB, pinned to the `1.28.0` GitHub release
(`lucide-icons-1.28.0.zip`, sha256
`79b02addeac22305ac49b238cb810161c4bfe7c60856a6d6b8f37cd42f1f6365`), plus that
tag's `LICENSE`. Only the icons `assets/icons/sf-to-lucide.json` actually names
are vendored, not the 1,756-icon set.

It exists because SF Symbols is an Apple-platform-only font that cannot be
redistributed in a Linux or Windows build, and the reference names 228 distinct
symbols across `systemName:` and `systemImage:` (excluding `build/`, which holds a
third-party dev tool's own SwiftPM checkouts). The map carries those plus
`folder`, which the desktop rail needs and the phone has no equivalent of.
`sf-to-lucide.json` is keyed by the SF name so a ported call site stays
character-for-character identical to the Swift; the `approximate` list still
needs a design review.

Licensing is two licences in one file: ISC for Lucide, and MIT (Cole Bemis) for
the icons inherited from Feather, which `LICENSE` enumerates by name. Both are
permissive and require only that the notice ship with the product, which is why
`LICENSE` sits next to the SVGs and is bundled with them.

A version bump is not free: Lucide renames icons between releases and keeps the
old name only as an alias in its JS packages, never as a file on disk, so a bump
must be re-verified against the map or a renamed icon becomes a blank glyph at
runtime.

If it dies: Feather is the ancestor and is unmaintained; Phosphor (MIT) and
Tabler (MIT) are the live alternatives at comparable coverage. Because every
call site names an SF symbol and not a Lucide icon, replacing the set is a
rewrite of one JSON file and a re-fetch, not a change to any screen.

## Shared core from riseonly-tools

Path dependencies on `../riseonly-tools/crates`, one canon for the four trees
(Cargo.toml `[workspace.dependencies]`). Values are never edited here: the
source is `riseonly-tools/canon`, the gate is `codegen check`, CANON lands
changes.

| Crate | Why | If it dies |
|---|---|---|
| `rise-perm`, `rise-moderation`, `rise-rtc`, `rise-music`, `rise-limits`, `rise-events`, `rise-catalog`, `rise-chat`, `rise-endpoints`, `rise-regions`, `rise-bot-api` | The eleven riseonly-tools crates linked today: permissions, moderation, call contract, music, limits, events, catalog, chat contract, endpoints, regions, bot api (reply_markup wire keys). The desktop had none of them; linking is a pure gain. | They are ours; a dead crate is fixed in riseonly-tools. |
| `rise-gifts` | The gifts vocabulary of riseonly-tools (`canon/gifts.json`): item kinds, media slots, attribute kinds, rarity classes, weights. `RiseGiftItemKind` and the rest of RiseCanon's gifts names on the desktop. | It is ours; a dead crate is fixed in riseonly-tools. |
| `rise-privacy` | The privacy vocabulary of riseonly-tools (`canon/privacy.json`): `BlockScope` for `RiseBlockScope` in `rise_user_block_wire_models.rs`. rise-canon links it but does not re-export it. | It is ours; a dead crate is fixed in riseonly-tools. |
| `icu_provider` | Only to turn on its `sync` feature: without it ICU payloads hold `Rc`, `DateFormatter` is not `Send` and the shared formatter statics of iOS cannot be written. Already in the lock through `icu_datetime`. | Goes with the ICU crates. |
| `rise-otp` | The one-time code vocabulary of riseonly-tools: channels and their wire spellings, read by `AuthCodeChannel`. No dependencies. | Literal channel names, which drift from the canon silently. |
| `rise-canon` | The generated client facade of riseonly-tools: client serde models with the Swift type names, request bodies, event payloads, HTTP paths, server texts, plus `pub use` of the other riseonly-tools crates. Desktop writes `use rise_canon::...` the way iOS writes `import RiseCanon`. | Ours; the emitter lives in riseonly-tools. |

## Approved, not yet linked

Each version is pinned when the crate is added
(row updated then).

| Crate or tool | Why | If it dies |
|---|---|---|
| `wry` | The system web view for the screens where iOS uses `WKWebView`. | Opening the link in the default browser. |
| `pipewire` 0.8 (Linux, also screen share system sound, screen_capture/linux.rs) | Inside Flatpak the camera is reached only through `org.freedesktop.portal.Camera`, whose `OpenPipeWireRemote` hands back a PipeWire fd; frames are read from that remote (camera_capture, P33/P75). MIT. Approved by . | Without it, Flatpak builds report the camera Unsupported; outside the sandbox V4L2 still works. |

## Build machines

### Linux: OrbStack machine `rise-desktop-linux`

`scripts/linux-check.sh <ws> check|test` runs cargo inside it. Setup:

- `orb create -a arm64 ubuntu:noble rise-desktop-linux` (Ubuntu 24.04.5 LTS).
- apt, `--no-install-recommends`: the packages of `.github/workflows/ci.yml`
  (`libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libfontconfig-dev
  libvulkan1 libasound2-dev cmake clang lld`) plus what the `-sys` crates of
  Cargo.lock probe for on Linux: `build-essential pkg-config libdbus-1-dev
  libfreetype-dev libclang-dev`, and `curl ca-certificates git rsync`.
- rustup-init 1.29.2 from `static.rust-lang.org/rustup/dist/aarch64-unknown-linux-gnu`,
  sha256 checked against the published `rustup-init.sha256`; toolchain 1.95.0
  (profile minimal, rustfmt, clippy) in the machine's own `~/.rustup` and `~/.cargo`.
- Registry: `~/.cargo/registry/{cache,index}` and `~/.cargo/git/db` copied from
  the Mac, so `--offline` works; sources are unpacked inside the machine.
- Memory: OrbStack's VM is shared with the backend docker containers, so the
  VM limit stays as it is and cargo runs in a systemd user scope with
  `MemoryMax=6G` (`RISE_LINUX_MEM`).
- Target dir `files/riseonly-desktop-build/target-linux` on the Mac disk
  (about 1.6 GB after a check).

Rebuilding the machine is the list above; nothing else lives in it.

### Windows: cargo-xwin on the Mac

`scripts/windows-check.sh <ws>` runs `cargo xwin check --target
x86_64-pc-windows-msvc --workspace --tests`. Setup:

- `cargo install cargo-xwin --version 0.23.1 --locked` (crates.io, MIT).
- Homebrew `llvm` 23.1.4 (keg-only, `/opt/homebrew/opt/llvm/bin`): `clang-cl`,
  `llvm-lib` and `llvm-rc` for the `cc` and `embed-resource` build scripts.
- Microsoft CRT and Windows SDK headers and libraries, fetched by xwin into
  `~/.cache/cargo-xwin` (about 0.6 GB); the licence is accepted with
  `XWIN_ACCEPT_LICENSE=1` in the script.
- `scripts/xwin-llvm-rc.sh`: the llvm-rc that gpui's manifest needs from macOS
  (embed-resource runs llvm-rc in the `.rc` folder, gpui names the manifest
  from the crate root).
- Target dir `files/riseonly-desktop-build/target-win`.

| Tool | Why | If it dies |
|---|---|---|
| `cargo-xwin` 0.23.1 | Windows type-check of every `windows.rs` seam without a Windows machine. | A Windows machine or CI runner. |
| `icu_experimental`, `fixed_decimal` | Relative time phrases for the `RelativeDateTimeFormatter` twin from the same CLDR data Apple uses (`rise-i18n`). ISC, Unicode-3.0. | The relative time part moves to `icu4x` stable when it lands there; the phrases are CLDR data, so a generated table replaces the crate. |
| `icu_collator` | Language-aware sort order for the `compare(_:locale:)` and `localizedCaseInsensitiveCompare` twins in `rise-core::foundation` from the same CLDR data Apple sorts with. Unicode-3.0. | Comparing by UTF-16 code unit: the names keep their letters, but accented ones land at the end of the list. |
| Homebrew `llvm` | The clang-cl, llvm-lib and llvm-rc the cross check calls. | The LLVM release tarball from github.com/llvm/llvm-project. |
| `gtk` 0.18, `gdkx11` 0.18, `webkit2gtk` 2.0 (v2_40), `javascriptcore-rs` 1.1 (v2_28) | Linux only: the riseonly-web-view-helper process that hosts bot mini app pages (rise-platform feature web_view_helper). | crates.io; system libwebkit2gtk-4.1 and libgtk-3 at run time. |
