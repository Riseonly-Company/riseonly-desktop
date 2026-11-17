use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use gpui::{App, AsyncApp, Bounds, Entity, Pixels, Point, WeakEntity, point, px};
use rise_canon::{RiseGuideAnchor, RiseGuideCanon};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorBackingView;
use rise_ui::guide_anchor_ui::guide_anchor_registry::GuideAnchorRegistry;
use rise_ui::guide_anchor_ui::guide_scroll_host::{GuideScrollHost, WeakGuideScrollHost};
use rise_ui::guide_overlay_ui::guide_overlay_theme::GuideOverlayTheme;

use crate::core::guides::guide_screen_resolver::GuideScreenResolver;
use crate::core::widgets::wrappers::list_shell_header_collapse::ListShellHeaderCollapse;
use crate::core::widgets::navigations::main_tab_bar::MainTabBarReveal;

const MAX_SEARCH_PAGES: usize = 30;

thread_local! {
    static KNOWN_HOSTS: RefCell<HashMap<RiseGuideAnchor, Vec<WeakGuideScrollHost>>> =
        RefCell::new(HashMap::new());
    static CHROMES: RefCell<HashMap<RiseGuideAnchor, WeakEntity<ListShellHeaderCollapse>>> =
        RefCell::new(HashMap::new());
}

/// element is mounted but has no usable rect yet
struct NotLaidOutYet;

pub enum GuideAnchorScroller {}

impl GuideAnchorScroller {
    pub fn mount_window_ms() -> i64 {
        RiseGuideCanon::TIMINGS_ANCHOR_SETTLE_MS * 2
    }

    /// waits for the anchor to mount and then brings it into view
    /// returns whether anything moved, the trace line needs it to tell scrolled to it from already there
    pub async fn reveal(
        anchor: RiseGuideAnchor,
        waiting_for_mount_within: i64,
        animated: bool,
        cx: &AsyncApp,
    ) -> bool {
        let tab_bar = cx.update(|cx| {
            MainTabBarReveal::shared(cx).update(cx, |reveal, cx| reveal.reveal(anchor, animated, cx))
        });
        if let Some(moved) = tab_bar {
            return moved;
        }
        let chrome = CHROMES.with(|chromes| chromes.borrow().get(&anchor).and_then(WeakEntity::upgrade));
        if let Some(collapse) = chrome
            && cx.update(|cx| ListShellHeaderCollapse::reveal_folded(&collapse, cx))
        {
            return true;
        }
        let deadline = monotonic_ms() + waiting_for_mount_within.max(0);
        loop {
            if let Some(moved) = cx.update(|cx| Self::bring_into_view_if_mounted(anchor, animated, cx)) {
                return moved;
            }
            if monotonic_ms() >= deadline {
                break;
            }
            sleep_ms(GuideAnchorRegistry::follow_interval_ms(), cx).await;
        }
        if waiting_for_mount_within <= 0 || GuideAnchorRegistry::is_registered(anchor) {
            return false;
        }
        let Some(host) = cx.update(|cx| Self::search_host(anchor, cx)) else {
            return false;
        };
        Self::search(anchor, &host, animated, cx).await
    }

    /// remembers the scroll views that carry a mounted anchor while a guide runs
    /// a lazy list drops rows that scroll far away and a hidden achievement moves below the fold
    /// so the list it was in is where to look again
    pub fn note_mounted(anchor: RiseGuideAnchor, view: &GuideAnchorBackingView) {
        let chain = view.scroll_hosts();
        if chain.is_empty() {
            return;
        }
        let boxes: Vec<WeakGuideScrollHost> = chain.iter().map(GuideScrollHost::downgrade).collect();
        KNOWN_HOSTS.with(|known| {
            known.borrow_mut().insert(anchor, boxes);
        });
    }

    pub fn register_chrome(anchors: &[RiseGuideAnchor], collapse: &Entity<ListShellHeaderCollapse>) {
        let weak = collapse.downgrade();
        CHROMES.with(|chromes| {
            let mut chromes = chromes.borrow_mut();
            for anchor in anchors {
                chromes.insert(*anchor, weak.clone());
            }
        });
    }

    pub fn forget_hosts() {
        KNOWN_HOSTS.with(|known| known.borrow_mut().clear());
    }

    fn page_dwell_ms(animated: bool) -> i64 {
        if animated { 380 } else { 100 }
    }

    /// none while the anchor isnt in a window or not laid out yet
    fn bring_into_view_if_mounted(anchor: RiseGuideAnchor, animated: bool, cx: &mut App) -> Option<bool> {
        let view = GuideAnchorRegistry::anchor_view(anchor)?;
        view.window()?;
        let moved = Self::bring_into_view(&view, animated, cx).ok()?;
        Self::note_mounted(anchor, &view);
        Some(moved)
    }

