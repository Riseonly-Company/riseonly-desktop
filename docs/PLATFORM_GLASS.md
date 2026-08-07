# Materials: Liquid Glass, vibrancy, and the honest fallback

The product target is the mobile design, one to one, in a modern macOS desktop
shell: a vertical rail on the left the way Telegram for macOS does it, and
Liquid Glass wherever a real macOS app would use it.

This document exists because that requirement collides with how GPUI draws, and
the collision has exactly one correct resolution.

## Why this cannot be painted

GPUI renders the whole window into one Metal layer. It has no backdrop filter,
no way to sample what is behind the window, and every backend is 8-bit BGRA with
no HDR or wide-gamut path. A shader can imitate a blurred panel, but it cannot
sample the desktop wallpaper, the window underneath, or the content scrolling
behind a sidebar — which is the entire point of the material.

Real Liquid Glass is `NSGlassEffectView`, macOS 26 and later. Real vibrancy is
`NSVisualEffectView`. Both are AppKit views. To get either, actual AppKit views
must exist in the window, which means native code beside the Rust.

## Three tiers, chosen per surface

**Tier 0 — the window.** Already free. `gpui_macos` ships an `NSVisualEffectView`
subclass and honours `WindowBackgroundAppearance::Blurred`. Setting it gives the
whole window a real system material behind a transparent Metal layer. This is the
baseline for the app background and costs nothing to adopt.

**Tier 1 — real glass regions.** For the left rail, the sidebar, the composer
bar, sheets and popovers: `NSGlassEffectView` instances hosted as sibling views
below the Metal layer, with GPUI leaving those rectangles transparent and the
platform layer keeping the views' frames in sync with the layout.

This needs a small Swift target — `RiseGlass` — exposing a C ABI, compiled by
`build.rs` and linked into `rise-platform` on macOS only. Swift because
`NSGlassEffectView` and its configuration API are Swift-first and change shape
between OS versions; a thin Swift file tracks that far more cheaply than
hand-rolled `objc2` message sends.

**Tier 2 — the fallback.** macOS 13 through 25, and Linux and Windows always: a
painted material from theme tokens. Translucent fill, a hairline border, a soft
inner highlight. It reads as a deliberate flat design rather than a broken
attempt at glass.

## The rule that keeps the other two platforms from rotting

**No component ever asks for glass.** A component asks the theme for a
*material* — `Material::Chrome`, `Material::Panel`, `Material::Overlay` — and the
platform decides what that material actually is on this machine and this OS
version.

```
    component  ->  Material::Chrome
                        |
    rise-platform decides
                        |
      macOS 26+   ->  NSGlassEffectView region
      macOS 13-25 ->  NSVisualEffectView region
      Linux/Win   ->  painted token fallback
```

If a component could ask for glass directly, every such call site would become a
hole on Linux, and nobody would notice until a Linux user opened a screenshot.
Routing through a material keeps the fallback a property of the design system
rather than an afterthought at each call site.

## Where the code lives

```
crates/rise-theme/src/tokens/material.rs   the Material enum and its painted form
crates/rise-platform/src/materials.rs      the policy and the types: Material,
                                           MaterialBacking, resolve(), RegionId,
                                           RegionRect, GlassRegion, LayoutGate,
                                           GlassLayout, the GlassSurface trait,
                                           and current_glass_surface()
crates/rise-platform/src/macos/glass/mod.rs        MacGlassSurface: the extern "C"
                                           declarations and the policy around them
crates/rise-platform/src/macos/glass/RiseGlass.swift   the AppKit views themselves
crates/rise-platform/build.rs              compiles the Swift into a static
                                           library, macOS only, and fails loudly
                                           with the fix when swiftc is missing
crates/rise-widgets/src/glass_panel/       the component that consumes a material
```

`rise-platform` is already the only crate allowed `#[cfg(target_os)]`, and this
fits there without a new exception.

The one entry point the application uses is
`rise_platform::materials::current_glass_surface()`, which returns
`Box<dyn GlassSurface>`: the real AppKit-hosting surface on a macOS that can host
one, and `painted_glass_surface()` — whose `commit` reports `Unsupported` so the
caller paints the material from theme tokens — everywhere else. The app never
names a platform and never learns which it got.

Two details of that surface are worth knowing before changing it:

- **The version is not the whole answer.** `MacOsVersion::backing()` says what the
  *running* OS could host; it cannot say whether the view actually came back, which
  no version number predicts. So `NSGlassEffectView` is resolved by
  `NSClassFromString` behind `#available`, never referenced as a type, and
  `rise_glass_probe_backing` reports what actually resolved. The surface caps the
  version policy with that probe, and the cap can only lower.
  It is *not* a rescue for a build made against an older SDK — that rationale was
  written here first and is false. The Swift compiles clean against the 13.3, 14
  and 15 SDKs, and the 15-SDK build run on macOS 26.1 reports Liquid Glass,
  because both `NSClassFromString` and `#available` resolve against the running
  OS. An old SDK is survivable because nothing names the class as a type.
- **The gate is the constraint, made mechanical.** `LayoutGate` admits a batch only
  when it carries a newer `Generation`. A render pass has none to offer, so a region
  animated frame by frame — the thing that desynchronises it from the content drawn
  around it — stops being expressible rather than merely discouraged.

## Constraints worth writing down before someone rediscovers them

A glass region is an AppKit view under a transparent hole in the Metal layer.
That means its rectangle must be **static for the duration of a frame** and
updated from the layout pass, not from a render pass. Animating a glass region's
frame per frame will desynchronise it from the content drawn around it.

Glass regions cannot overlap arbitrary GPUI content that should appear *behind*
them — the view is below the Metal layer, so anything GPUI draws in that
rectangle sits on top. Design so glass is background, never a mid-stack layer.

Nothing may escape the window bounds. GPUI has no native popups on X11 or
Windows, so a glass popover is an in-window overlay on every platform; on macOS
it happens to be a real material, elsewhere it is painted.

Do not ship glass behind scrolling text without checking legibility at both
appearances. Apple's own guidance is that Liquid Glass sits behind chrome, not
behind dense content, and the reference app follows the same instinct.

## Shell shape

The desktop shell is a left rail, not a bottom tab bar:

```
  rail        list / sidebar        content            aside
  56pt        320pt                 flexible           300pt
  ---------------------------------------------------------
  posts       chats                 conversation       profile
  chats       folders               feed               details
  search      search results        post
  vacancies
  profile
  settings
```

The rail carries the sections the phone puts in a tab bar, plus the ones a
desktop has room for. Folders stack under the section icons the way Telegram for
macOS does. The rail is `Material::Chrome`; the list is `Material::Panel`;
content is opaque.

This is the only part of the design that deliberately departs from the phone,
and `RootTab` plus `PanePolicy` already model it — the rail is a presentation of
`RootTab::ALL`, and the columns are the pane slots.
