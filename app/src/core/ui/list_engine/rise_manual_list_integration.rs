use std::any::TypeId;
use std::collections::HashMap;

use gpui::{AnyView, App, Global, Pixels, Point, Window};

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListBottomStickyHeaderInteractive, RiseManualListReusableView, RiseManualListSelectionPresentable,
    RiseManualListTopStickyHeaderFading, RiseManualListTopStickyHeaderInteractive,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListMountEnvelopePolicy;

/// main thread cost class of installing a row
/// the deferred mount drain installs at most MAXIMUM_FALLBACK_HOSTED_MOUNTS_PER_IDLE_BATCH hosted
/// fallback rows per batch
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseManualListInstallCost {
    Native,
    HostedFallback,
}

impl RiseManualListInstallCost {
    /// cost of an item that doesnt state one, the chat reuse identifier rule
    pub fn default_cost(reuse_identifier: &str) -> RiseManualListInstallCost {
        if RiseManualListMountEnvelopePolicy::is_fallback_hosted_row(reuse_identifier) {
            RiseManualListInstallCost::HostedFallback
        } else {
            RiseManualListInstallCost::Native
        }
    }
}

/// a mounted row that gates animated media on viewport visibility
/// the engine calls it on mount, configure and frame updates
/// when the visible range changes and with false on unmount
pub trait RiseManualListViewportVisibilityReceiving {
    fn set_animated_media_visible_in_viewport(&mut self, is_visible: bool, cx: &mut App);
}

/// a mounted row whose natural height prepare_visible_viewport checks against the height it was
/// inserted with
pub trait RiseManualListPreparedHeightValidating {
    fn needs_prepared_viewport_height_validation(&self, cx: &App) -> bool;

    fn prepared_viewport_natural_height_mismatch(&mut self, cx: &mut App) -> Option<Pixels>;
}

/// casts a mounted row to a protocol, a row type registers each seam it conforms to once
/// idempotent and cheap enough for its constructor
/// the engine looks seams up by entity type and treats an unregistered row as non conforming
#[derive(Default)]
pub struct RiseManualListConformances {
    by_type: HashMap<TypeId, RiseManualListConformance>,
}

impl Global for RiseManualListConformances {}

type TopStickyHeaderFadingSeam = (fn(&AnyView, bool, bool, &mut App), fn(&AnyView, &App) -> bool);
type TopStickyHeaderInteractiveSeam =
    (fn(&AnyView, Point<Pixels>, &App) -> bool, fn(&AnyView, bool, &mut Window, &mut App));
type PreparedHeightValidatingSeam = (fn(&AnyView, &App) -> bool, fn(&AnyView, &mut App) -> Option<Pixels>);

#[derive(Clone, Copy, Default)]
struct RiseManualListConformance {
    viewport_visibility_receiving: Option<fn(&AnyView, bool, &mut App)>,
    prepared_height_validating: Option<PreparedHeightValidatingSeam>,
    selection_presentable: Option<fn(&AnyView, bool, f32, &mut App)>,
    reusable_view: Option<fn(&AnyView, &mut App)>,
    top_sticky_header_fading: Option<TopStickyHeaderFadingSeam>,
    top_sticky_header_interactive: Option<TopStickyHeaderInteractiveSeam>,
    bottom_sticky_header_interactive: Option<fn(&AnyView, &mut Window, &mut App)>,
    cached_row: bool,
    replays_moved_row: bool,
}