    /// pages through scroll until its lazy content builds the anchor, then centers it
    /// down first since a fresh screen starts at the top, then up from where the reader was
    /// found nothing means the screen goes back to where it was
    async fn search(anchor: RiseGuideAnchor, scroll: &GuideScrollHost, animated: bool, cx: &AsyncApp) -> bool {
        let origin = scroll.content_offset();
        for direction in [1.0_f32, -1.0] {
            if direction < 0.0 {
                if Self::is_held(scroll) || !scroll.is_in_window() {
                    return false;
                }
                if (f32::from(scroll.content_offset().y) - f32::from(origin.y)).abs() > 0.5 {
                    cx.update(|cx| scroll.set_content_offset(origin, false, cx));
                }
            }
            let mut position = if direction < 0.0 {
                f32::from(origin.y)
            } else {
                f32::from(scroll.content_offset().y)
            };
            let mut pages = 0;
            while pages < MAX_SEARCH_PAGES {
                let Some(next) = Self::next_offset(scroll, position, direction) else {
                    break;
                };
                let x = scroll.content_offset().x;
                cx.update(|cx| scroll.set_content_offset(point(x, px(next)), animated, cx));
                position = next;
                pages += 1;
                if Self::wait_for_mount(anchor, Self::page_dwell_ms(animated), animated, cx).await {
                    return true;
                }
                if Self::is_held(scroll) || !scroll.is_in_window() {
                    return false;
                }
            }
        }
        if !Self::is_held(scroll) && scroll.is_in_window() {
            cx.update(|cx| scroll.set_content_offset(origin, animated, cx));
        }
        false
    }

    async fn wait_for_mount(anchor: RiseGuideAnchor, within_ms: i64, animated: bool, cx: &AsyncApp) -> bool {
        let deadline = monotonic_ms() + within_ms;
        loop {
            if cx.update(|cx| Self::bring_into_view_if_mounted(anchor, animated, cx)).is_some() {
                return true;
            }
            if monotonic_ms() >= deadline {
                return false;
            }
            sleep_ms(GuideAnchorRegistry::follow_interval_ms(), cx).await;
        }
    }

    /// where the next page starts, none at the edge
    /// most of a viewport so what was at the bottom stays in sight at the top
    /// and the reader sees the screen travel and not jump
    fn next_offset(scroll: &GuideScrollHost, position: f32, direction: f32) -> Option<f32> {
        let lower = 0.0_f32;
        let upper = lower.max(f32::from(scroll.max_content_offset().y));
        let height = scroll.frame().map_or(0.0, |frame| f32::from(frame.size.height));
        let page = (height * 0.8).max(80.0);
        let next = (position + direction * page).max(lower).min(upper);
        ((next - position).abs() > 0.5).then_some(next)
    }

    fn search_host(anchor: RiseGuideAnchor, cx: &mut App) -> Option<GuideScrollHost> {
        let known: Vec<GuideScrollHost> = KNOWN_HOSTS.with(|known| {
            known
                .borrow()
                .get(&anchor)
                .map(|hosts| hosts.iter().filter_map(WeakGuideScrollHost::upgrade).collect())
                .unwrap_or_default()
        });
        if let Some(host) = known.into_iter().find(|host| Self::is_searchable(host)) {
            return Some(host);
        }
        for neighbour in GuideAnchorRegistry::visible_neighbours(anchor) {
            if let Some(host) = neighbour.scroll_hosts().into_iter().find(|host| Self::is_searchable(host)) {
                return Some(host);
            }
        }
        let active = GuideScreenResolver::current_active_screen(cx)?;
        if active.raw_value() != anchor.screen() {
            return None;
        }
        Self::frontmost_list(cx)
    }

    fn is_searchable(scroll: &GuideScrollHost) -> bool {
        if !scroll.is_in_window() || !scroll.is_scroll_enabled() || Self::is_held(scroll) {
            return false;
        }
        let Some(frame) = scroll.frame() else { return false };
        if f32::from(frame.size.height) <= 1.0 {
            return false;
        }
        f32::from(scroll.max_content_offset().y) > 0.5
    }

    fn is_held(scroll: &GuideScrollHost) -> bool {
        scroll.is_tracking()
    }

    fn frontmost_list(cx: &mut App) -> Option<GuideScrollHost> {
        let window = cx.active_window().or_else(|| cx.windows().first().copied())?;
        let viewport = window.update(cx, |_, window, _| window.viewport_size()).ok()?;
        let window_bounds = Bounds { origin: Point::default(), size: viewport };

        let mut best: Option<GuideScrollHost> = None;
        let mut best_area = 0.0_f32;
        for scroll in GuideScrollHost::live() {
            if scroll.window() != Some(window) || !Self::is_searchable(&scroll) {
                continue;
            }
            let Some(laid_out) = scroll.frame() else { continue };
            let frame = laid_out.intersect(&window_bounds);
            let area = f32::from(frame.size.width) * f32::from(frame.size.height);
            if area <= 0.0 {
                continue;
            }
            if area <= best_area || !Self::is_uncovered(&scroll, frame) {
                continue;
            }
            best = Some(scroll);
            best_area = area;
        }
        best
    }

