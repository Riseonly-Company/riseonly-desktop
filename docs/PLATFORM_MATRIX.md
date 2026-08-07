# Platform matrix — what is verified, and what is only written

This file exists so that nobody has to guess. The project targets macOS, Linux
and Windows from one codebase, but **only macOS is built and run today**. Saying
"cross-platform" without saying which arms have actually executed is how a team
discovers in month four that two thirds of its seams never worked.

The rule this table enforces: a claim of coverage must be traceable to something
that ran.

## What the three columns mean

- **Verified** — the code path executed on a real machine, or a test that would
  fail if the behaviour were wrong ran in the suite. On macOS both are possible.
- **Compiled** — the code is type-checked by the CI matrix
  (`ubuntu-22.04`, `windows-latest`) but has never executed. It can be wrong in
  every way a compiler does not catch.
- **Written** — the branch exists and is honest about what it does, but neither
  compiles here nor has run. Only used where the arm deliberately reports
  `Unsupported`.

Anything marked `Unsupported` is not a gap in this table. It is a seam that
reports, correctly, that it cannot do the thing — which is the whole point of
`PlatformSupport`. A caller can then offer an alternative instead of silently
getting nothing.

## Policy vs binding

Nearly every seam in `rise-platform` is split in two:

```
    pure policy  ──  takes HostOs as an argument, tested for all three on a Mac
         │
    OS binding   ──  #[cfg(target_os)], one call, no decisions
```

That split is why the "Verified" column is not simply "macOS only". The
*decisions* Linux and Windows will make are tested here today; what is unverified
elsewhere is the final call into the OS API. Where a row below says a Linux
decision is verified, it means exactly that and nothing more.

## The seams