impl RiseManualListConformances {
    fn entry<T: 'static>(cx: &mut App) -> &mut RiseManualListConformance {
        cx.default_global::<Self>().by_type.entry(TypeId::of::<T>()).or_default()
    }

    fn lookup(view: &AnyView, cx: &App) -> Option<RiseManualListConformance> {
        cx.try_global::<Self>()?.by_type.get(&view.entity_type()).copied()
    }

    pub fn register_viewport_visibility_receiving<T: RiseManualListViewportVisibilityReceiving + 'static>(
        cx: &mut App,
    ) {
        fn seam<T: RiseManualListViewportVisibilityReceiving + 'static>(view: &AnyView, is_visible: bool, cx: &mut App) {
            if let Ok(entity) = view.clone().downcast::<T>() {
                entity.update(cx, |row, cx| row.set_animated_media_visible_in_viewport(is_visible, cx));
            }
        }
        Self::entry::<T>(cx).viewport_visibility_receiving = Some(seam::<T>);
    }

    pub fn register_prepared_height_validating<T: RiseManualListPreparedHeightValidating + 'static>(cx: &mut App) {
        fn needs<T: RiseManualListPreparedHeightValidating + 'static>(view: &AnyView, cx: &App) -> bool {
            view.clone()
                .downcast::<T>()
                .is_ok_and(|entity| entity.read(cx).needs_prepared_viewport_height_validation(cx))
        }
        fn mismatch<T: RiseManualListPreparedHeightValidating + 'static>(
            view: &AnyView,
            cx: &mut App,
        ) -> Option<Pixels> {
            let entity = view.clone().downcast::<T>().ok()?;
            entity.update(cx, |row, cx| row.prepared_viewport_natural_height_mismatch(cx))
        }
        Self::entry::<T>(cx).prepared_height_validating = Some((needs::<T>, mismatch::<T>));
    }

    pub fn register_selection_presentable<T: RiseManualListSelectionPresentable>(cx: &mut App) {
        fn seam<T: RiseManualListSelectionPresentable>(
            view: &AnyView,
            is_active: bool,
            content_translation_x: f32,
            cx: &mut App,
        ) {
            if let Ok(entity) = view.clone().downcast::<T>() {
                entity.update(cx, |row, cx| {
                    row.apply_manual_list_selection_presentation(is_active, content_translation_x, cx)
                });
            }
        }
        Self::entry::<T>(cx).selection_presentable = Some(seam::<T>);
    }

    pub fn register_cached_row<T: 'static>(cx: &mut App) {
        Self::entry::<T>(cx).cached_row = true;
    }

    pub(crate) fn caches_row(view: &AnyView, cx: &App) -> bool {
        Self::lookup(view, cx).is_some_and(|conformance| conformance.cached_row)
    }

    /// invariant: same as register_cached_row and nothing the row draws reads its window position
    /// except the mouse and on_replayed_move callbacks, so a scroll replays the row moved
    pub fn register_moved_replay_row<T: 'static>(cx: &mut App) {
        let entry = Self::entry::<T>(cx);
        entry.cached_row = true;
        entry.replays_moved_row = true;
    }

    pub(crate) fn replays_moved_row(view: &AnyView, cx: &App) -> bool {
        Self::lookup(view, cx).is_some_and(|conformance| conformance.replays_moved_row)
    }

    pub fn register_reusable_view<T: RiseManualListReusableView>(cx: &mut App) {
        fn seam<T: RiseManualListReusableView>(view: &AnyView, cx: &mut App) {
            if let Ok(entity) = view.clone().downcast::<T>() {
                entity.update(cx, |row, cx| row.prepare_for_manual_list_reuse(cx));
            }
        }
        Self::entry::<T>(cx).reusable_view = Some(seam::<T>);
    }

    pub fn register_top_sticky_header_fading<T: RiseManualListTopStickyHeaderFading>(cx: &mut App) {
        fn set_faded<T: RiseManualListTopStickyHeaderFading>(view: &AnyView, faded: bool, animated: bool, cx: &mut App) {
            if let Ok(entity) = view.clone().downcast::<T>() {
                entity.update(cx, |row, cx| row.set_top_sticky_header_faded(faded, animated, cx));
            }
        }
        fn on_screen<T: RiseManualListTopStickyHeaderFading>(view: &AnyView, cx: &App) -> bool {
            view.clone()
                .downcast::<T>()
                .is_ok_and(|entity| entity.read(cx).is_top_sticky_header_on_screen())
        }
        Self::entry::<T>(cx).top_sticky_header_fading = Some((set_faded::<T>, on_screen::<T>));
    }

    pub fn register_top_sticky_header_interactive<T: RiseManualListTopStickyHeaderInteractive>(cx: &mut App) {
        fn contains<T: RiseManualListTopStickyHeaderInteractive>(view: &AnyView, point: Point<Pixels>, cx: &App) -> bool {
            view.clone()
                .downcast::<T>()
                .is_ok_and(|entity| entity.read(cx).top_sticky_header_contains(point))
        }
        fn activate<T: RiseManualListTopStickyHeaderInteractive>(
            view: &AnyView,
            is_pinned: bool,
            window: &mut Window,
            cx: &mut App,
        ) {
            if let Ok(entity) = view.clone().downcast::<T>() {
                entity.update(cx, |row, cx| row.activate_top_sticky_header(is_pinned, window, cx));
            }
        }
        Self::entry::<T>(cx).top_sticky_header_interactive = Some((contains::<T>, activate::<T>));
    }

    pub fn register_bottom_sticky_header_interactive<T: RiseManualListBottomStickyHeaderInteractive>(cx: &mut App) {
        fn activate<T: RiseManualListBottomStickyHeaderInteractive>(view: &AnyView, window: &mut Window, cx: &mut App) {
            if let Ok(entity) = view.clone().downcast::<T>() {
                entity.update(cx, |row, cx| row.activate_bottom_sticky_header(window, cx));
            }
        }
        Self::entry::<T>(cx).bottom_sticky_header_interactive = Some(activate::<T>);
    }

    pub(crate) fn set_animated_media_visible_in_viewport(view: &AnyView, is_visible: bool, cx: &mut App) -> bool {
        let Some(seam) = Self::lookup(view, cx).and_then(|conformance| conformance.viewport_visibility_receiving)
        else {
            return false;
        };
        seam(view, is_visible, cx);
        true
    }

    pub(crate) fn needs_prepared_viewport_height_validation(view: &AnyView, cx: &App) -> Option<bool> {
        let (needs, _) = Self::lookup(view, cx)?.prepared_height_validating?;
        Some(needs(view, cx))
    }

    pub(crate) fn prepared_viewport_natural_height_mismatch(view: &AnyView, cx: &mut App) -> Option<Pixels> {
        let (_, mismatch) = Self::lookup(view, cx)?.prepared_height_validating?;
        mismatch(view, cx)
    }

    pub(crate) fn apply_manual_list_selection_presentation(
        view: &AnyView,
        is_active: bool,
        content_translation_x: f32,
        cx: &mut App,
    ) {
        if let Some(seam) = Self::lookup(view, cx).and_then(|conformance| conformance.selection_presentable) {
            seam(view, is_active, content_translation_x, cx);
        }
    }

    pub(crate) fn prepare_for_manual_list_reuse(view: &AnyView, cx: &mut App) {
        if let Some(seam) = Self::lookup(view, cx).and_then(|conformance| conformance.reusable_view) {
            seam(view, cx);
        }
    }

    pub(crate) fn is_top_sticky_header_fading(view: &AnyView, cx: &App) -> bool {
        Self::lookup(view, cx).is_some_and(|conformance| conformance.top_sticky_header_fading.is_some())
    }

    pub(crate) fn set_top_sticky_header_faded(view: &AnyView, faded: bool, animated: bool, cx: &mut App) {
        if let Some((set_faded, _)) = Self::lookup(view, cx).and_then(|conformance| conformance.top_sticky_header_fading)
        {
            set_faded(view, faded, animated, cx);
        }
    }

    pub(crate) fn is_top_sticky_header_on_screen(view: &AnyView, cx: &App) -> Option<bool> {
        let (_, on_screen) = Self::lookup(view, cx)?.top_sticky_header_fading?;
        Some(on_screen(view, cx))
    }

    pub(crate) fn top_sticky_header_contains(view: &AnyView, point: Point<Pixels>, cx: &App) -> Option<bool> {
        let (contains, _) = Self::lookup(view, cx)?.top_sticky_header_interactive?;
        Some(contains(view, point, cx))
    }

    pub(crate) fn activate_top_sticky_header(
        view: &AnyView,
        is_pinned: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some((_, activate)) =
            Self::lookup(view, cx).and_then(|conformance| conformance.top_sticky_header_interactive)
        else {
            return false;
        };
        activate(view, is_pinned, window, cx);
        true
    }

    pub(crate) fn activate_bottom_sticky_header(view: &AnyView, window: &mut Window, cx: &mut App) -> bool {
        let Some(activate) = Self::lookup(view, cx).and_then(|conformance| conformance.bottom_sticky_header_interactive)
        else {
            return false;
        };
        activate(view, window, cx);
        true
    }
}
