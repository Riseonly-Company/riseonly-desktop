use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    AnyView, App, AppContext, Context, Edges, Entity, EmptyView, Hsla, IntoElement, ParentElement, Pixels, Render, Styled,
    Window, div, px,
};
use gpui::FontWeight;
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::{ThemePalette, ThemeStore};
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT};

use crate::core::ui::list_engine::rise_manual_list_integration::{
    RiseManualListConformances, RiseManualListInstallCost, RiseManualListPreparedHeightValidating,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListItemProvider, RiseManualListItemViewFactory,
    RiseManualListReusableView, RiseManualListScrollMetrics, RiseManualListTransaction, RiseManualListTransactionAnchor,
    RiseManualListTransactionEdgeMutation, RiseManualListTransactionJumpAnimationDirection,
    RiseManualListTransactionUpdateAnimation,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;
use crate::modules::comment::components::native_list::comment_hosted_row_view::CommentHostedRowView;
use crate::modules::comment::components::native_list::comment_list_models::{
    CommentListInput, CommentListRow, CommentListRowKind, CommentRowMetrics,
};
use crate::modules::comment::components::native_list::comment_row_heights::{
    CommentRowHeightCache, CommentRowHeightEstimator, CommentRowHeightKey,
};

pub enum CommentListPolicy {}

impl CommentListPolicy {
    pub const EXACT_ROWS_ON_OPEN: usize = 8;
    pub const PREFETCH_VIEWPORTS: f32 = 1.5;
    pub const MINIMUM_PREFETCH_DISTANCE: f32 = 600.0;
    pub const AUDITED_NEIGHBOURS: usize = 12;
    pub const SCRATCH_AUDITED_NEIGHBOURS: usize = 24;
    pub const SCRATCH_AUDITS_PER_REST: usize = 4;

    pub fn should_load_more(distance_from_bottom: f32, viewport_height: f32, has_more: bool, is_loading_more: bool) -> bool {
        if !has_more || is_loading_more || viewport_height <= 1.0 {
            return false;
        }
        let threshold = Self::MINIMUM_PREFETCH_DISTANCE.max(viewport_height * Self::PREFETCH_VIEWPORTS);
        distance_from_bottom <= threshold
    }
}

struct CommentListControllerState {
    rows_by_id: RefCell<HashMap<String, CommentListRow>>,
    palette_epoch: Cell<i64>,
    applied_heights: RefCell<HashMap<String, f32>>,
}

/// comment list on the manual list engine
/// rows are values and heights are known before a row shows, pages land during inertia
/// without moving what you read
pub struct CommentListController {
    list_view: Entity<RiseManualListView>,
    background_color: Hsla,

    input: Option<CommentListInput>,
    state: Rc<CommentListControllerState>,
    applied_identity: Option<String>,
    applied_ids: Vec<String>,
    applied_content_versions: HashMap<String, i64>,
    applied_palette: Option<ThemePalette>,
    applied_expanded_ids: HashSet<String>,
    estimated_ids: HashSet<String>,
    applied_width: f32,
    scratch_host: Option<Entity<CommentHostedRowView>>,
    last_metrics: Option<RiseManualListScrollMetrics>,
    load_more_requested_for_count: Option<usize>,
    is_audit_scheduled: bool,
    is_short_list_settle_scheduled: bool,
    identity_applied_at: Option<Duration>,
    is_bottom_reveal_scheduled: bool,
    handled_focus_id: Option<String>,
    settling_focus: Option<(String, Duration)>,
    reported_missing_focus_for_count: Option<usize>,
    is_stopped: bool,
    needs_commit: bool,
}

impl CommentListController {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list_view = cx.new(|cx| RiseManualListView::new(window, cx));
        let this = cx.weak_entity();
        let width_target = this.clone();
        list_view.update(cx, |list, _| {
            list.rests_short_content_at_top = true;
            list.warms_mounted_rows = true;
            list.on_viewport_width_changed = Some(Rc::new(move |_width: Pixels, window: &mut Window, cx: &mut App| {
                let this = width_target.clone();
                window.defer(cx, move |_window, cx| {
                    this.update(cx, |controller, cx| controller.set_needs_commit(cx)).ok();
                });
            }));
            list.on_scroll_metrics_changed = Some(Rc::new(
                move |metrics: &RiseManualListScrollMetrics, window: &mut Window, cx: &mut App| {
                    let this = this.clone();
                    let metrics = metrics.clone();
                    window.defer(cx, move |window, cx| {
                        this.update(cx, |controller, cx| controller.scroll_metrics_did_change(metrics, window, cx))
                            .ok();
                    });
                },
            ));
        });
        Self {
            list_view,
            background_color: gpui::transparent_black(),
            input: None,
            state: Rc::new(CommentListControllerState {
                rows_by_id: RefCell::new(HashMap::new()),
                palette_epoch: Cell::new(0),
                applied_heights: RefCell::new(HashMap::new()),
            }),
            applied_identity: None,
            applied_ids: Vec::new(),
            applied_content_versions: HashMap::new(),
            applied_palette: None,
            applied_expanded_ids: HashSet::new(),
            estimated_ids: HashSet::new(),
            applied_width: 0.0,
            scratch_host: None,
            last_metrics: None,
            load_more_requested_for_count: None,
            is_audit_scheduled: false,
            is_short_list_settle_scheduled: false,
            identity_applied_at: None,
            is_bottom_reveal_scheduled: false,
            handled_focus_id: None,
            settling_focus: None,
            reported_missing_focus_for_count: None,
            is_stopped: false,
            needs_commit: false,
        }
    }

    pub fn submit(&mut self, input: CommentListInput, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        if self.background_color != input.background_color {
            self.background_color = input.background_color;
            cx.notify();
        }
        if self.applied_palette.as_ref() != Some(&input.palette) {
            if self.applied_palette.is_some() {
                self.state.palette_epoch.set(self.state.palette_epoch.get().wrapping_add(1));
            }
            self.applied_palette = Some(input.palette.clone());
        }
        let insets = Edges { top: px(input.top_inset), right: px(0.0), bottom: px(input.bottom_inset), left: px(0.0) };
        let scroll_enabled = !input.is_interaction_locked;
        let is_loading_more = input.is_loading_more;
        self.input = Some(input);
        let reveals_bottom = self.bottom_inset_growth_hides_end(insets.bottom, cx);
        self.list_view.update(cx, |list, cx| {
            if list.viewport_insets() != insets {
                list.set_viewport_insets(insets, window, cx);
            }
            if list.is_scroll_enabled() != scroll_enabled {
                list.set_is_scroll_enabled(scroll_enabled, cx);
            }
        });
        if !is_loading_more {
            self.load_more_requested_for_count = None;
        }
        self.commit_if_needed(window, cx);
        self.schedule_short_list_top_settle(window, cx);
        if reveals_bottom {
            self.schedule_bottom_reveal(window, cx);
        }
    }

    fn bottom_inset_growth_hides_end(&self, next_bottom: Pixels, cx: &App) -> bool {
        let settled = self
            .identity_applied_at
            .is_some_and(|applied_at| RiseClock::monotonic().saturating_sub(applied_at) >= Duration::from_millis(400));
        if !settled || self.applied_ids.is_empty() {
            return false;
        }
        let list = self.list_view.read(cx);
        let previous_bottom = list.viewport_insets().bottom;
        if f32::from(next_bottom) <= f32::from(previous_bottom) + 0.5 {
            return false;
        }
        let metrics = list.scroll_metrics();
        !metrics.is_tracking && !metrics.is_dragging && f32::from(metrics.distance_from_bottom) <= 2.0
    }

    fn schedule_bottom_reveal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_bottom_reveal_scheduled {
            return;
        }
        self.is_bottom_reveal_scheduled = true;
        cx.defer_in(window, |this, window, cx| {
            this.is_bottom_reveal_scheduled = false;
            if this.is_stopped || this.applied_ids.is_empty() {
                return;
            }
            let list = this.list_view.read(cx);
            let metrics = list.scroll_metrics();
            let insets = list.viewport_insets();
            let overflows = f32::from(metrics.content_height) + f32::from(insets.top) + f32::from(insets.bottom)
                > f32::from(metrics.viewport_size.height);
            if !overflows || metrics.is_tracking || metrics.is_dragging || list.is_scroll_in_progress() {
                return;
            }
            this.list_view.update(cx, |list, cx| {
                let _ = list.smart_scroll_to_bottom(true, window, cx);
            });
        });
    }

    fn schedule_short_list_top_settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_short_list_settle_scheduled {
            return;
        }
        self.is_short_list_settle_scheduled = true;
        cx.defer_in(window, |this, window, cx| {
            this.is_short_list_settle_scheduled = false;
            this.settle_short_list_at_top(window, cx);
        });
    }

    fn settle_short_list_at_top(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        let Some(first_id) = self.applied_ids.first().cloned() else { return };
        let list = self.list_view.read(cx);
        let metrics = list.scroll_metrics();
        if metrics.is_tracking
            || metrics.is_dragging
            || metrics.is_decelerating
            || list.is_scroll_in_progress()
            || list.is_programmatic_scroll_in_flight()
        {
            return;
        }
        let insets = list.viewport_insets();
        if f32::from(metrics.content_height) + f32::from(insets.top) + f32::from(insets.bottom)
            > f32::from(metrics.viewport_size.height)
        {
            return;
        }
        let Some(frame) = list.viewport_frame(&first_id) else { return };
        if (f32::from(frame.origin.y) - f32::from(insets.top)).abs() <= 1.0 {
            return;
        }
        self.list_view.update(cx, |list, cx| {
            let _ = list.smart_scroll_to_top(true, window, cx);
        });
    }

    pub fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_stopped = true;
        self.input = None;
        self.state.rows_by_id.borrow_mut().clear();
        self.scratch_host = None;
        self.list_view.update(cx, |list, cx| {
            list.on_viewport_width_changed = None;
            list.on_scroll_metrics_changed = None;
            list.stop(window, cx);
        });
    }

    fn set_needs_commit(&mut self, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        self.needs_commit = true;
        cx.notify();
    }

    fn commit_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.needs_commit = false;
        if self.is_stopped {
            return;
        }
        let Some(input) = self.input.clone() else { return };
        let width = f32::from(self.list_view.read(cx).bounds().size.width);
        if width <= 1.0 {
            return;
        }

        let scale = window.scale_factor();
        let ids: Vec<String> = input.rows.iter().map(|row| row.id.clone()).collect();
        let is_new_list = self.applied_identity.as_deref() != Some(input.identity.as_str());
        let width_changed = (self.applied_width - width).abs() > 0.5;
        let is_scrolling = self
            .last_metrics
            .as_ref()
            .map(|metrics| metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating)
            .unwrap_or(false);

        let mut heights: HashMap<String, f32> = HashMap::with_capacity(input.rows.len());
        let mut estimated: HashSet<String> = HashSet::new();
        let mut changed: HashSet<String> = HashSet::new();
        let mut exact_budget =
            if is_new_list || self.applied_ids.is_empty() { CommentListPolicy::EXACT_ROWS_ON_OPEN } else { 0 };
        let height_cache = CommentRowHeightCache::shared(cx);
        let expanded_ids: HashSet<String> =
            input.rows.iter().filter(|row| row.is_text_expanded).map(|row| row.id.clone()).collect();
        let toggled_ids: HashSet<String> = if is_new_list {
            HashSet::new()
        } else {
            let applied: HashSet<&String> = self.applied_ids.iter().collect();
            expanded_ids
                .symmetric_difference(&self.applied_expanded_ids)
                .filter(|id| applied.contains(id))
                .cloned()
                .collect()
        };

        for row in &input.rows {
            let key = CommentRowHeightKey::new(row, width, scale);
            let cached = height_cache.update(cx, |cache, _| cache.height(&key));
            let measured = if cached.is_none() && (row.is_accessory() || exact_budget > 0 || toggled_ids.contains(&row.id))
                && !is_scrolling
            {
                self.scratch_measure(row, width, &input.content, window, cx)
            } else {
                None
            };
            let height = if let Some(cached) = cached {
                cached
            } else if let Some(measured) = measured {
                height_cache.update(cx, |cache, _| cache.insert(measured, key.clone()));
                measured
            } else {
                estimated.insert(row.id.clone());
                Self::estimate(&self.state, row, width, scale, window)
            };
            if !row.is_accessory() && exact_budget > 0 {
                exact_budget -= 1;
            }
            if row.is_accessory() {
                estimated.insert(row.id.clone());
            }
            heights.insert(row.id.clone(), height);

            let render_version = Self::render_version(row, self.state.palette_epoch.get());
            let applied_height = self.state.applied_heights.borrow().get(&row.id).copied().unwrap_or(-1.0);
            if self.applied_content_versions.get(&row.id) != Some(&render_version)
                || (applied_height - height).abs() > 0.25
            {
                changed.insert(row.id.clone());
            }
        }
        self.release_scratch_content(cx);

        if !(is_new_list || width_changed || ids != self.applied_ids || !changed.is_empty()) {
            self.estimated_ids = estimated;
            self.resolve_scroll_requests(window, cx);
            return;
        }

        {
            let mut rows = HashMap::with_capacity(input.rows.len());
            for row in &input.rows {
                rows.insert(row.id.clone(), row.clone());
            }
            *self.state.rows_by_id.borrow_mut() = rows;
        }

        let retained: HashSet<String> = self.applied_ids.iter().cloned().collect();
        let updated: HashSet<String> = changed.intersection(&retained).cloned().collect();
        let mutation = if is_new_list {
            None
        } else {
            RiseManualListTransactionEdgeMutation::diff(&self.applied_ids, &ids)
        };
        let is_pure_suffix_insert = !is_new_list
            && !width_changed
            && updated.is_empty()
            && mutation.as_ref().is_some_and(|mutation| {
                mutation.removed_prefix_count == 0
                    && mutation.removed_suffix_count == 0
                    && mutation.inserted_prefix_count == 0
                    && mutation.inserted_suffix_count > 0
            });

        let pins_top = {
            let list = self.list_view.read(cx);
            !is_new_list
                && !is_scrolling
                && !list.is_scroll_in_progress()
                && mutation
                    .as_ref()
                    .is_some_and(|mutation| mutation.inserted_prefix_count > 0 || mutation.removed_prefix_count > 0)
                && f32::from(list.scroll_metrics().distance_from_top) < 1.0
        };

        let provider: RiseManualListItemProvider = {
            let state = Rc::downgrade(&self.state);
            let content = input.content.clone();
            let heights = heights.clone();
            let estimated = estimated.clone();
            let background = self.background_color;
            Rc::new(move |id: &str, provided_width: f32, window: &mut Window, _cx: &mut App| {
                Self::item(&state, id, provided_width, scale, &heights, &estimated, background, &content, window)
                    .unwrap_or_else(Self::empty_item)
            })
        };
        let update_animation = if toggled_ids.is_empty() || width_changed {
            None
        } else {
            let mut animation = RiseManualListTransactionUpdateAnimation::new(
                0.34,
                0.86,
                toggled_ids.clone(),
                HashSet::new(),
                HashSet::new(),
                HashSet::new(),
            );
            animation.reveals_growth_ids = toggled_ids.clone();
            animation.reveals_growth_band_background = Some(input.background_color);
            animation.reveals_growth_footer_height = CommentRowMetrics::VERTICAL_PADDING
                + CommentRowMetrics::ACTIONS_MIN_HEIGHT
                + CommentRowMetrics::ACTIONS_BOTTOM_PADDING;
            let toggles_sit_on_footer = {
                let rows = self.state.rows_by_id.borrow();
                toggled_ids.iter().all(|id| {
                    matches!(
                        rows.get(id).map(|row| &row.kind),
                        Some(CommentListRowKind::Comment(comment, _)) if comment.media.is_none()
                    )
                })
            };
            if toggles_sit_on_footer {
                let toggle_font = Font::system_weight(
                    (CommentRowMetrics::TEXT_TOGGLE_FONT_SIZE + ThemeStore::text_delta_snapshot()).max(1.0),
                    FontWeight::MEDIUM,
                );
                animation.reveals_growth_toggle_height =
                    ((toggle_font.size * NATURAL_LINE_HEIGHT).ceil() - 3.0 + CommentRowMetrics::BODY_SPACING).max(0.0);
            }
            Some(animation)
        };
        let transaction = RiseManualListTransaction::new(
            ids.clone(),
            if is_new_list || pins_top {
                RiseManualListTransactionAnchor::Top
            } else {
                RiseManualListTransactionAnchor::PreserveVisible
            },
            Some(width),
            if is_new_list || width_changed { None } else { Some(updated) },
            update_animation,
            None,
            mutation,
            is_pure_suffix_insert,
            HashSet::new(),
            provider,
        );

        if self.applied_identity.as_deref() != Some(input.identity.as_str()) {
            self.identity_applied_at = Some(RiseClock::monotonic());
        }
        self.applied_identity = Some(input.identity.clone());
        self.applied_ids = ids;
        self.applied_width = width;
        *self.state.applied_heights.borrow_mut() = heights;
        self.applied_expanded_ids = expanded_ids;
        let palette_epoch = self.state.palette_epoch.get();
        self.applied_content_versions =
            input.rows.iter().map(|row| (row.id.clone(), Self::render_version(row, palette_epoch))).collect();
        self.estimated_ids = estimated;
        if is_new_list {
            self.handled_focus_id = None;
            self.settling_focus = None;
            self.reported_missing_focus_for_count = None;
            self.load_more_requested_for_count = None;
        }

        let this = cx.weak_entity();
        let completion: Box<dyn FnOnce(&mut Window, &mut App)> = Box::new(move |window: &mut Window, cx: &mut App| {
            window.defer(cx, move |window, cx| {
                this.update(cx, |controller, cx| {
                    if controller.is_stopped {
                        return;
                    }
                    controller.resolve_scroll_requests(window, cx);
                    controller.schedule_audit(cx);
                    controller.evaluate_pagination(window, cx);
                })
                .ok();
            });
        });
        self.list_view.update(cx, |list, cx| list.enqueue(transaction, Some(completion), None, window, cx));
    }

    #[allow(clippy::too_many_arguments)]
    fn item(
        state: &Weak<CommentListControllerState>,
        id: &str,
        width: f32,
        scale: f32,
        heights: &HashMap<String, f32>,
        estimated: &HashSet<String>,
        background: Hsla,
        content: &Rc<dyn Fn(&CommentListRow, &mut Window, &mut App) -> AnyView>,
        window: &mut Window,
    ) -> Option<RiseManualListItem> {
        let strong = state.upgrade()?;
        let row = strong.rows_by_id.borrow().get(id).cloned()?;
        let key = CommentRowHeightKey::new(&row, width, scale);
        let height = heights.get(id).copied().unwrap_or_else(|| Self::estimate(&strong, &row, width, scale, window));
        let is_estimated = estimated.contains(id);
        let content_version = Self::item_version(&row, height, scale, strong.palette_epoch.get());

        let view: RiseManualListItemViewFactory = {
            let state = state.clone();
            let content = content.clone();
            let id = id.to_string();
            Rc::new(move |reusable: Option<AnyView>, window: &mut Window, cx: &mut App| -> AnyView {
                let view = reusable
                    .and_then(|reusable| reusable.downcast::<CommentHostedRowView>().ok())
                    .unwrap_or_else(|| cx.new(CommentHostedRowView::new));
                let strong = state.upgrade();
                let current = strong
                    .as_ref()
                    .and_then(|strong| strong.rows_by_id.borrow().get(&id).cloned())
                    .unwrap_or_else(|| row.clone());
                let render_version = strong
                    .as_ref()
                    .map(|strong| Self::render_version(&current, strong.palette_epoch.get()))
                    .unwrap_or(current.content_version);
                view.update(cx, |view, cx| {
                    view.configure(&current, render_version, key.clone(), height, is_estimated, &content, window, cx)
                });
                view.into()
            })
        };

        Some(RiseManualListItem::new(
            height,
            CommentHostedRowView::REUSE_IDENTIFIER.to_string(),
            content_version,
            RiseManualListItemMountPolicy::DeferredDuringFastScroll,
            None,
            None,
            Some(Rc::new(move |_window: &mut Window, cx: &mut App| -> AnyView {
                RiseManualListConformances::register_moved_replay_row::<CommentListPlaceholderView>(cx);
                cx.new(|_| CommentListPlaceholderView { background }).into()
            })),
            Some(RiseManualListInstallCost::HostedFallback),
            view,
        ))
    }

    fn empty_item() -> RiseManualListItem {
        RiseManualListItem::new(
            1.0,
            "default".to_string(),
            0,
            RiseManualListItemMountPolicy::Immediate,
            None,
            None,
            None,
            None,
            Rc::new(|reusable: Option<AnyView>, _window: &mut Window, cx: &mut App| -> AnyView {
                reusable.unwrap_or_else(|| cx.new(|_| EmptyView).into())
            }),
        )
    }

    fn estimate(state: &CommentListControllerState, row: &CommentListRow, width: f32, scale: f32, window: &Window) -> f32 {
        match &row.kind {
            CommentListRowKind::Comment(comment, placement) => {
                CommentRowHeightEstimator::height(comment, *placement, row.is_text_expanded, width, scale, window)
            }
            CommentListRowKind::Accessory(_) => state.applied_heights.borrow().get(&row.id).copied().unwrap_or(120.0),
        }
    }

    fn render_version(row: &CommentListRow, palette_epoch: i64) -> i64 {
        if palette_epoch == 0 {
            return row.content_version;
        }
        let mut hasher = DefaultHasher::new();
        row.content_version.hash(&mut hasher);
        palette_epoch.hash(&mut hasher);
        hasher.finish() as i64
    }

    /// list item version, the drawn content plus its committed height in pixels
    /// a height only change just repins the row
    /// the hosted row reinstalls only when the render version changes
    fn item_version(row: &CommentListRow, height: f32, scale: f32, palette_epoch: i64) -> i64 {
        let mut hasher = DefaultHasher::new();
        Self::render_version(row, palette_epoch).hash(&mut hasher);
        ((height * scale.max(1.0)).round() as i64).hash(&mut hasher);
        hasher.finish() as i64
    }

    fn scratch_measure(
        &mut self,
        row: &CommentListRow,
        width: f32,
        content: &Rc<dyn Fn(&CommentListRow, &mut Window, &mut App) -> AnyView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<f32> {
        let host = match &self.scratch_host {
            Some(host) => host.clone(),
            None => cx.new(CommentHostedRowView::new),
        };
        self.scratch_host = Some(host.clone());
        let key = CommentRowHeightKey::new(row, width, window.scale_factor());
        let render_version = Self::render_version(row, self.state.palette_epoch.get());
        host.update(cx, |host, cx| {
            host.configure(row, render_version, key, 64.0, false, content, window, cx);
            host.measure(width, window, cx)
        })
    }

    /// drops the measured content like images, players and observers and keeps the host
    fn release_scratch_content(&mut self, cx: &mut Context<Self>) {
        let Some(scratch_host) = self.scratch_host.clone() else { return };
        if scratch_host.read(cx).row_id().is_none() {
            return;
        }
        scratch_host.update(cx, |host, cx| host.prepare_for_manual_list_reuse(cx));
    }

    const FOCUS_SETTLE_WINDOW: Duration = Duration::from_millis(1500);

    /// brings the focused comment back to the middle if a commit after the jump moved it out of view
    /// a row still fully visible stays where it is
    fn settle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, since)) = self.settling_focus.clone() else { return };
        let user_moved = self
            .last_metrics
            .as_ref()
            .is_some_and(|metrics| metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating);
        if user_moved
            || RiseClock::monotonic().saturating_sub(since) > Self::FOCUS_SETTLE_WINDOW
            || !self.state.rows_by_id.borrow().contains_key(&id)
        {
            self.settling_focus = None;
            return;
        }
        if self.list_view.read(cx).is_programmatic_scroll_in_flight() {
            return;
        }
        self.list_view.update(cx, |list, cx| {
            let _ = list.smart_scroll_to_item_centered(
                &id,
                RiseManualListTransactionJumpAnimationDirection::TowardNewer,
                false,
                window,
                cx,
            );
        });
    }

    fn schedule_audit(&mut self, cx: &mut Context<Self>) {
        if self.is_audit_scheduled || self.estimated_ids.is_empty() {
            return;
        }
        self.is_audit_scheduled = true;
        cx.notify();
    }

    fn audit_mounted_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_stopped || self.estimated_ids.is_empty() {
            return;
        }
        if let Some(metrics) = &self.last_metrics
            && (metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating)
        {
            return;
        }
        let visible: Vec<String> = self.list_view.read(cx).visible_ids().to_vec();
        let Some(first) = visible.first().and_then(|id| self.applied_ids.iter().position(|applied| applied == id))
        else {
            return;
        };
        let Some(last) = visible.last().and_then(|id| self.applied_ids.iter().position(|applied| applied == id))
        else {
            return;
        };
        let lower = first.saturating_sub(CommentListPolicy::AUDITED_NEIGHBOURS);
        let upper = (self.applied_ids.len() - 1).min(last + CommentListPolicy::AUDITED_NEIGHBOURS);
        if lower > upper {
            return;
        }

        let height_cache = CommentRowHeightCache::shared(cx);
        let mut corrected = false;
        for id in self.applied_ids[lower..=upper].to_vec() {
            if !self.estimated_ids.contains(&id) {
                continue;
            }
            let Some(view) = self
                .list_view
                .read(cx)
                .mounted_view(&id)
                .and_then(|view| view.downcast::<CommentHostedRowView>().ok())
            else {
                continue;
            };
            let Some(key) = view.read(cx).height_key().cloned() else { continue };
            if view.read(cx).row_id() != Some(id.as_str()) {
                continue;
            }
            let mismatch = view.update(cx, |view, cx| view.prepared_viewport_natural_height_mismatch(window, cx));
            if let Some(natural) = mismatch {
                height_cache.update(cx, |cache, _| cache.insert(natural, key));
                corrected = true;
                continue;
            }
            let applied_height = self.state.applied_heights.borrow().get(&id).copied();
            if !view.read(cx).needs_prepared_viewport_height_validation()
                && let Some(height) = applied_height
            {
                height_cache.update(cx, |cache, _| cache.insert(height, key));
                self.estimated_ids.remove(&id);
            }
        }
        let has_more = self.measure_unmounted_band(first, last, &mut corrected, window, cx);
        if corrected {
            self.commit_if_needed(window, cx);
        }
        if has_more {
            self.schedule_audit(cx);
        }
    }

    fn measure_unmounted_band(
        &mut self,
        first: usize,
        last: usize,
        corrected: &mut bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(input) = self.input.clone() else { return false };
        let width = f32::from(self.list_view.read(cx).bounds().size.width);
        if width <= 1.0 || self.applied_ids.is_empty() {
            return false;
        }
        let lower = first.saturating_sub(CommentListPolicy::SCRATCH_AUDITED_NEIGHBOURS);
        let upper = (self.applied_ids.len() - 1).min(last + CommentListPolicy::SCRATCH_AUDITED_NEIGHBOURS);
        if lower > upper {
            return false;
        }
        let band: HashSet<String> =
            self.applied_ids[lower..=upper].iter().filter(|id| self.estimated_ids.contains(*id)).cloned().collect();
        if band.is_empty() {
            return false;
        }
        let scale = window.scale_factor();
        let height_cache = CommentRowHeightCache::shared(cx);
        let mut budget = CommentListPolicy::SCRATCH_AUDITS_PER_REST;
        let mut has_more = false;
        for row in input.rows.iter().filter(|row| band.contains(&row.id) && !row.is_accessory()) {
            if self.list_view.read(cx).mounted_view(&row.id).is_some() {
                continue;
            }
            if budget == 0 {
                has_more = true;
                break;
            }
            budget -= 1;
            let key = CommentRowHeightKey::new(row, width, scale);
            let Some(measured) = self.scratch_measure(row, width, &input.content, window, cx) else { continue };
            height_cache.update(cx, |cache, _| cache.insert(measured, key));
            self.estimated_ids.remove(&row.id);
            let applied = self.state.applied_heights.borrow().get(&row.id).copied();
            if !matches!(applied, Some(height) if (height - measured).abs() <= 0.25) {
                *corrected = true;
            }
        }
        self.release_scratch_content(cx);
        has_more
    }

    fn scroll_metrics_did_change(
        &mut self,
        metrics: RiseManualListScrollMetrics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_scrolling = self
            .last_metrics
            .as_ref()
            .map(|metrics| metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating)
            .unwrap_or(false);
        let is_scrolling = metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating;
        self.last_metrics = Some(metrics);
        self.evaluate_pagination(window, cx);
        if was_scrolling && !is_scrolling {
            self.set_needs_commit(cx);
            self.schedule_audit(cx);
        }
    }

    /// checked on every scroll callback, drag and deceleration alike
    /// one request per list length, re-armed when the page lands or fails
    fn evaluate_pagination(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = &self.input else { return };
        let Some(metrics) = &self.last_metrics else { return };
        if self.load_more_requested_for_count == Some(self.applied_ids.len()) {
            return;
        }
        if !CommentListPolicy::should_load_more(
            f32::from(metrics.distance_from_bottom),
            f32::from(metrics.viewport_size.height),
            input.has_more,
            input.is_loading_more,
        ) {
            return;
        }
        self.load_more_requested_for_count = Some(self.applied_ids.len());
        let load_more = input.load_more.clone();
        window.defer(cx, move |window, cx| load_more(window, cx));
    }

    fn resolve_scroll_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.input.clone() else { return };
        self.settle_focus(window, cx);
        if let Some(focus_id) = input.focus_id.clone()
            && self.handled_focus_id.as_deref() != Some(focus_id.as_str())
        {
            if self.state.rows_by_id.borrow().contains_key(&focus_id) {
                self.handled_focus_id = Some(focus_id.clone());
                self.settling_focus = Some((focus_id.clone(), RiseClock::monotonic()));
                self.list_view.update(cx, |list, cx| {
                    let _ = list.smart_scroll_to_item_centered(
                        &focus_id,
                        RiseManualListTransactionJumpAnimationDirection::TowardNewer,
                        true,
                        window,
                        cx,
                    );
                });
                let on_focus_resolved = input.on_focus_resolved.clone();
                window.defer(cx, move |window, cx| on_focus_resolved(&focus_id, window, cx));
            } else if self.reported_missing_focus_for_count != Some(self.applied_ids.len()) {
                self.reported_missing_focus_for_count = Some(self.applied_ids.len());
                let on_focus_missing = input.on_focus_missing.clone();
                window.defer(cx, move |window, cx| on_focus_missing(window, cx));
            }
        }
        let reveal_id = input
            .reveal_id
            .clone()
            .filter(|reveal_id| self.state.rows_by_id.borrow().contains_key(reveal_id));
        if let Some(reveal_id) = reveal_id {
            let is_last = self.applied_ids.last() == Some(&reveal_id);
            self.list_view.update(cx, |list, cx| {
                if is_last {
                    let _ = list.smart_scroll_to_bottom(true, window, cx);
                } else {
                    let _ = list.smart_scroll_to_item_centered(
                        &reveal_id,
                        RiseManualListTransactionJumpAnimationDirection::TowardNewer,
                        true,
                        window,
                        cx,
                    );
                }
            });
            let on_revealed = input.on_revealed.clone();
            window.defer(cx, move |window, cx| on_revealed(&reveal_id, window, cx));
        }
    }
}

impl Render for CommentListController {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_commit {
            self.commit_if_needed(window, cx);
        }
        if self.is_audit_scheduled {
            self.is_audit_scheduled = false;
            self.audit_mounted_rows(window, cx);
        }
        div().size_full().bg(self.background_color).child(self.list_view.clone())
    }
}

/// stand-in for a row deferred during a fast scroll, list background and no hits
struct CommentListPlaceholderView {
    background: Hsla,
}

impl Render for CommentListPlaceholderView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(self.background)
    }
}