| Seam | macOS | Windows | Linux | What is actually unverified |
|---|---|---|---|---|
| `secure_store.rs` | Verified | Compiled | Compiled | Nothing OS-specific: the trait and the entry naming are pure. |
| `keyring_store.rs` | Verified — real Keychain round trip | Compiled | Compiled | Credential Manager and Secret Service have never stored a byte. `is_available()` does a real round trip rather than trusting a build flag, so a missing Secret Service is detected at runtime instead of assumed. |
| `paths.rs` | Verified | Compiled | Compiled | The per-environment sibling directories and cloud-sync detection are exercised on macOS only. |
| `single_instance.rs` | Verified | Compiled | Compiled | `fs4`'s advisory lock. The trap is already handled: `try_lock_exclusive` returns `Ok(false)` on contention and `Err` only on a failed syscall, so treating it as a plain `Result` silently admits a second process. Whether Windows' `LockFileEx` releases on a hard kill as promptly as `flock` is untested. |
| `deep_link.rs` | Verified | Verified | Verified | Nothing — it is pure parsing with no OS call at all. |
| `gpui_shim.rs` | Verified | Verified | Verified | The capability answers are `cfg!` constants asserted against this build's target; they encode gpui's behaviour, they do not query it. |
| `notifications.rs` | Verified | Compiled | Compiled | The whole filtering policy is pure and tested. gpui implements `show_system_notification` on all three backends, so the binding is one call everywhere. Unverified: that Windows' toast tag digest and Linux's notification-server `replaces_id` really do collapse forty messages into one banner. Delivery can still be refused (authorization denied) and gpui no-ops then — the seam reports `Blocked`, it does not pretend. |
| `tray.rs` | Verified — `NSStatusItem` + Dock badge | **Written blind**, fully implemented | `Unsupported`, reason recorded | `Shell_NotifyIconW` has never run. Linux is *not* implemented: `StatusNotifierItem` is a D-Bus object that must stay exported for the app's lifetime, and this seam is synchronous and main-thread-bound while `zbus` here has `blocking-api` off. It reports `Unsupported` and names the interface. `media_keys.rs` can serve MPRIS only because its service is `async` and borrows the caller's runtime; the tray has no such caller. Closing this means giving the tray an executor, which is a decision, not an implementation detail. |
| `window_chrome.rs` | Verified | Compiled | Compiled | Every per-OS decision (control side, decoration mode, titlebar height, reserved inset) is a pure function tested for all three. Unverified: that a Linux compositor actually grants the client-side decorations we request — which is why the request and the grant are separate types. |
| `autostart.rs` | Verified — `SMAppService` | **Written blind** | **Real, and exercised on this Mac** | Linux is std-only file writing, so its tests genuinely run here; only the two env reads are unrun. Windows' `HKCU\...\Run` write has never executed. macOS reports `Unsupported` for a bare-binary run, because `SMAppService` needs a real bundle — which is why this project always runs from `Riseonly.app`. |
| `file_dialog.rs` | Verified | Verified | Verified | No `#[cfg(target_os)]` at all. The per-OS difference (macOS can offer one dialog that takes files *or* directories; Windows and Linux cannot) is read off gpui's source at the pinned rev and degrades explicitly rather than silently. |
| `reveal_in_file_manager.rs` | Verified | Verified | Verified | No `#[cfg(target_os)]` at all — every decision is pure and all three arms are asserted. The binding is the same three gpui calls everywhere. The bundle test judges the *resolved* path, not the caller's spelling: on macOS an application bundle is a directory, so "show me this folder" would otherwise become "run this" for a symlink or a trailing separator. |
| `global_hotkey.rs` | Verified — Carbon `RegisterEventHotKey` | `Unsupported`, reason recorded | `Unsupported`, reason recorded | Carbon was chosen over an `NSEvent` global monitor deliberately: it needs no Accessibility permission, and demanding that on first launch to support one shortcut is a bad trade. Windows needs a message loop gpui owns and exposes no hook for; X11 needs an x11/xcb crate that is not in the dependency set; Wayland has no global-shortcut protocol at all by design, only the desktop portal. Each is a named `UnavailableReason`, not a silent no-op. |
| `media_keys.rs` | Verified — MediaPlayer.framework | `Unsupported`, reason recorded | **Written blind** — MPRIS over D-Bus | `SystemMediaTransportControls` is WinRT and needs an interop cast from an HWND this layer does not own; faking it was refused. The macOS card needs a real bundle identity to appear at all. The republish-throttle is pure and tested — a steady 1× playback provably does not republish every tick. |
| `updater.rs` | Verified | Compiled | Compiled | No `#[cfg(target_os)]`; the install plan is a pure function of host *and install shape*. Deliberately does **not** fetch bytes — network belongs to `rise-engine`. Applying is refused outright until Phase 8 provides a signed feed, and that refusal is a tested state rather than a comment. Note the real trap found while writing it: gpui's `path_for_auxiliary_executable` is `todo!()` on Windows at this rev, so the helper is resolved beside `current_exe()` instead, and `HelperUnavailable` is a genuine outcome. |
| `materials.rs` | Verified — `NSProcessInfo` version probe | Compiled | Compiled | Policy and types only; the AppKit side is the row below. Every (material, host, version) combination is asserted, including that an unrecognised macOS version falls back *down* — assuming glass and getting nothing is a blank rail, assuming painted and getting a flat panel is merely plainer. `current_glass_surface()` is the one entry point, and the painted surface it returns off macOS is a named value (`painted_glass_surface()`) precisely so that the branch which never runs here is still exercised here. |
| `macos/glass/` — `RiseGlass.swift` + `MacGlassSurface` | **Split.** Policy and the ABI: Verified. Hosting a view: **driven once by hand, not by `cargo test`** — see below | `Unsupported` — `current_glass_surface()` returns the painted surface | `Unsupported`, same | The Swift compiles and links into the test binary, and the suite calls into it: the probe reports what this machine can instantiate, and `RiseGlass.swift` is asserted to name the same `NSVisualEffectMaterial` cases as `Material::vibrancy_material()`, so the two halves cannot drift apart unnoticed. What `cargo test` **cannot** reach is hosting an actual view: libtest runs every test on a spawned thread, so no test is ever on the AppKit main thread, and a surface with no window correctly reports `GlassError::Refused`. The view tree itself — container inserted below the `GPUIView` and above gpui's own blur view, a real `NSGlassEffectView` instantiated on macOS 26.1, the corner radius reaching the native view, a region reusing its view across passes and a region that stopped being laid out being destroyed, `clear` leaving nothing, `detach` removing the container — was driven once against the compiled archive by a main-thread harness outside the repository. That is a one-off observation, not coverage; nothing in CI re-checks it. |

## What app/ must still do for these seams to work

A seam being correct is not the same as it being wired. These are obligations on
the application, and each one fails *silently* if forgotten — which is why they
are written here rather than left in a commit message.

- **`App::set_app_identity(identifier, name)` during startup.** Windows drops
  every toast from an unpackaged application that has not set one, without an
  error. macOS and Linux do not need it. Nothing in `rise-platform` can call it
  for you: it is a `gpui::App` method and the app owns the `App`.
- **`App::on_system_notification_response`** must be registered, or clicking a
  banner does nothing. Route it through `NotificationTag::conversation_route`.
- **Throttle notifications by tag yourself on Linux.** `tag_replaces_previous`
  reports `Unsupported` there because gpui's Linux backend never sets the XDG
  `replaces_id`, so forty messages in one conversation stack forty banners.
