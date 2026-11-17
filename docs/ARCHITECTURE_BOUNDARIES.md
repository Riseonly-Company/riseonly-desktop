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

## 3. One tree, several layer crates, calls up only through a slot

The whole application (`modules`, `app`, and every `core` file that references a feature) is one tree,
`app/src`, the way iOS is one target, and every path mirrors iOS. The tree is compiled by several crates
so that an edit rebuilds a small part of it. The files never move:

- `scripts/crate-views.json` says which crate compiles each file: rules by directory, then the files that
  differ from their rule. `riseonly` (`app/`) is the top crate; the others are `crates/riseonly-*`.
- A view crate is two generated files, `Cargo.toml` and `src/lib.rs`. The `lib.rs` names its files in
  `app/src` and re-exports the crates below at the same module paths, so `crate::modules::...` and
  `crate::core::...` are the same path in every crate and no source file knows which crate it is in.
- `scripts/gen-mod-tree.py --write` regenerates them, `--check` is the gate. There is no `mod.rs` in `app/src`.

Direction is the one rule. A file names only items of its own crate and of the crates below it
(`deps` in the map). Calls down are direct, stores included, exactly where iOS calls them. A call up goes
through a `Hooks` slot and nothing else:

- a kit crate calls a feature (`<Lower>Hooks` in the kit crate, installed by the app at start);
- a lower crate calls a higher one, or an early phase calls a later one: `<lower>_hooks.rs` next to the
  caller declares the slot (`FeatureHookSlot`), `<topic>_hooks_install.rs` next to the implementation
  fills it, and the generated `install_feature_hooks()` calls every install at start. The generated
  `feature_hook_slots()` lists every slot and a generated test fails when an install leaves one empty.

A type the lower crate needs is not a slot: its definition lives in a lower file and the file that used to
hold it keeps a `pub use`, so the old path still resolves.

A new file takes the crate of its directory rule. If it names something above that crate, either it
belongs higher (add it to `files` in the map) or the call needs a slot. An `impl Type { .. }` block lives
in the crate of `Type`; an extension that needs a higher crate is a trait in the higher file.

## Where code lives

- Kit crates hold the iOS `Core` files that reference no feature: `core/ui` in
  `rise-ui`, `core/widgets` in `rise-widgets`, `Media` in `rise-media`, `Theme`
  in `rise-theme` and so on, with per-file exceptions from the graph in
  `scripts/crate-map.json`. The app crates re-export the kit, so the path
  `crate::core::...` is the same everywhere. Kit files live in the kit's own
  directory; app files stay in `app/src` whatever crate compiles them (section 3).
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
