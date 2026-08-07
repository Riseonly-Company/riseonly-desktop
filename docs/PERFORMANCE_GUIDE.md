# Performance contract, restated for desktop

The canon is `../../riseonly-ios/PERFORMANCE_GUIDE.md`. Read it in full. A screen
that shows correct data but drops frames is not finished. That does not change.

This document records only what desktop changes.

## Frame budget is no longer one number

The reference assumes 60 or 120 Hz. A desktop app meets 60, 120, 144 and 165 Hz,
sometimes across two monitors in the same session. Budget from the actual refresh
rate, and keep the same rule: during continuous motion the app's main-thread work
takes a fraction of the interval, not all of it.

## Gestures become pointer, wheel and keyboard

Section 8 of the canon — axis ownership, activation offset, predicted velocity,
pager settle — was written for a finger. Most of it does not transfer. What does
transfer is the principle underneath: motion moves one light layer, never the
whole tree, and a gesture always ends in a valid final state.

Windows precision-touchpad inertia comes through gpui's direct-manipulation path.

## Two new hazards that iOS did not have

**Overdraw.** gpui has long-standing open issues for overdrawing and for missing
damage and present regions. The cost is battery and compositor load rather than a
visible hitch, and it does not show up in a frame trace. Minimise permanently
animating elements; an animation forces layout recalculation and repaint.

**Media memory.** Never route video or sticker frames through a per-frame
`RenderImage`. There is a documented case of six static images costing 300 MB
until the CPU-side bytes stopped being retained after GPU upload, at which point
it became 12 MB. Multiply that by sixty frames per second and several streams.
One persistent texture per stream, written into. CI asserts an RSS ceiling while
scrolling a synthetic feed.

## Lists

gpui's `list` already provides what the reference had to build by hand:
SumTree-backed variable heights, `ListAlignment::Bottom`, and splice-based
prepend anchoring so loading older history does not move the viewport.

Sections 6.2 to 6.4 of the canon still apply on top of it: geometry prepared
before commit, a bounded mount budget, and a visual anchor preserved across
mutations.

## Pagination thresholds must stop being row counts

The reference triggers prefetch eight rows from the edge. That is blind to
viewport height and wrong on a 4K display, where eight rows may be a quarter of a
screen or a tenth of one. Every list uses the distance-based model from section 7.

## What tests can and cannot prove here

`TestAppContext` is cross-platform, so behavioural tests run everywhere.
Screenshot tests do not: `render_to_image` exists only on macOS and
`current_headless_renderer()` returns `None` elsewhere. Visual regressions on
Linux and Windows are caught by manual smoke checks in the release checklist, and
that limitation is stated rather than papered over.
