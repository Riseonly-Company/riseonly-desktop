# Architectural boundaries

`scripts/check-boundaries.py` checks the rules below, and CI builds every change
for macOS, Linux and Windows.

## 1. `rise-engine` never imports gpui or an OS crate

`crates/rise-engine` is the twin of `Packages/RiseEngine` in the iOS reference.
It stays free of gpui and of platform APIs so it is testable headless. This rule applies to the `rise-engine` crate only:
`modules/*/engine/**` inside the `riseonly` crate follows the iOS files 1 to 1,
gpui included where iOS imports SwiftUI.

Checked in the manifest and in every source file of the crate.

The crate reaches the OS only through `rise-platform` seams
(`secure_store` under `RiseKeychainKeyStore`, `file_manager::FileManager`,
`system_activity` in uploads, `url_resource_values`). `rise-platform` has a
default cargo feature `gpui` (`dep:gpui`, `dep:gpui_platform`); every seam that
touches gpui, and every seam that depends on one, carries
`#[cfg(feature = "gpui")]` from the `cfg` list of `rise-platform` in
`scripts/crate-map.json` (written by `gen-mod-tree.py`, never by hand), and so
do the matching crate-root re-exports in `lib.rs`. `rise-engine` depends on
`rise-platform` by path with `default-features = false`, so its dependency graph
holds no gpui; the app and the kit crates keep the default. A new engine import
from `rise-platform` must name a module outside that `cfg` list; a seam that
starts to use gpui joins the list in the same submission.

## 2. Nobody writes `#[cfg(target_os)]` by hand

The three OS layers come from file names. A file named `macos.rs`, `linux.rs`
or `windows.rs` (or a `macos/`, `linux/`, `windows/` directory) gets its
`#[cfg(target_os = ...)]` line from `scripts/gen-mod-tree.py`, which also writes
every `mod.rs` and every `mod` list in `lib.rs`.

- A platform seam in `rise-platform` (and the same in `rise-media`) is a
  directory `src/<seam>/`: `<seam>.rs` holds the trait and the shared policy,
  `macos.rs`, `linux.rs`, `windows.rs` hold only the system calls.
- Feature code and stores contain no `cfg(target_os)` and no `macos.rs`: they
  call the seam and read `PlatformSupport`.
- `mod.rs` files are generated, hand-written code never goes there.

Note that gpui itself is not uniform: on Windows `hide_other_apps` and
`unhide_other_apps` are live `unimplemented!()` calls, and `start_external_drag`
does not exist. Go through `rise_platform::gpui_shim`, never gpui directly.

## 3. Calls between modules follow iOS

The whole application (`modules`, `app`, and every `core` file that references
a feature) lives in one crate, `riseonly` (`app/`), the way iOS is one target. Inside it modules call each other directly,
stores included, exactly where iOS does; stores may use `RiseEngine` where the
iOS store does.

A `Hooks` slot exists only in two cases:

- a kit crate calls a feature (`<Lower>Hooks` in the kit crate, installed by
  `riseonly` at start);
- an early phase calls a later one (F1-F3 calls a store, engine or page of
  F4-F5): `<lower>_hooks.rs` next to the caller, installed by the feature's
  `<topic>_hooks_install.rs`, indexed by the generated
  `install_feature_hooks()`.

No slot exists outside these two cases.

## Where code lives

- Kit crates hold the iOS `Core` files that reference no feature: `core/ui` in
  `rise-ui`, `core/widgets` in `rise-widgets`, `Media` in `rise-media`, `Theme`
  in `rise-theme` and so on, with per-file exceptions from the graph in
  `scripts/crate-map.json`. `riseonly` re-exports the kit, so the path
  `crate::core::...` is the same everywhere.
- Every path mirrors iOS: `modules/chat/engine/messages/rise_message_realtime.rs`
  is the twin of `modules/chat/engine/messages/RiseMessageRealtime.swift`.
- Account isolation is 1 to 1 with iOS: the members that iOS resets from
  `AccountStateResetCoordinator` are ported in the same order. There is no per-account root entity dropped whole.
- A new crate, a dependency between crates or a new external crate needs a row
  in `DEPENDENCIES.md` and a pinned version.

## Sheets are modals on the desktop

This is a deliberate difference from iOS. Every sheet of the kit
(`NativeBottomSheetPresentationHost` and `BottomSheetFullPageUi` without `full_screen_cover`) shows
as a dialog in the middle of the window: dim backdrop, all corners rounded, no grabber, no drag, no
stops. It closes by Escape, by a backdrop click where the iOS sheet closed by a swipe, and by the
button in its header. Tab stays inside. Full-screen covers are unchanged. Screens do not change for
this: the frame, the outline and the keyboard rules live in
`crates/rise-ui/src/bottom_sheet_ui/modal_sheet_surface.rs`.