- **Run from `Riseonly.app`, never a bare binary.** Keychain ACLs, the
  now-playing card and `SMAppService` all key off bundle identity, and each
  degrades quietly rather than failing loudly without it.
- **Build the glass surface from the layout pass, and drop it on the main thread.**
  `current_glass_surface()` returns a value that owns AppKit views: it is neither
  `Send` nor `Sync`, and dropping it anywhere but the main thread leaks the
  container instead of removing it. `commit` takes a `GlassLayout` stamped with a
  `Generation` and refuses anything that is not newer, so a render pass cannot
  move a region even by accident. The first `commit` before a window exists
  reports `GlassError::Refused`, which is an ordinary startup state, not a fault.
- **A macOS build machine needs `swiftc`.** `crates/rise-platform/build.rs`
  compiles `RiseGlass.swift` and panics with `xcode-select --install` when it is
  missing rather than skipping, because a build that quietly omits the bridge
  ships an app with a blank rail and no diagnostic anywhere. Linux and Windows
  builds do nothing at all.
- **`single_instance` and `deep_link` are wired as of Phase 6.** `main.rs`
  acquires the lock before it opens anything, the serving task owns the
  `PrimaryInstance` so the lock lives exactly as long as the app, and
  `app/src/app/shell/handover.rs` polls the inbox and turns handed-over argv into
  routes. It drains a second inbox as well: macOS delivers a link to an already
  running app as an Apple Event, never as argv, so `Application::on_open_urls`
  parks those links for the same loop. Windows and Linux hand links over as argv
  and use only the file inbox — the URL inbox is simply always empty there.
  Closing the last window ends the process (`QuitMode::LastWindowClosed`),
  because a windowless process would hold the lock with no menu bar and no
  window-scoped shortcuts to open another one; a real macOS menu bar plus
  `on_reopen` is Phase 8 work. Exercised on macOS by hand: a second launch carrying two links exits
  without opening a window while the first logs both screens. The poll interval
  is 400 ms, so a link takes up to that long to appear — a watch instead of a
  poll would need this crate to own a thread, which it deliberately does not.
  A failure to take the lock is logged and the app still starts; on a filesystem
  with no advisory locking that means two instances, which is better than
  refusing to run.

## Standing limitations, all platforms

- **No native popups anywhere.** GPUI has them only on Wayland, so every menu,
  dropdown and tooltip is an in-window overlay on every platform.
  `gpui_shim::supports_native_popups()` returns `Unsupported` unconditionally and
  a test pins it, because a design that assumes a real popup breaks X11 and
  Windows. Since Phase 6 the arithmetic that enforces it lives in
  `rise_widgets::place`, which is total: any anchor, any size — including an
  overlay larger than the window — resolves to an origin inside the frame.
- **Screenshot tests are macOS-only.** `render_to_image` is implemented nowhere
  else and `current_headless_renderer()` returns `None`. Behavioural tests run
  everywhere; pixel tests do not.
- **Dragging content out of the app** does not exist on Windows — `gpui` has no
  `start_external_drag` there. Callers must offer copy or save instead. Nothing
  calls it yet: dragging IN is wired and policed as of Phase 6, dragging OUT
  waits for something in the app to own a file.
- **The window cannot report that it is minimised.** `gpui` can minimise a window
  and cannot tell you it is minimised, and it knows nothing about system idle or
  a locked screen. `SessionActivity` takes all three as signals and is tested on
  all of them; only window focus and the window count have a sensor today. The
  three missing ones need a seam here that does not exist.
- **Windows CJK input** goes through IMM32 with no TSF, which is the weakest part
  of the whole stack — weaker than Linux, contrary to reputation.

## rise-media, texture import (Phase 4, first slice)

| Mechanism | macOS | Linux | Windows |
|---|---|---|---|
| Zero-copy import | **Verified.** IOSurface; VideoToolbox already decodes into GPU memory, so the import is a no-op that keeps the buffer alive | Written, `NotExercised` | Written, `NotExercised` |
| Presentation | **Verified.** `gpui::surface()` takes the CVPixelBuffer directly | Owned `wgpu::Texture` — `surface()` does not exist off macOS | Same as Linux |
| Hardware decode | **Verified** that VideoToolbox opens; no bitstream has been decoded end to end | VAAPI, unrun | D3D11VA, unrun |
| Import bodies | n/a — nothing to import | **NOT WRITTEN.** The DMA-BUF path returns `NotExercised` rather than a stub that appears to work | **NOT WRITTEN**, same |

