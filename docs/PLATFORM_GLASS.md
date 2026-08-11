# Materials: Liquid Glass, vibrancy, and the honest fallback

The product target is the mobile design, one to one, in a modern macOS desktop
shell: a vertical rail on the left the way Telegram for macOS does it, and
Liquid Glass wherever a real macOS app would use it.

This document exists because that requirement collides with how GPUI draws, and
the collision has exactly one correct resolution.

> ## Currently switched off, on purpose
>
> **The shipping shell uses none of this.** The app is one OPAQUE slab filling
> its window edge to edge — see "Shell shape" at the bottom.
>
> The mac window is nevertheless opened `WindowBackgroundAppearance::Transparent`,
> and that is not a material: it is the window's CORNER. The corner is cut out of
> the window by `window_chrome::round_window_corner`, and a window that has
> declared itself opaque has nowhere to cut it out of — Apple's rule for
> `isOpaque` is that a window with rounded corners sets it `NO`. The alpha reaches
> those four corners and nothing else, because the slab paints every other pixel.
>
> A native region is an AppKit view *below* the Metal layer, and it samples what
> is behind the WINDOW. Put an opaque app in front of one and it stops being a
> material: it becomes a hole punched through the app to the wallpaper. The slab
> is that opaque app whatever the window declares, so `Material::ceiling()`
> returns `Painted` for all three materials, and every surface takes the tier-2
> fallback on every machine.
>
> Nothing below is deleted, and none of it is dead in the sense of being
> unreachable. The version probe, the Swift target, the layout gate and the
> region bookkeeping all still work, and raising a ceiling is the whole change
> needed to switch a surface back. Read the rest as the design of a subsystem
> that is loaded and idle.

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
baseline for the app background and costs nothing to adopt. *(Not what ships:
`preferred_window_material` asks for `Transparent`, which is plain alpha for the
corner and puts nothing behind the layer — see the note at the top. Raising it to
`Blurred` is a one-word change, but a material behind an opaque slab is invisible,
so the surfaces above it would have to stop painting first.)*

**Tier 1 — real glass regions.** For the left rail, the sidebar, the composer
bar, sheets and popovers: `NSGlassEffectView` instances hosted as sibling views
below the Metal layer, with GPUI leaving those rectangles transparent and the
platform layer keeping the views' frames in sync with the layout.

A region at the window's edge is clipped by the outer `NSWindow` mask in
`round_window_corner`, even though it sits below the Metal layer. It still keeps
its own radius for edges inside the window, but it cannot square off the outer
window corner.

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
  ╭──────────────────────────────────────────────────────────╮
  │ ╭──────╮ list / sidebar  │ content          │ aside       │
  │ │ rail │ 320pt           │ flexible         │ 300pt       │
  │ │ 90pt │                 │                  │             │
  │ │posts │ chats           │ conversation     │ profile     │
  │ │chats │ folders         │ feed             │ details     │
  │ │search│ search results  │ post             │             │
  │ ╰──────╯                                                 │
  ╰──────────────────────────────────────────────────────────╯
   ↑ only the rail is inset 5pt. Content remains edge-to-edge; its columns are
     flush and a `column_divider` hairline separates them.
```

The rail carries the sections the phone puts in a tab bar, plus the ones a
desktop has room for. Folders stack under the section icons the way Telegram for
macOS does.

**The content is one slab; the rail is the one inset surface.** Telegram for
macOS is the reference for its 5pt breathing room, 90pt width and native traffic
lights. Selection keeps Riseonly's product-primary red glyph and label, with no
hover box. The rail resolves the window radius once when the shell is created,
then subtracts its 5pt inset so both corner arcs stay parallel. No global content
padding follows from that: lists, media and future scrolling content remain free
to reach the window edge. The rejected design was one where every surface
floated as its own panel; that still reads as translucent patchwork and must not
return.

**THE CORNER IS THE WINDOW'S, AND GPUI DOES NOT GIVE IT TO US.** This was
believed for a long time and it is false: gpui opens a `NSTitledWindowMask`
window, but AppKit's rounding never reaches the single Metal layer the whole app
renders into. Put the app beside System Settings and ours is the square one. The
corner has to be cut by hand, and `window_chrome::round_window_corner` cuts it in
two places. `NSWindow`'s own corner mask shapes the WindowServer surface, hit
region and shadow; the `CAMetalLayer` mask clips gpui's compositor surface to
exactly the same curve. Applying only the Metal mask is the deceptive failure
shown by the first implementation: the black background looks round, but the
native window keeps its smaller contour. Setting `cornerRadius` on
`NSThemeFrame.layer` is also insufficient because AppKit owns that layer and
restores it during layout. The bridge therefore changes the window mask and the
Metal mask together; the measurements are recorded in `macos/window_corner.rs`.

The traffic lights remain inside the window mask and are far from its corner edge.

Two things make that mask work, and both are easy to undo by accident:

1. **The window must not be opaque.** A rounded corner is a piece of the window
   cut away, and an opaque window has nowhere to cut it out of — asking gpui for
   `WindowBackgroundAppearance::Opaque` sets `-[NSWindow setOpaque:YES]` and the
   corners come back filled whatever the mask says. `preferred_window_material`
   answers `Transparent` on macOS for this and nothing else. On the other two it
   stays `Opaque`: DWM rounds server-side, and the Linux corner comes out of
   `DecorationMode::ClientSide`, where it is ours to paint anyway.
2. **The radius is a measured table, not a guess and not a token.**
   `macos_window_corner` holds a measured 26.5pt continuous curve on macOS 26 and
   a 10pt circular curve before it. The 26.5pt value is fitted against the
   half-coverage edge of a System Settings window captured on the same 1x
   display; rows 2 through 24 differ by at most 0.2 physical pixels. The private
   `-[NSThemeFrame cornerRadius]` answer is 16pt and does not match that shape.
   There is no public AppKit setter for the WindowServer mask, so the bridge
   intentionally sends private `-[NSWindow _setCornerRadius:]` and reads back
   `-[NSWindow _cornerRadius]`. That makes this direct-distribution code, not a
   Mac App Store-safe implementation, and the selector must be rechecked on each
   major macOS release. Do not add a theme token for it: a design-system number
   would drift from the OS, and the point is to match the OS exactly.

An earlier attempt inset the ENTIRE application as a rounded slab with a 5pt
ring around it. That is still the wrong shape of solution: future lists and media
must be able to reach the window edge, and the shadow must trace the actual
window. The rail's local inset does not replace the native mask; AppKit still
owns the outer silhouette and shadow.

One rule survives from that attempt, for a different reason than it had:
**only the outermost element paints a background.** GPUI's `ContentMask` is a
plain rectangle — `overflow_hidden` does not clip to a corner radius — so a fill
inside a rounded element never gets rounded by it. AppKit saves the corners here
regardless, but a column repainting the app's own colour is pure overdraw, which
is why `BlockUi::column`, `ScreenShellUi` and `BoxUi::screen` all paint nothing
and the window buttons sit in a rail that has no fill.

`RootTab` plus `PanePolicy` model the columns — the rail is a presentation of
`RootTab::ALL`, and the columns are the pane slots.