    fn is_uncovered(scroll: &GuideScrollHost, frame: Bounds<Pixels>) -> bool {
        scroll.answers_for(frame.center())
    }

    fn bring_into_view(view: &GuideAnchorBackingView, animated: bool, cx: &mut App) -> Result<bool, NotLaidOutYet> {
        view.window().ok_or(NotLaidOutYet)?;
        let mut target = view.window_frame().ok_or(NotLaidOutYet)?;

        let mut moved = false;
        for scroll in Self::scroll_ancestors(view) {
            let Some(delta) = Self::offset_delta(target, &scroll) else {
                continue;
            };
            let offset = scroll.content_offset();
            scroll.set_content_offset(
                point(offset.x + px(delta.0), offset.y + px(delta.1)),
                animated,
                cx,
            );
            target = Bounds {
                origin: point(target.origin.x - px(delta.0), target.origin.y - px(delta.1)),
                size: target.size,
            };
            moved = true;
        }
        Ok(moved)
    }

    fn scroll_ancestors(view: &GuideAnchorBackingView) -> Vec<GuideScrollHost> {
        view.scroll_hosts().into_iter().filter(Self::is_eligible).collect()
    }

    fn is_eligible(scroll: &GuideScrollHost) -> bool {
        if !scroll.is_scroll_enabled() {
            return false;
        }
        !scroll.is_tracking() && !scroll.is_decelerating()
    }

    fn offset_delta(target: Bounds<Pixels>, scroll: &GuideScrollHost) -> Option<(f32, f32)> {
        let visible = scroll.frame()?;
        let visible_width = f32::from(visible.size.width);
        let visible_height = f32::from(visible.size.height);
        if visible_width <= 1.0 || visible_height <= 1.0 {
            return None;
        }

        let clearance = GuideOverlayTheme::CUTOUT_PADDING + f32::from(GuideOverlayTheme::LAYOUT.cutout_clearance);
        let inset_x = clearance.min(visible_width / 4.0);
        let inset_y = clearance.min(visible_height / 4.0);
        let seen_min_x = f32::from(visible.origin.x);
        let seen_min_y = f32::from(visible.origin.y);
        let seen_max_x = seen_min_x + visible_width;
        let seen_max_y = seen_min_y + visible_height;

        let range = scroll.max_content_offset();
        let range_x = f32::from(range.x);
        let range_y = f32::from(range.y);
        let owns_horizontal_offset = scroll.is_scroll_enabled();

        let target_min_x = f32::from(target.origin.x);
        let target_min_y = f32::from(target.origin.y);
        let target_max_x = target_min_x + f32::from(target.size.width);
        let target_max_y = target_min_y + f32::from(target.size.height);

        let mut delta_x = if range_x > 0.0 && owns_horizontal_offset {
            axis_delta(
                target_min_x,
                target_max_x,
                seen_min_x + inset_x,
                seen_max_x - inset_x,
                seen_min_x,
                seen_max_x,
            )
        } else {
            0.0
        };
        let mut delta_y = if range_y > 0.0 {
            axis_delta(
                target_min_y,
                target_max_y,
                seen_min_y + inset_y,
                seen_max_y - inset_y,
                seen_min_y,
                seen_max_y,
            )
        } else {
            0.0
        };

        let offset = scroll.content_offset();
        delta_x = clamped_delta(delta_x, f32::from(offset.x), 0.0, range_x);
        delta_y = clamped_delta(delta_y, f32::from(offset.y), 0.0, range_y);

        (delta_x.abs() > 0.5 || delta_y.abs() > 0.5).then_some((delta_x, delta_y))
    }
}

/// how far the content has to move along one axis in the direction of a rising content offset
/// band is where the element should end up and seen is all the reader can see now
/// they differ by the clearance
fn axis_delta(
    lower: f32,
    upper: f32,
    band_lower: f32,
    band_upper: f32,
    seen_lower: f32,
    seen_upper: f32,
) -> f32 {
    if lower >= band_lower && upper <= band_upper {
        return 0.0;
    }
    if upper - lower >= band_upper - band_lower {
        return lower - band_lower;
    }
    if upper <= seen_lower || lower >= seen_upper {
        return (lower + upper) / 2.0 - (band_lower + band_upper) / 2.0;
    }
    if lower < band_lower { lower - band_lower } else { upper - band_upper }
}

fn clamped_delta(delta: f32, offset: f32, lower: f32, upper: f32) -> f32 {
    if delta == 0.0 {
        return 0.0;
    }
    let ceiling = lower.max(upper);
    (offset + delta).max(lower).min(ceiling) - offset
}

fn monotonic_ms() -> i64 {
    i64::try_from(RiseClock::monotonic().as_millis()).unwrap_or(i64::MAX)
}

async fn sleep_ms(ms: i64, cx: &AsyncApp) {
    cx.background_executor().timer(Duration::from_millis(ms.unsigned_abs())).await;
}
