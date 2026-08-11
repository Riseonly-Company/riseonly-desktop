# Dependencies

The reference keeps external dependencies to a minimum and so do we. Every crate
below has a reason and an exit. Adding one without a row here is a review reject.

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
| `parking_lot` | Cheaper mutexes than std for hot paths. | std. |
| `smallvec` | Avoids allocating for the small collections the shell holds per frame. | `Vec`. |
| `tracing` | Structured logging, compiled out in release paths that need it. | `log`. |
| `directories` | Per-OS data directory, outside cloud-synced folders. | Hand-rolled per OS. |
| `base64` | Encodes secret bytes for the OS credential stores, which disagree about bytes vs strings (Secret Service is text-oriented). One encoding beats three byte paths. | Hand-rolled; it is 40 lines. |
| `fs4` | Advisory file lock on the account database. A desktop app can be launched twice; an advisory lock is released by the OS when the process dies, which a lock file is not. | Per-OS `flock`/`LockFileEx` in `rise-platform`. |
| `anyhow`, `thiserror` | Error plumbing: typed per subsystem, boxed at boundaries. | std. |
| `sha2` | Verifying a downloaded update against the hash the manifest declares, and hashing an account id before it becomes a credential-store entry name (that name is visible in Keychain Access and `secret-tool search`). An update you cannot verify is worse than no update, so this is not optional. | `ring`, or the `Security.framework`/CNG digests per OS. |

## Phase 3 native seams

These four are target-gated, so a macOS build never compiles the other two, and
none of them is reachable from `rise-engine`. Each exists because gpui has no
equivalent, not because it was more convenient.

| Crate | Target | Why | If it dies |
|---|---|---|---|
| `objc2`, `objc2-foundation`, `objc2-app-kit` | macOS | The tray, the Dock badge, launch-at-login and the macOS version probe are AppKit and ServiceManagement, and gpui exposes none of them. Pinned to the same 0.6/0.3 family gpui already links, so there is no second ObjC runtime in the binary. | Nothing else is viable for AppKit from Rust; the fallback is hand-written `msg_send` against the ObjC runtime, which is what `objc2` already is. |
| `raw-window-handle` | macOS | Reads the public AppKit `NSView` handle from `gpui::Window` so `rise-platform` can reach the owning `NSWindow` and GPUI's Metal layer and give both the same system corner. The version matches GPUI's own resolved 0.6 trait. MIT/Apache-2.0. | Add an equivalent native-view accessor to GPUI or maintain the one-line accessor in the pinned GPUI fork. |
| `windows` | Windows | `Shell_NotifyIconW` for the tray and the `HKCU\...\Run` value for autostart. Microsoft's own bindings, generated from the platform metadata. | `winapi`, which is unmaintained, or hand-declared `extern "system"` items. |
| `zbus` | Linux | The tray is `StatusNotifierItem` and the now-playing card is MPRIS; both are D-Bus interfaces with no C library worth linking. | Hand-rolled D-Bus over a Unix socket, which is a protocol implementation, not a weekend. |

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

## Licensing

`gpui` and the whole `gpui_*` family are Apache-2.0 and safe to link into a
closed-source product. The rest of the Zed repository — `crates/ui`, `theme`,
`icons`, `editor`, `markdown`, `picker`, `workspace` — is GPL-3.0-or-later.
Read it as reference; never copy it in.

Not yet added, and each carries a condition:
- `gpui-component` (Apache-2.0) will become a hard dependency the moment we need
  text input, because gpui ships no input widget and no selection over static
  text. Phase 1 does not need it.
- `ffmpeg` must be built LGPL-only: no `--enable-gpl`, no libx264/libx265,
  dynamically linked, sources published. Enforced in CI when it lands.

## Phase 4 media

| Crate | Why | If it dies |
|---|---|---|
| `cpal` | The sound device on three operating systems. CoreAudio, WASAPI and ALSA have nothing in common beyond the idea of a callback, so a hand-written seam here would *be* cpal. Apache-2.0. | `rodio` is a layer on top of it, not an alternative; below it the fallback is three per-OS bindings in `rise-platform`. |
| `symphonia` | Audio decode: MP3, AAC-LC, ALAC, FLAC, Vorbis, PCM. Pure Rust, so the music player carries no FFmpeg licensing burden and no C parser is fed attacker-supplied bytes. MPL-2.0, which is file-level copyleft: linking is fine, and modifying a symphonia file would oblige us to publish that file. We do not modify it. | The LGPL FFmpeg is already vendored for video and could decode audio too — at the cost of putting FFmpeg on the music player's critical path, which is exactly what this avoids. |
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
`src/macos/glass/RiseGlass.swift` into a static archive on every macOS build and
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

## Phase 5 assets

| Asset | Why | If it dies |
|---|---|---|
| `Inter` 4.1 (`assets/fonts`, SIL OFL 1.1) | The UI typeface, vendored as six static TTFs (300/400/500/600/700/900) rather than resolved from the host. Six because the reference draws with six: `.light`, `.regular`, `.medium`, `.semibold`, `.bold` and `.heavy`. `rise_theme::Typography` snaps any other weight to the nearest shipped face, because a weight with no face makes the text stack substitute a different family per OS — the exact failure vendoring a font is meant to remove. CoreText, cosmic-text and DirectWrite each map a system family name to a different file with different metrics, so a layout measured against the host font is correct on one OS only — and on Linux `system-ui`/`sans-serif` need not resolve at all. Inter is the neutral UI face closest to the SF Pro the iOS reference draws with, and the OFL permits redistribution inside a closed-source bundle provided `OFL.txt` ships with it. Static faces, not `InterVariable.ttf`, because font-kit's named-instance and `wght`-axis handling is not verifiable across all three stacks here. | Any OFL/Apache text face with the same six weights and a similar x-height — Public Sans, Source Sans 3, IBM Plex Sans. Swapping it re-measures every fixed layout in the storybook, which is why the release is pinned by SHA-256 in `assets/fonts/CONTEXT.txt`. |

## flag-icons (vendored assets, `assets/icons/flags`)

Not a crate. 256 SVGs, one per region code, 4:3, copied verbatim from the
`flags/4x3/` directory of flag-icons **7.5.0** (MIT, Copyright (c) 2013
Panayiotis Lipiridis). Pinned by npm tarball and its sha512
`kd+MNXviFIg5hijH766tt+3x76ele1AXlo4zDdCxIvqWZhKt4T83bOtxUOOMlTx/EcFdUMH5yvQgYlFh1EqqFg==`.
1.62 MiB total. `assets/icons/flags/CONTEXT.txt` records the pin, what was left
behind, and the lookup rule.

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
product decision, not a version bump. See the CONTEXT.txt.

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
character-for-character identical to the Swift; see `assets/icons/CONTEXT.txt`
for that rule and for the `approximate` list a designer still has to review.

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