What *is* verified on all three, on a Mac: every per-host **decision** —
`ImportPlan`, `HwAccel::for_host`, `hardware_support`, `copy_row_alignment`,
the colour matrices, the feed scheduler's pool policy and the memory ceiling.
These are pure functions of `HostOs`, exactly as `host_os.rs` intends, so the
only thing left unverified elsewhere is the final call into the OS API.

- **The DMA-BUF trap, written down before someone rediscovers it.** VAAPI's
  `vaExportSurfaceHandle` gives an fd per plane *plus a DRM format modifier*.
  The image must be created with that modifier, not with `OPTIMAL` tiling.
  Getting it wrong yields a correctly-sized, garbled picture, not an error.
- **The DXGI trap.** D3D12 cannot open a legacy shared handle at all. The
  texture must be created with `D3D11_RESOURCE_MISC_SHARED_NTHANDLE`.
- **FFmpeg is never the system one.** Homebrew's is `--enable-gpl` with
  libx264/libx265, and linking it relicenses this product. `build.rs` refuses to
  guess; `cargo make ffmpeg-license` re-derives the licence from the artefact;
  and a test asserts it against `avcodec_configuration()` on the **linked**
  library, which no script edit can fake.

## rise-media, audio (Phase 4)

| Mechanism | macOS | Linux | Windows |
|---|---|---|---|
| Decode (symphonia) | **Verified.** A real WAV is built in the test and decoded frame for frame; seek, truncation and a lying header all covered | Same code, no `cfg` anywhere — pure Rust | Same |
| Device output (cpal) | **Verified** on a machine with a sound card; the test skips out loud where there is none | ALSA, unrun | WASAPI, unrun |
| Configuration choice | **Verified for all three shapes on a Mac** — CoreAudio's wide range, WASAPI shared mode's single fixed rate, ALSA's several formats are each a case in the suite | as macOS | as macOS |
| Resampling | **Verified** — a 1 kHz tone stays 1 kHz across 44.1→48, 48→44.1 and 22.05→48, and block boundaries are continuous | unrun in situ | unrun in situ, **and this is the arm that needs it**: WASAPI shared mode does not negotiate, so every 44.1 kHz track is converted |
| Gapless trim | **Verified.** `iTunSMPB` parsing, the LAME/Xing tag, and a two-track album joining with no lost frames and no added silence | as macOS — pure arithmetic | as macOS |

There is no `#[cfg(target_os)]` anywhere in `audio/`. Every OS difference is
inside cpal, and every decision about it — which rate, which format, whether to
resample — is a pure function of what the device advertises, so all three device
shapes are exercised here.

**Opus and AMR are refused by name.** `file-service` accepts both; neither has a
pure-Rust decoder, and routing them through the LGPL FFmpeg would put FFmpeg on
the music player's critical path, which PHASES.txt explicitly avoided. They
report `Unsupported::NoPureRustDecoder` rather than producing silence. Voice
messages are Ogg/Opus, so this has to be revisited when chat lands in Phase 7.

## rise-media, stickers (Phase 4)

| Mechanism | macOS | Linux | Windows |
|---|---|---|---|
| Rasterisation (rlottie) | **Verified.** A real Lottie document is rasterised by the linked library and its pixels asserted, including byte order | Same C++ source, compiled by `build.rs`; unrun | Same; the MSVC flag set is written blind |
| Frame cache, tick, playback budget | **Verified** — all pure, no OS anywhere | as macOS | as macOS |
| Atlas | **Verified** as allocation policy. Nothing has been uploaded to a GPU yet: the texture itself is Phase 5 | as macOS | as macOS |
| Container handling | **Verified** for JSON, `.ros`/`.tgs` gzip, gzip bombs and truncation. `.lottie` (zip) is detected and refused by name | as macOS | as macOS |

- **rlottie is vendored, never a system library.** No operating system ships one.
  `scripts/vendor-rlottie.sh` fetches a pinned, hash-checked upstream tarball and
  `scripts/check-rlottie-license.py` re-derives the licensing from the tree —
  including the one MPL-2.0 file, whose source has to ship with the release.
  Nothing is taken from `../telegram`, which is GPLv2.
- **The image module is compiled in, not dlopened.** Upstream's default builds a
  separate `librlottie-image-loader` and loads it at runtime, which would mean
  shipping and signing a second binary inside the `.app`. Without an image
  loader, a sticker with an embedded raster asset renders blank.
- **A sticker gets no filesystem root.** `resource_path` is empty, so an
  animation cannot name a file on the machine. Embedded base64 assets still work.
