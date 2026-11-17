use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, Context, Entity, FocusHandle, Focusable, Hsla, InteractiveElement,
    IntoElement, ParentElement, Pixels, Point, Render, ScrollHandle, Size, StatefulInteractiveElement, Styled,
    Subscription, WeakEntity, Window, div, point, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, SelectionFeedbackGenerator, impact, selection_changed};
use rise_theme::{RiseThemeAppearance, ThemeStore};
use rise_ui::hold_context_menu_ui::hold_context_menu_action_list::{
    DismissMenu, HoldContextMenuActionList, HoldContextMenuInkKey, KEY_CONTEXT, install_key_bindings,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_backdrop_view::HoldContextMenuBackdropView;
use rise_ui::hold_context_menu_ui::hold_context_menu_controller_preview::HoldContextMenuControllerPreviewHost;
use rise_ui::hold_context_menu_ui::hold_context_menu_frame_keys::{HoldContextMenuHitTargets, HoldContextMenuHitTargetsKey};
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::HoldContextMenuShadow;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::rise_frame_rate::RiseMotionCadence;
use rise_ui::top_presenter::TopPresenter;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::environment_values::{ColorScheme, ColorSchemeKey};
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec, Spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, Transition, TransitionModifiers, UnitPoint};

use crate::core::ui::emoji_picker_ui::emoji_picker_recents_store::EmojiPickerRecentsStore;
use crate::core::ui::emoji_picker_ui::emoji_picker_ui::EmojiPickerUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_emoji_bar::{
    HoldContextMenuAllowedReactionsPanel, HoldContextMenuEmojiBar, HoldContextMenuTailDots,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_host::HoldContextMenuWindowPresenter;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_motion_host::HoldContextMenuMotionHost;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_page_host::{
    HoldContextMenuPageContent, HoldContextMenuPageHost, HoldContextMenuPageSizeChanged, HoldContextMenuScrollContent,
    HoldContextMenuScrollViewportModifiers,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::{
    HoldContextMenuPresenter, HoldContextMenuReleaseEvent, HoldContextMenuRequest, HoldContextMenuRequestID,
    HoldContextMenuSourceLift,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_scroll_lock::HoldContextMenuScrollLock;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_touch_tracker::HoldContextMenuTouchTracker;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

type OverlayAction = Rc<dyn Fn(&mut Window, &mut App)>;
type OverlayAppAction = Rc<dyn Fn(&mut App)>;

#[derive(IntoElement)]
struct HoldContextMenuLiftedSourceView {
    source_lift: Rc<HoldContextMenuSourceLift>,
    size: Size<Pixels>,
    replays_source: bool,
}

struct HoldContextMenuLiftedSourceAttachment {
    source_lift: Rc<HoldContextMenuSourceLift>,
}

impl RenderOnce for HoldContextMenuLiftedSourceView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source_lift = self.source_lift;
        let attachment = window.use_keyed_state("hold-menu-lifted-source", cx, |_, cx| {
            source_lift.attach(cx);
            HoldContextMenuLiftedSourceAttachment { source_lift: Rc::clone(&source_lift) }
        });
        let attached = Rc::ptr_eq(&attachment.read(cx).source_lift, &source_lift);
        if !attached {
            source_lift.attach(cx);
            let next = Rc::clone(&source_lift);
            attachment.update(cx, |attachment, _| attachment.source_lift = next);
        }
        let host = div().relative().w(self.size.width).h(self.size.height);
        match source_lift.layout_replaying(self.size, self.replays_source) {
            Some(lifted) => host.child(lifted),
            None => host,
        }
    }
}

/// reads where the overlay copy of the source sits in the window, overlay scroll included
/// for a snapshot or hosted copy that has no lifted view to ask
#[derive(Clone, Default)]
struct HoldContextMenuSourceProbe {
    view: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}

impl HoldContextMenuSourceProbe {
    fn center_in_screen(&self) -> Option<Point<Pixels>> {
        self.view.get().map(|frame| frame.center())
    }
}

struct HoldContextMenuSourceProbeView;

impl HoldContextMenuSourceProbeView {
    fn element(probe: &HoldContextMenuSourceProbe) -> impl IntoElement {
        let frame = Rc::clone(&probe.view);
        div()
            .absolute()
            .inset_0()
            .child(measure("hold-menu-source-probe", move |bounds, _window, _cx| frame.set(Some(bounds))))
    }
}

pub enum HoldContextMenuOverflowPreviewPolicy {}

impl HoldContextMenuOverflowPreviewPolicy {
    pub const DEFAULT_SCROLL_ANCHOR: UnitPoint = UnitPoint::BOTTOM;

    pub fn estimated_menu_height(actions: &[HoldContextMenuAction], accessory_height: f32) -> f32 {
        let section_breaks = actions.iter().skip(1).filter(|action| action.section_break).count();
        10.0 + actions.len() as f32 * HoldContextMenuMetrics::MENU_ROW_HEIGHT
            + section_breaks as f32 * (0.66 + 2.0 * HoldContextMenuMetrics::MENU_SECTION_SEPARATOR_GAP)
            + accessory_height
    }

    pub fn resolved_menu_height(measured_height: f32, actions: &[HoldContextMenuAction], accessory_height: f32) -> f32 {
        if measured_height > 0.0 {
            measured_height
        } else {
            Self::estimated_menu_height(actions, accessory_height)
        }
    }

    pub fn safe_scroll_inset(configured: f32, safe_area: f32) -> f32 {
        configured.max(safe_area + 6.0)
    }

    pub fn initial_scroll_offset(content_height: f32, viewport_height: f32) -> f32 {
        (content_height - viewport_height).max(0.0)
    }
}

#[derive(Clone, Copy)]
struct HoldContextMenuOverlayLayout {
    emoji_x: f32,
    emoji_y: f32,
    source_x: f32,
    source_y: f32,
    menu_x: f32,
    menu_y: f32,
    needs_scroll: bool,
    scroll_content_height: f32,
    picker_x: f32,
    picker_y: f32,
    picker_width: f32,
    picker_height: f32,
    expanded_source_y: f32,
    scroll_content_height_expanded: f32,
    initial_scroll_offset_y: f32,
    emoji_overlap: bool,
    at_cursor: bool,
}

/// window band a cursor menu is placed in
#[derive(Clone, Copy)]
struct HoldContextMenuOverlayCursorFrame {
    width: f32,
    edge: f32,
    top_limit: f32,
    bottom_limit: f32,
    menu_w: f32,
    menu_h: f32,
}

const SUBMENU_BACK_ID: &str = "hold.submenu.back";

fn open_spring() -> MotionSpec {
    MotionSpec::spring(Spring::new(HoldContextMenuMetrics::OPEN_RESPONSE, HoldContextMenuMetrics::OPEN_DAMPING))
}

fn close_spring() -> MotionSpec {
    MotionSpec::spring(Spring::new(HoldContextMenuMetrics::CLOSE_RESPONSE, HoldContextMenuMetrics::CLOSE_DAMPING))
}

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value.max(0.0))
}

fn snapshot_field(request: &HoldContextMenuRequest) -> HashMap<String, String> {
    HashMap::from([("snapshot".to_owned(), if request.snapshot.is_none() { "0" } else { "1" }.to_owned())])
}

fn clamp(v: f32, lower: f32, upper: f32) -> f32 {
    lower.max(v.min(upper))
}

pub struct HoldContextMenuOverlay {
    request: HoldContextMenuRequest,
    on_dismiss: OverlayAction,
    presenter: Entity<HoldContextMenuPresenter>,

    appeared: bool,
    lift_landed: bool,
    current_lift_scale: f32,
    live_preview_height: f32,
    source_opacity: Animated<f32>,
    backdrop_revealed: bool,
    menu_panel_size: Size<Pixels>,
    action_page_size: Option<Size<Pixels>>,
    menu_fully_opened: bool,
    release_selection_armed: bool,
    source_handed_back: bool,
    source_landing_finished: bool,
    source_return_finished: bool,
    source_copy_hidden: Cell<bool>,
    dismissal_finished: bool,
    backdrop_close_finished: bool,
    return_center: Option<Point<Pixels>>,
    return_shift: Option<Point<Pixels>>,
    source_probe: HoldContextMenuSourceProbe,

    hit_targets: HoldContextMenuHitTargets,
    motion_cadence: RiseMotionCadence,
    hovered_action_id: Option<String>,
    hovered_emoji: Option<String>,
    hover_haptic: SelectionFeedbackGenerator,

    last_handled_release: Option<HoldContextMenuReleaseEvent>,

    active_submenu: Option<Vec<HoldContextMenuAction>>,
    accessory_details_visible: bool,

    emoji_picker_expanded: bool,
    emoji_picker_presence: Presence<()>,
    emoji_picker_appearance: Animated<f32>,

    is_dismissing: bool,
    close_began_at: Option<Duration>,
    is_committing_preview: bool,
    preview_commit_delivered: bool,

    focus_handle: FocusHandle,
    list_focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,
    scroll_anchored: bool,
    window_handle: AnyWindowHandle,
    interaction_observer: HoldContextMenuInteractionObserver,
    _subscriptions: Vec<Subscription>,
}

impl HoldContextMenuOverlay {
    pub fn new(
        request: HoldContextMenuRequest,
        on_dismiss: OverlayAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        install_key_bindings(cx);
        let presenter = HoldContextMenuPresenter::shared(cx);
        let interaction_observer = HoldContextMenuInteractionObserver::new(presenter.read(cx));
        let subscription = cx.observe_in(&presenter, window, |this, presenter, window, cx| {
            this.presenter_did_change(&presenter, window, cx);
        });
        cx.defer_in(window, |this, window, cx| this.open_in(window, cx));
        Self {
            current_lift_scale: request.initial_lift_scale,
            live_preview_height: f32::from(request.source_frame.size.height),
            request,
            on_dismiss,
            presenter,
            appeared: false,
            lift_landed: false,
            source_opacity: Animated::new(1.0),
            backdrop_revealed: false,
            menu_panel_size: Size::default(),
            action_page_size: None,
            menu_fully_opened: false,
            release_selection_armed: false,
            source_handed_back: false,
            source_landing_finished: false,
            source_return_finished: false,
            source_copy_hidden: Cell::new(false),
            dismissal_finished: false,
            backdrop_close_finished: false,
            return_center: None,
            return_shift: None,
            source_probe: HoldContextMenuSourceProbe::default(),
            hit_targets: HoldContextMenuHitTargets::new(),
            motion_cadence: RiseMotionCadence::new(),
            hovered_action_id: None,
            hovered_emoji: None,
            hover_haptic: SelectionFeedbackGenerator::new(),
            last_handled_release: None,
            active_submenu: None,
            accessory_details_visible: false,
            emoji_picker_expanded: false,
            emoji_picker_presence: Presence::new(None).with_transition(
                Transition::scale_anchored(0.42, UnitPoint::TOP).combined(Transition::opacity()),
            ),
            emoji_picker_appearance: Animated::new(0.0),
            is_dismissing: false,
            close_began_at: None,
            is_committing_preview: false,
            preview_commit_delivered: false,
            focus_handle: cx.focus_handle(),
            list_focus_handle: cx.focus_handle(),
            scroll_handle: ScrollHandle::new(),
            scroll_anchored: false,
            window_handle: window.window_handle(),
            interaction_observer,
            _subscriptions: vec![subscription],
        }
    }

    fn is_active(&self, cx: &App) -> bool {
        self.presenter
            .read(cx)
            .active_request()
            .is_some_and(|active| active.id == self.request.id)
    }

    fn source_size(&self) -> Size<f32> {
        size(f32::from(self.request.source_frame.size.width), self.live_preview_height)
    }

    fn resolved_actions(&self, cx: &App) -> Vec<HoldContextMenuAction> {
        self.request.resolved_actions(cx)
    }

    fn displayed_actions(&self, cx: &App) -> Vec<HoldContextMenuAction> {
        if self.accessory_details_visible || self.emoji_picker_expanded {
            return Vec::new();
        }
        if let Some(submenu) = self.active_submenu.as_ref() {
            return Self::submenu_rows(submenu);
        }
        self.resolved_actions(cx)
    }

    /// every submenu starts with a back row to the root page, same as telegram
    fn submenu_rows(submenu: &[HoldContextMenuAction]) -> Vec<HoldContextMenuAction> {
        let mut rows = submenu.to_vec();
        if let Some(first) = rows.first_mut() {
            first.section_break = true;
        }
        let back = HoldContextMenuAction::new(SUBMENU_BACK_ID, tr("common_back")).system_image("chevron.left");
        std::iter::once(back).chain(rows).collect()
    }

    fn listener0(&self, cx: &Context<Self>, f: fn(&mut Self, &mut Window, &mut Context<Self>)) -> OverlayAction {
        let this = cx.entity().downgrade();
        Rc::new(move |window: &mut Window, cx: &mut App| {
            this.update(cx, |this, cx| f(this, window, cx)).ok();
        })
    }

    fn listener1(
        &self,
        cx: &Context<Self>,
        f: fn(&mut Self, String, &mut Window, &mut Context<Self>),
    ) -> Rc<dyn Fn(String, &mut Window, &mut App)> {
        let this = cx.entity().downgrade();
        Rc::new(move |value: String, window: &mut Window, cx: &mut App| {
            this.update(cx, |this, cx| f(this, value, window, cx)).ok();
        })
    }

    fn flat_chrome_color(&self, cx: &mut App) -> Option<Hsla> {
        self.request
            .uses_flat_chrome
            .then(|| ThemeStore::shared(cx).read(cx).current_theme().bg._200)
    }

    fn backdrop(&self, at_cursor: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let solid_color: Option<Hsla> = Some(gpui::transparent_black());
        let on_hidden = self.listener0(cx, |this, window, cx| {
            if !this.is_dismissing || !this.is_active(cx) {
                return;
            }
            this.backdrop_close_finished = true;
            this.finish_dismiss_if_ready(window, cx);
        });
        let on_tap = cx.listener(|this, _, window, cx| this.dismiss_from_backdrop(window, cx));
        let anchor_cover = (self.request.snapshot.is_some() && !self.request.is_centered && !at_cursor).then(|| {
            let frame = self.request.source_frame;
            div()
                .absolute()
                .left(frame.origin.x)
                .top(frame.origin.y)
                .w(frame.size.width)
                .h(frame.size.height)
                .bg(ThemeStore::shared(cx).read(cx).current_theme().bg._100)
        });
        div()
            .id("hold-menu-backdrop")
            .absolute()
            .inset_0()
            .children(anchor_cover)
            .child(
                HoldContextMenuBackdropView::new("hold-menu-backdrop-effect", self.backdrop_revealed)
                    .solid_color(solid_color)
                    .on_hidden(Some(on_hidden)),
            )
            .on_click(on_tap)
    }

    /// backdrop takes a tap as soon as the menu is on screen
    /// gating it on the old 0.38 s fully opened delay swallowed the first tap after an immediate release
    fn dismiss_from_backdrop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        tracing::warn!(
            target: "riseonly::X1529",
            opened = self.menu_fully_opened,
            dismissing = self.is_dismissing,
            "menu backdrop click"
        );
        if !self.menu_fully_opened || self.is_dismissing || self.is_committing_preview {
            return;
        }
        self.dismiss(window, cx);
    }

    fn content_stack(
        &self,
        layout: HoldContextMenuOverlayLayout,
        viewport: Size<Pixels>,
        now: MotionTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut stack = div().relative().size_full();
        if let Some(preview) = self.request.controller_preview.clone() {
            let source_frame = preview.borrow().source_frame;
            let frame = if self.is_committing_preview {
                Bounds::new(Point::default(), viewport)
            } else if self.appeared {
                let source_size = self.source_size();
                Bounds::new(
                    point(px(layout.source_x), px(layout.source_y)),
                    size(px(source_size.width), px(source_size.height)),
                )
            } else {
                let center = self.return_center.unwrap_or(source_frame.center());
                Bounds::centered_at(center, source_frame.size)
            };
            let phase = if self.is_committing_preview {
                3
            } else if self.is_dismissing {
                2
            } else if self.appeared {
                1
            } else {
                0
            };
            let on_commit = self.listener0(cx, |this, window, cx| this.commit_preview(window, cx));
            let on_settled = self.listener0(cx, |this, window, cx| this.preview_did_settle(window, cx));
            let on_backdrop_tap = self.listener0(cx, |this, window, cx| this.dismiss_from_backdrop(window, cx));
            stack = stack.child(HoldContextMenuControllerPreviewHost::new(
                "hold-menu-controller-preview",
                preview,
                frame,
                phase,
                on_commit,
                on_settled,
                on_backdrop_tap,
            ));
        }
        let source_on_top =
            self.request.controller_preview.is_none() && !self.emoji_picker_expanded && !layout.at_cursor;
        if self.request.controller_preview.is_none() && !source_on_top {
            stack = stack.child(self.source_view(layout, now, window, cx));
        }

        let emoji_over_source = layout.emoji_overlap || layout.emoji_y >= layout.source_y;
        let emoji_bar_first = self.request.show_emoji_list && !layout.needs_scroll && !emoji_over_source;
        let emoji_bar_last = self.request.show_emoji_list && !layout.needs_scroll && emoji_over_source;
        if emoji_bar_first {
            stack = stack.child(self.emoji_bar(layout, cx));
        }
        if !layout.needs_scroll {
            stack = stack.child(self.action_list(layout, 0.0, cx));
        }
        if !layout.needs_scroll {
            if let Some(panel) = self.emoji_picker_panel(layout, now, window, cx) {
                stack = stack.child(panel);
            }
        }
        if source_on_top {
            stack = stack.child(self.source_view(layout, now, window, cx));
        }
        if emoji_bar_last {
            stack = stack.child(self.emoji_bar(layout, cx));
        }
        stack.into_any_element()
    }

    fn preview_did_settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_committing_preview {
            if self.preview_commit_delivered || !self.is_active(cx) {
                return;
            }
            self.preview_commit_delivered = true;
            let Some(preview) = self.request.controller_preview.clone() else { return };
            let id = self.request.id;
            let dismiss: OverlayAction = Rc::new(move |_window: &mut Window, cx: &mut App| {
                HoldContextMenuPresenter::dismiss_if_active(id, cx);
            });
            let on_commit = Rc::clone(&preview.borrow().on_commit);
            on_commit(dismiss, window, cx);
        } else {
            self.source_motion_completed(true, window, cx);
        }
    }

    fn overflow_preview_mask(&self, layout: HoldContextMenuOverlayLayout, viewport: Size<Pixels>) -> (f32, f32) {
        if layout.needs_scroll && !self.is_dismissing && self.request.controller_preview.is_none() {
            let top = layout.source_y
                - if self.request.show_emoji_list { HoldContextMenuMetrics::EMOJI_TO_SOURCE_GAP } else { 0.0 };
            let bottom = layout.menu_y - layout.initial_scroll_offset_y;
            let band_top = if self.appeared { top } else { 0.0 };
            let band_bottom = if self.lift_landed { bottom } else { bottom.max(f32::from(viewport.height)) };
            (band_top, (band_bottom - band_top).max(0.0))
        } else {
            (0.0, f32::from(viewport.height))
        }
    }

    fn reaction_layers(
        &self,
        layout: HoldContextMenuOverlayLayout,
        now: MotionTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut layers = Vec::new();
        if self.request.show_emoji_list {
            layers.push(self.emoji_bar(layout, cx).into_any_element());
        }
        if let Some(panel) = self.emoji_picker_panel(layout, now, window, cx) {
            layers.push(panel);
        }
        layers
    }

    fn source_view(
        &self,
        layout: HoldContextMenuOverlayLayout,
        now: MotionTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let source_size = self.source_size();
        let source_frame = self.request.source_frame;
        let source_w = f32::from(source_frame.size.width);
        let source_h = f32::from(source_frame.size.height);
        let final_source_y = if self.emoji_picker_expanded { layout.expanded_source_y } else { layout.source_y };
        let source_center = self.return_center.unwrap_or(self.request.initial_source_center);
        let initial_center = point(
            f32::from(source_center.x),
            f32::from(source_center.y) + layout.initial_scroll_offset_y,
        );
        let open_center = point(layout.source_x + source_w / 2.0, final_source_y + source_size.height / 2.0);
        let fly_back = if self.is_dismissing && layout.needs_scroll { self.return_shift } else { None };
        let resting_center = match fly_back {
            Some(shift) => point(open_center.x + f32::from(shift.x), open_center.y + f32::from(shift.y)),
            None if self.request.is_centered || (self.is_dismissing && layout.needs_scroll) => open_center,
            None => initial_center,
        };
        let fades_in_place =
            self.request.is_centered || (self.is_dismissing && layout.needs_scroll && fly_back.is_none());
        let source_in_place = layout.at_cursor
            && self.request.source_lift.is_none()
            && self.request.snapshot.is_none()
            && self.request.controller_preview.is_none();

        let lifted_core: AnyElement = if source_in_place {
            div().w(px(source_w)).h(px(source_h)).into_any_element()
        } else if let Some(source_lift) = self.request.source_lift.clone() {
            let this = cx.entity().downgrade();
            source_lift.set_on_content_growth(Some(Rc::new(move |growth: f32, window: &mut Window, cx: &mut App| {
                Self::centered_source_did_resize(&this, source_h + growth, window, cx);
            })));
            div()
                .w(px(source_w))
                .h(px(source_h + source_lift.content_growth()))
                .child(HoldContextMenuLiftedSourceView {
                    source_lift,
                    size: source_frame.size,
                    replays_source: self.lift_landed && !self.is_dismissing && !self.source_handed_back,
                })
                .into_any_element()
        } else if let Some(snapshot) = self.request.snapshot.as_ref() {
            div()
                .w(px(source_w))
                .h(px(source_h))
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .child(snapshot.element())
                .into_any_element()
        } else if self.request.is_centered {
            let this = cx.entity().downgrade();
            div()
                .relative()
                .w(px(source_size.width))
                .child((self.request.content)(window, cx))
                .child(measure("hold-menu-centered-source", move |bounds, window, cx| {
                    Self::centered_source_did_resize(&this, f32::from(bounds.size.height), window, cx);
                }))
                .into_any_element()
        } else {
            div()
                .w(px(source_w))
                .h(px(source_h))
                .child((self.request.content)(window, cx))
                .into_any_element()
        };


        let clipped_core = match self.request.source_bubble_clip.as_ref() {
            Some(clip) if self.request.source_lift.is_none() && self.request.snapshot.is_none() => {
                let corners = clip.uneven_shape();
                div()
                    .rounded_tl(corners.top_left)
                    .rounded_tr(corners.top_right)
                    .rounded_bl(corners.bottom_left)
                    .rounded_br(corners.bottom_right)
                    .overflow_hidden()
                    .child(lifted_core)
                    .into_any_element()
            }
            _ => lifted_core,
        };
        let mut source_content = div().relative();
        if !layout.needs_scroll && !layout.at_cursor {
            let shadow = HoldContextMenuShadow::default()
                .radius(HoldContextMenuMetrics::LIFTED_SHADOW_RADIUS)
                .opacity(HoldContextMenuMetrics::LIFTED_SHADOW_OPACITY as f32)
                .offset_y(HoldContextMenuMetrics::LIFTED_SHADOW_OFFSET_Y)
                .bubble_clip(self.request.source_bubble_clip.clone());
            source_content = source_content
                .child(div().absolute().inset_0().opacity(if self.appeared { 1.0 } else { 0.0 }).child(shadow));
        }
        if let Some(clip) = self.request.source_bubble_clip.as_ref().filter(|_| !layout.at_cursor) {
            let corners = clip.uneven_shape();
            let plate = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
            source_content = source_content.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_tl(corners.top_left)
                    .rounded_tr(corners.top_right)
                    .rounded_bl(corners.bottom_left)
                    .rounded_br(corners.bottom_right)
                    .bg(plate),
            );
        }
        let source_content = source_content
            .child(HoldContextMenuSourceProbeView::element(&self.source_probe))
            .child(clipped_core);

        let anchor = point(
            f32::from(self.request.initial_source_center.x),
            f32::from(self.request.initial_source_center.y) + layout.initial_scroll_offset_y,
        );
        let destination = if self.appeared { open_center } else { resting_center };
        let response = if self.source_handed_back {
            HoldContextMenuMetrics::HIGHLIGHT_FADE_DURATION / HoldContextMenuMetrics::SOURCE_RESTORE_DELAY_FACTOR
        } else if self.is_dismissing {
            HoldContextMenuMetrics::CLOSE_RESPONSE
        } else {
            HoldContextMenuMetrics::OPEN_RESPONSE
        };
        let damping =
            if self.is_dismissing { HoldContextMenuMetrics::CLOSE_DAMPING } else { HoldContextMenuMetrics::OPEN_DAMPING };
        let phase = if self.is_dismissing {
            2
        } else if self.appeared {
            1
        } else {
            0
        };
        self.source_copy_hidden.set(source_in_place);
        let opacity = self.source_opacity.frame_value(now, window)
            * if (fades_in_place && !self.appeared) || source_in_place { 0.0 } else { 1.0 };
        let on_motion_completed: Option<OverlayAppAction> = if self.is_dismissing {
            if layout.needs_scroll {
                Some(self.app_listener0(cx, |this, window, cx| this.source_motion_completed(true, window, cx)))
            } else {
                Some(self.app_listener0(cx, |this, window, cx| this.source_motion_completed(false, window, cx)))
            }
        } else if self.appeared {
            Some(self.app_listener0(cx, |this, _window, cx| {
                this.lift_landed = true;
                cx.notify();
            }))
        } else {
            None
        };
        let host = HoldContextMenuMotionHost::new(
            "hold-menu-source-motion",
            "source",
            size(px(source_size.width), px(source_size.height)),
            source_content,
        )
            .offset(point(px(destination.x - anchor.x), px(destination.y - anchor.y)))
            .scale(if layout.at_cursor { 1.0 } else { self.current_lift_scale })
            .opacity(opacity)
            .response(response)
            .damping(damping)
            .phase(phase)
            .on_motion_completed(on_motion_completed);
        div()
            .absolute()
            .left(px(anchor.x - source_size.width / 2.0))
            .top(px(anchor.y - source_size.height / 2.0))
            .w(px(source_size.width))
            .h(px(source_size.height))
            .child(host)
            .into_any_element()
    }

    fn centered_source_did_resize(this: &WeakEntity<Self>, height: f32, window: &mut Window, cx: &mut App) {
        this.update(cx, |this, cx| {
            if !height.is_finite() || height <= 0.0 || this.is_dismissing
                || (height - this.live_preview_height).abs() <= 0.5
            {
                return;
            }
            this.begin_motion(Self::default_motion_duration(), window, cx);
            this.live_preview_height = height;
            cx.notify();
        })
        .ok();
    }

    fn emoji_bar(&self, layout: HoldContextMenuOverlayLayout, cx: &mut Context<Self>) -> AnyElement {
        let revealed = self.appeared
            && self.active_submenu.is_none()
            && !self.accessory_details_visible
            && !self.emoji_picker_expanded;
        let emoji_size = self.estimated_emoji_size();
        let on_select = self.listener1(cx, |this, emoji, window, cx| this.handle_emoji_button_tap(emoji, window, cx));
        let on_expand = self
            .shows_emoji_expand()
            .then(|| self.listener0(cx, |this, window, cx| this.expand_emoji_picker(window, cx)));
        let bar = HoldContextMenuEmojiBar::new(
            "hold-menu-emoji-bar",
            self.request.emojis.clone(),
            self.request.alignment,
            self.hovered_emoji.clone(),
            on_select,
        )
        .hit_targets(Some(self.hit_targets.clone()))
        .is_revealed(revealed)
        .on_expand(on_expand);
        let host = HoldContextMenuMotionHost::new("hold-menu-reactions-motion", "reactions", emoji_size, bar)
            .scale(if revealed { 1.0 } else { HoldContextMenuMetrics::INITIAL_PANEL_SCALE });
        div()
            .id("hold-menu-emoji-bar-frame")
            .absolute()
            .left(px(layout.emoji_x))
            .top(px(layout.emoji_y))
            .w(emoji_size.width)
            .h(emoji_size.height)
            .occlude()
            .child(host)
            .into_any_element()
    }

    fn action_list(
        &self,
        layout: HoldContextMenuOverlayLayout,
        pinned_scroll_offset_y: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let anchor_w = if self.menu_panel_size.width > Pixels::ZERO {
            f32::from(self.menu_panel_size.width)
        } else {
            HoldContextMenuMetrics::MENU_MIN_WIDTH
        };
        let revealed = self.appeared && !self.emoji_picker_expanded && !self.is_committing_preview;
        let page_size = self.action_page_size.unwrap_or_else(|| {
            let actions = self.resolved_actions(cx);
            let estimated = HoldContextMenuOverflowPreviewPolicy::estimated_menu_height(
                &actions,
                self.accessory_height(),
            );
            size(px(anchor_w), px(estimated.min(self.menu_maximum_height())))
        });
        let trailing = self.request.alignment == HoldContextMenuAlignment::Trailing;
        let this = cx.entity().downgrade();
        let on_size_changed: HoldContextMenuPageSizeChanged =
            Rc::new(move |page_size: Size<Pixels>, window: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| this.page_size_did_change(page_size, window, cx)).ok();
            });
        let depth = if self.active_submenu.is_some() || self.accessory_details_visible { 1 } else { 0 };
        let this = cx.entity().downgrade();
        let content: HoldContextMenuPageContent = Rc::new(move |_window: &mut Window, cx: &mut App| {
            this.update(cx, |this, cx| this.panel_page_content(cx))
                .unwrap_or_else(|_| div().into_any_element())
        });
        let page = HoldContextMenuPageHost::new(
            "hold-menu-page-host",
            self.panel_page_id(),
            depth,
            trailing,
            revealed,
            self.flat_chrome_color(cx),
            on_size_changed,
            content,
        );
        let host = HoldContextMenuMotionHost::new("hold-menu-actions-motion", "actions", page_size, page)
            .content_alignment(Some(if trailing { Alignment::TopTrailing } else { Alignment::TopLeading }))
            .scale(if revealed { 1.0 } else { HoldContextMenuMetrics::INITIAL_PANEL_SCALE });
        let page_w = f32::from(page_size.width);
        let x = if trailing { layout.menu_x + anchor_w - page_w } else { layout.menu_x };
        div()
            .id("hold-menu-panel-frame")
            .absolute()
            .left(px(x))
            .top(px(layout.menu_y - pinned_scroll_offset_y))
            .w(page_size.width)
            .h(page_size.height)
            .occlude()
            .child(host)
            .into_any_element()
    }

    fn page_size_did_change(&mut self, page_size: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_active(cx) {
            return;
        }
        if self.action_page_size != Some(page_size) {
            self.action_page_size = Some(page_size);
            cx.notify();
        }
        if self.active_submenu.is_some() || self.accessory_details_visible || page_size == self.menu_panel_size {
            return;
        }
        let first_measurement = self.menu_panel_size == Size::default();
        self.menu_panel_size = page_size;
        if first_measurement {
            self.record_resolved_layout(page_size, window);
        }
        cx.notify();
    }

    fn accessory_height(&self) -> f32 {
        self.request.accessory.as_ref().map_or(0.0, |accessory| accessory.summary_height + 0.66)
    }

    fn menu_maximum_height(&self) -> f32 {
        self.request
            .controller_preview
            .as_ref()
            .map_or(f32::MAX, |preview| f32::from(preview.borrow().menu_maximum_height()))
    }

    fn panel_page_id(&self) -> String {
        if self.accessory_details_visible {
            return "details".to_owned();
        }
        if let Some(submenu) = self.active_submenu.as_ref() {
            let ids: Vec<&str> = submenu.iter().map(|action| action.id.as_str()).collect();
            return format!("submenu:{}", ids.join(","));
        }
        "root".to_owned()
    }

    fn panel_page_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let ink = self.menu_ink(cx);
        let content: AnyElement = if let Some(accessory) =
            self.request.accessory.as_ref().filter(|_| self.accessory_details_visible)
        {
            let close = self.app_listener0(cx, |this, window, cx| this.close_accessory_details(window, cx));
            let action = self.app_listener1(cx, |this, id, window, cx| this.handle_accessory_action(id, window, cx));
            (accessory.details_view)(close, action)
        } else if let Some(submenu) = self.active_submenu.as_ref().map(|submenu| Self::submenu_rows(submenu)) {
            let list = self
                .action_list_rows("hold-menu-submenu-actions", submenu.clone(), cx)
                .max_width(HoldContextMenuMetrics::MENU_SUBMENU_MAX_WIDTH);
            self.preview_action_viewport(
                &submenu,
                HoldContextMenuMetrics::MENU_SUBMENU_MAX_WIDTH,
                list.into_any_element(),
                cx,
            )
        } else {
            let mut column = div().flex().flex_col();
            if let Some(accessory) = self.request.accessory.as_ref() {
                let open = self.app_listener0(cx, |this, window, cx| this.open_accessory_details(window, cx));
                column = column
                    .child(
                        div()
                            .min_w(px(HoldContextMenuMetrics::MENU_MIN_WIDTH))
                            .max_w(px(HoldContextMenuMetrics::MENU_MAX_WIDTH))
                            .min_h(px(accessory.summary_height))
                            .child((accessory.summary_view)(open)),
                    )
                    .child(
                        div()
                            .h(px(0.66))
                            .mx(px(HoldContextMenuMetrics::MENU_SECTION_SEPARATOR_INSET))
                            .bg(ink.opacity(HoldContextMenuMetrics::MENU_SEPARATOR_OPACITY as f32)),
                    );
            }
            let actions = self.resolved_actions(cx);
            let rows = self.root_actions(actions.clone(), cx);
            column
                .child(self.preview_action_viewport(&actions, HoldContextMenuMetrics::MENU_MAX_WIDTH, rows, cx))
                .into_any_element()
        };
        env_scope::<HoldContextMenuInkKey>(ink).child(content).into_any_element()
    }

    fn menu_ink(&self, cx: &mut App) -> Hsla {
        if self.request.prefers_dark_chrome {
            return white();
        }
        ThemeStore::shared(cx).read(cx).current_theme().text.primary
    }

    fn preview_action_viewport(
        &self,
        actions: &[HoldContextMenuAction],
        width: f32,
        rows: AnyElement,
        cx: &App,
    ) -> AnyElement {
        let Some(preview) = self.request.controller_preview.as_ref() else {
            return match self.request.max_visible_rows {
                Some(limit) if limit >= 0 && actions.len() > limit as usize => {
                    let visible = &actions[..limit as usize];
                    div()
                        .id("hold-menu-limited-actions")
                        .overflow_y_scroll()
                        .hold_context_menu_scroll_viewport(cx)
                        .w(px(HoldContextMenuMetrics::MENU_MAX_WIDTH))
                        .h(px(HoldContextMenuOverflowPreviewPolicy::estimated_menu_height(visible, 0.0)))
                        .child(div().relative().child(rows).child(HoldContextMenuScrollContent::new()))
                        .into_any_element()
                }
                _ => rows,
            };
        };
        let preview = preview.borrow();
        let height = f32::from(preview.menu_maximum_height())
            .min(HoldContextMenuOverflowPreviewPolicy::estimated_menu_height(actions, 0.0));
        div()
            .id("hold-menu-preview-actions")
            .overflow_y_scroll()
            .hold_context_menu_scroll_viewport(cx)
            .w(px(width.min(f32::from(preview.size().width))))
            .h(px(height))
            .child(div().relative().child(rows).child(HoldContextMenuScrollContent::new()))
            .into_any_element()
    }

    fn action_list_rows(
        &self,
        id: &'static str,
        actions: Vec<HoldContextMenuAction>,
        cx: &mut Context<Self>,
    ) -> HoldContextMenuActionList {
        let on_select = self.listener1(cx, |this, id, window, cx| this.handle_row_button_tap(id, window, cx));
        let this = cx.entity().downgrade();
        let on_highlight: Rc<dyn Fn(Option<String>, &mut Window, &mut App)> =
            Rc::new(move |id: Option<String>, _window: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| this.pointer_did_highlight(id, cx)).ok();
            });
        let on_dismiss = self.listener0(cx, |this, window, cx| this.dismiss(window, cx));
        HoldContextMenuActionList::new(id, actions, self.hovered_action_id.clone(), on_select)
            .hit_targets(Some(self.hit_targets.clone()))
            .chrome(false)
            .max_width(if self.request.controller_preview.is_none() {
                HoldContextMenuMetrics::MENU_TITLE_MAX_WIDTH
            } else {
                HoldContextMenuMetrics::MENU_MAX_WIDTH
            })
            .row_line_limit(2)
            .on_highlight(on_highlight)
            .on_dismiss(on_dismiss)
            .focus_handle(self.list_focus_handle.clone())
    }

    fn root_actions(&self, actions: Vec<HoldContextMenuAction>, cx: &mut Context<Self>) -> AnyElement {
        self.action_list_rows("hold-menu-root-actions", actions, cx).into_any_element()
    }

    fn pointer_did_highlight(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.is_dismissing || self.hovered_action_id == id {
            return;
        }
        self.hovered_action_id = id;
        self.hover_did_adopt(self.hovered_action_id.is_some());
        cx.notify();
    }

    /// one haptic tick per row the finger adopts, same cue as the tab bar menu
    fn hover_did_adopt(&self, adopted: bool) {
        if !adopted {
            return;
        }
        let _ = self.hover_haptic.selection_changed();
        let _ = self.hover_haptic.prepare();
    }

    fn commit_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = self.request.controller_preview.clone() else { return };
        if self.is_dismissing || self.is_committing_preview || self.preview_commit_delivered {
            return;
        }
        self.menu_fully_opened = false;
        self.release_selection_armed = false;
        let on_expand = Rc::clone(&preview.borrow().on_expand);
        on_expand(window, cx);
        self.is_committing_preview = true;
        cx.notify();
    }

    /// a limited chat whose whole list already sits in the bar has nothing more to expand
    fn shows_emoji_expand(&self) -> bool {
        let Some(allowed) = self.request.expanded_emojis.as_ref() else { return true };
        allowed.len() > self.request.emojis.len()
            || allowed.iter().any(|emoji| !self.request.emojis.contains(emoji))
    }

    fn emoji_picker_panel(
        &self,
        layout: HoldContextMenuOverlayLayout,
        now: MotionTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let content: AnyElement = if let Some(allowed) = self.request.expanded_emojis.clone() {
            let on_pick = self.listener1(cx, |this, emoji, window, cx| this.pick_from_panel(emoji, window, cx));
            HoldContextMenuAllowedReactionsPanel::new("hold-menu-allowed-reactions", allowed, on_pick).into_any_element()
        } else {
            let pick = self.listener1(cx, |this, emoji, window, cx| this.pick_from_panel(emoji, window, cx));
            let pick_custom = Rc::clone(&pick);
            EmojiPickerUi::new("hold-menu-emoji-picker", move |emoji, window, cx| pick(emoji, window, cx))
                .rows_settling(self.emoji_picker_appearance.is_running(now))
                .shows_custom_emoji(self.request.allows_custom_emoji)
                .on_pick_custom(move |item: StickerItem, window, cx| {
                    pick_custom(format!("{}{}", MessageReaction::CUSTOM_EMOJI_PREFIX, item.id), window, cx);
                })
                .into_any_element()
        };
        let trailing = self.request.alignment == HoldContextMenuAlignment::Trailing;
        let tail = div()
            .absolute()
            .bottom_0()
            .when(trailing, |tail| tail.right_0())
            .when(!trailing, |tail| tail.left_0())
            .child(HoldContextMenuTailDots::new(self.request.alignment));
        let appearance = self.emoji_picker_appearance.frame_value(now, window);
        let panel = div()
            .relative()
            .w(px(layout.picker_width))
            .h(px(layout.picker_height))
            .child(content)
            .child(tail)
            .opacity(appearance)
            .scale_effect_anchored(0.55 + 0.45 * appearance, UnitPoint::BOTTOM);
        self.emoji_picker_presence
            .render_transitioned(now, window, move |_| {
                div()
                    .id("hold-menu-emoji-picker-frame")
                    .absolute()
                    .left(px(layout.picker_x - layout.picker_width / 2.0))
                    .top(px(layout.picker_y))
                    .w(px(layout.picker_width))
                    .h(px(layout.picker_height))
                    .occlude()
                    .child(panel)
            })
            .map(IntoElement::into_any_element)
    }

    fn pick_from_panel(&mut self, emoji: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.menu_fully_opened {
            return;
        }
        HoldContextMenuPresenter::note_close_imminent(cx);
        (self.request.on_emoji)(emoji, window, cx);
        self.dismiss(window, cx);
    }

    fn pick_emoji(&mut self, emoji: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dismissing {
            return;
        }
        EmojiPickerRecentsStore::shared(cx).update(cx, |store, cx| store.register(&emoji, cx));
        HoldContextMenuPresenter::note_close_imminent(cx);
        (self.request.on_emoji)(emoji, window, cx);
        self.dismiss(window, cx);
    }

    fn expand_emoji_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.emoji_picker_expanded {
            return;
        }
        self.begin_motion(Self::default_motion_duration(), window, cx);
        let _ = impact(ImpactStyle::Light);
        self.emoji_picker_expanded = true;
        self.emoji_picker_presence.set_in(Some(()), open_spring(), RiseClock::monotonic(), cx);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn handle_row_button_tap(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let drag = self.presenter.read(cx).drag_location;
        tracing::warn!(
            target: "riseonly::X1529",
            row = %id,
            opened = self.menu_fully_opened,
            dismissing = self.is_dismissing,
            drag = ?drag,
            "menu row button tap"
        );
        if !self.menu_fully_opened || self.emoji_picker_expanded || self.is_committing_preview {
            return;
        }
        if let Some(drag) = drag {
            if self.action_id(drag, cx).as_deref() != Some(id.as_str()) {
                return;
            }
        }
        self.handle_row_tap(id, window, cx);
    }

    fn handle_emoji_button_tap(&mut self, emoji: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.menu_fully_opened {
            return;
        }
        if let Some(drag) = self.presenter.read(cx).drag_location {
            if self.emoji_key(drag).as_deref() != Some(emoji.as_str()) {
                return;
            }
        }
        self.pick_emoji(emoji, window, cx);
    }

    /// one pointer can claim a row twice, the button and a lift off that moved but stayed in the same row
    /// the first to land begins the close and this guard makes the other a no op
    /// without it a destructive action runs twice
    fn handle_row_tap(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        tracing::warn!(
            target: "riseonly::X2255",
            row = %id,
            dismissing = self.is_dismissing,
            delivered = self.preview_commit_delivered,
            "menu row tap"
        );
        if self.is_dismissing || self.preview_commit_delivered {
            return;
        }
        if id == SUBMENU_BACK_ID && self.active_submenu.is_some() {
            self.close_submenu(window, cx);
            return;
        }
        let Some(action) = self.displayed_actions(cx).into_iter().find(|action| action.id == id) else {
            return;
        };
        if action.disabled {
            return;
        }
        if let Some(submenu) = action.submenu.filter(|submenu| !submenu.is_empty()) {
            self.open_submenu(submenu, window, cx);
            return;
        }
        if action.dismisses_menu {
            HoldContextMenuPresenter::note_close_imminent(cx);
        }
        let performed = HoldContextMenuPresenter::perform_action(&id, self.request.id, window, cx);
        tracing::warn!(target: "riseonly::X2254", row = %id, performed, "menu row performs");
        if !performed {
            tracing::warn!(target: "riseonly::X2255", row = %id, "menu row tap not performed");
            return;
        }
        if action.dismisses_menu {
            self.dismiss(window, cx);
        } else {
            self.clear_drag_location(cx);
            self.hovered_action_id = None;
            self.begin_motion(Self::default_motion_duration(), window, cx);
            cx.notify();
        }
    }

    fn clear_drag_location(&mut self, cx: &mut Context<Self>) {
        self.presenter.update(cx, |presenter, cx| {
            presenter.drag_location = None;
            cx.notify();
        });
    }

    fn open_submenu(&mut self, submenu: Vec<HoldContextMenuAction>, window: &mut Window, cx: &mut Context<Self>) {
        ChatPerformanceTrace::shared().record(
            "hold_menu.submenu_begin",
            HashMap::from([("actions".to_owned(), submenu.len().to_string())]),
        );
        self.clear_drag_location(cx);
        self.begin_motion(Self::default_motion_duration(), window, cx);
        self.hovered_action_id = None;
        self.hovered_emoji = None;
        self.accessory_details_visible = false;
        self.active_submenu = Some(submenu);
        cx.notify();
    }

    fn close_submenu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_drag_location(cx);
        self.begin_motion(Self::default_motion_duration(), window, cx);
        self.hovered_action_id = None;
        self.active_submenu = None;
        cx.notify();
    }

    fn open_accessory_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request.accessory.is_none() {
            return;
        }
        ChatPerformanceTrace::shared().record("hold_menu.details_begin", HashMap::new());
        self.clear_drag_location(cx);
        self.begin_motion(Self::default_motion_duration(), window, cx);
        self.hovered_action_id = None;
        self.hovered_emoji = None;
        self.active_submenu = None;
        self.accessory_details_visible = true;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn close_accessory_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_drag_location(cx);
        self.begin_motion(Self::default_motion_duration(), window, cx);
        self.accessory_details_visible = false;
        window.focus(&self.list_focus_handle, cx);
        cx.notify();
    }

    fn handle_accessory_action(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        HoldContextMenuPresenter::note_close_imminent(cx);
        HoldContextMenuPresenter::perform_action(&id, self.request.id, window, cx);
        self.dismiss(window, cx);
    }

    /// row a lift off at this point would commit, or none
    /// one function answers for both highlight and release so what lights up is what letting go picks
    /// rows are checked in list order
    fn action_id(&self, at: Point<Pixels>, cx: &App) -> Option<String> {
        let actions = self.displayed_actions(cx);
        for action in actions.iter().filter(|action| !action.disabled) {
            let key = HoldContextMenuHitTargetsKey::Action(action.id.clone());
            if self.hit_targets.frame(&key).is_some_and(|frame| frame.contains(&at)) {
                return Some(action.id.clone());
            }
        }
        self.nearest_action_id(&actions, at)
    }

    fn nearest_action_id(&self, actions: &[HoldContextMenuAction], to: Point<Pixels>) -> Option<String> {
        let mut best: Option<(String, f32)> = None;
        for action in actions.iter().filter(|action| !action.disabled) {
            let key = HoldContextMenuHitTargetsKey::Action(action.id.clone());
            let Some(frame) = self.hit_targets.frame(&key) else { continue };
            if to.x < frame.left() || to.x > frame.right() {
                continue;
            }
            let distance = if to.y < frame.top() {
                f32::from(frame.top() - to.y)
            } else if to.y > frame.bottom() {
                f32::from(to.y - frame.bottom())
            } else {
                0.0
            };
            if distance > HoldContextMenuMetrics::ROW_SNAP_TOLERANCE {
                continue;
            }
            if best.as_ref().is_some_and(|(_, current)| *current <= distance) {
                continue;
            }
            best = Some((action.id.clone(), distance));
        }
        best.map(|(id, _)| id)
    }

    fn emoji_key(&self, at: Point<Pixels>) -> Option<String> {
        if self.active_submenu.is_some() || self.accessory_details_visible || self.emoji_picker_expanded {
            return None;
        }
        let mut keys = self.request.emojis.clone();
        if self.shows_emoji_expand() {
            keys.push(HoldContextMenuEmojiBar::EXPAND_KEY.to_owned());
        }
        let mut best: Option<(String, f32)> = None;
        for emoji in keys {
            let Some(frame) = self.hit_targets.frame(&HoldContextMenuHitTargetsKey::Emoji(emoji.clone())) else {
                continue;
            };
            if frame.contains(&at) {
                return Some(emoji);
            }
            let dx = f32::from(frame.left() - at.x).max(0.0).max(f32::from(at.x - frame.right()));
            let dy = f32::from(frame.top() - at.y).max(0.0).max(f32::from(at.y - frame.bottom()));
            let distance = dx.max(dy);
            if distance > HoldContextMenuMetrics::EMOJI_SNAP_TOLERANCE {
                continue;
            }
            if best.as_ref().is_some_and(|(_, current)| *current <= distance) {
                continue;
            }
            best = Some((emoji, distance));
        }
        best.map(|(key, _)| key)
    }

    fn refresh_hover(&mut self, cx: &mut Context<Self>) {
        if !self.is_active(cx) {
            self.motion_cadence.stop();
            return;
        }
        let point = if self.is_dismissing { None } else { self.presenter.read(cx).drag_location };
        if point.is_some() || self.is_dismissing {
            let action = point.and_then(|point| self.action_id(point, cx));
            let emoji = point.and_then(|point| self.emoji_key(point));
            if self.hovered_action_id != action || self.hovered_emoji != emoji {
                let action_adopted = self.hovered_action_id != action && action.is_some();
                let emoji_adopted = self.hovered_emoji != emoji && emoji.is_some();
                self.hovered_action_id = action;
                self.hovered_emoji = emoji;
                self.hover_did_adopt(action_adopted);
                self.hover_did_adopt(emoji_adopted);
                cx.notify();
            }
        }
        self.arm_release_selection_if_measured(cx);
    }

    fn default_motion_duration() -> Duration {
        seconds(HoldContextMenuMetrics::OPEN_RESPONSE + 0.2)
    }

    fn begin_motion(&mut self, duration: Duration, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let on_frame: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |_window: &mut Window, cx: &mut App| {
            this.update(cx, |this, cx| this.refresh_hover(cx)).ok();
        });
        self.motion_cadence.request(duration, Some(on_frame), window, cx);
    }

    fn arm_release_selection_if_measured(&mut self, cx: &App) {
        if self.release_selection_armed || self.is_dismissing {
            return;
        }
        let has_action = self.displayed_actions(cx).iter().any(|action| {
            self.hit_targets.frame(&HoldContextMenuHitTargetsKey::Action(action.id.clone())).is_some()
        });
        let has_emoji = self
            .request
            .emojis
            .iter()
            .any(|emoji| self.hit_targets.frame(&HoldContextMenuHitTargetsKey::Emoji(emoji.clone())).is_some());
        if has_action || has_emoji {
            self.release_selection_armed = true;
        }
    }

    /// a release only arrives for a pointer the presenter already judged to be aiming
    /// so only liveness and measurement are checked here
    fn handle_release(&mut self, event: HoldContextMenuReleaseEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_hover(cx);
        self.commit_release(&event, window, cx);
        HoldContextMenuPresenter::clear_pending_release(cx);
    }

    fn commit_release(&mut self, event: &HoldContextMenuReleaseEvent, window: &mut Window, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X2254", opened = self.menu_fully_opened, armed = self.release_selection_armed, dismissing = self.is_dismissing, location = ?event.location, "menu release");
        if !self.menu_fully_opened || !self.release_selection_armed || self.is_dismissing {
            return;
        }
        let Some(location) = event.location else { return };
        if let Some(emoji) = self.emoji_key(location) {
            if emoji == HoldContextMenuEmojiBar::EXPAND_KEY {
                self.expand_emoji_picker(window, cx);
                return;
            }
            let _ = impact(ImpactStyle::Medium);
            self.pick_emoji(emoji, window, cx);
            return;
        }
        if let Some(id) = self.action_id(location, cx) {
            let _ = selection_changed();
            self.handle_row_tap(id, window, cx);
        }
    }

    fn presenter_did_change(
        &mut self,
        presenter: &Entity<HoldContextMenuPresenter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let change = self.interaction_observer.observe(presenter.read(cx));
        if change.dismissal_requested.as_ref() == Some(&self.request.id) {
            self.dismiss(window, cx);
        }
        if change.dragged {
            self.refresh_hover(cx);
        }
        if let Some(event) = change.released {
            if self.last_handled_release.as_ref().is_some_and(|last| last.id == event.id) {
                return;
            }
            self.last_handled_release = Some(event);
            self.handle_release(event, window, cx);
        }
    }

    fn open_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_motion(
            seconds(HoldContextMenuMetrics::OPEN_RESPONSE.max(HoldContextMenuMetrics::BACKDROP_FADE_IN_DURATION) + 0.2),
            window,
            cx,
        );
        let on_lift_source = Rc::clone(&self.request.on_lift_source);
        on_lift_source(cx);
        let _ = self.hover_haptic.prepare();
        ChatPerformanceTrace::shared().record("hold_menu.open_begin", snapshot_field(&self.request));
        self.backdrop_revealed = false;
        if self.presenter.read(cx).dismissal_requested_id().as_ref() == Some(&self.request.id) {
            self.dismiss(window, cx);
            return;
        }
        HoldContextMenuWindowPresenter::remember_source_focus(window, cx);
        cx.defer_in(window, |this, window, cx| {
            if !this.is_active(cx) || this.is_dismissing {
                return;
            }
            this.appeared = true;
            this.current_lift_scale = 1.0;
            this.emoji_picker_appearance.set_in(1.0, close_spring(), RiseClock::monotonic(), cx);
            this.backdrop_revealed = true;
            this.menu_fully_opened = true;
            window.focus(&this.list_focus_handle, cx);
            cx.notify();
        });
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(seconds(HoldContextMenuMetrics::RELEASE_ARM_DELAY)).await;
            this.update(cx, |this, cx| {
                if !this.is_active(cx) || this.is_dismissing {
                    return;
                }
                this.release_selection_armed = true;
            })
            .ok();
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(seconds(HoldContextMenuMetrics::OPEN_RESPONSE)).await;
            this.update(cx, |this, cx| {
                if !this.is_active(cx) || this.is_dismissing {
                    return;
                }
                ChatPerformanceTrace::shared().record("hold_menu.open_end", snapshot_field(&this.request));
            })
            .ok();
        })
        .detach();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dismissing || self.is_committing_preview {
            return;
        }
        tracing::warn!(target: "riseonly::X2255", opened = self.menu_fully_opened, "menu dismiss begins");
        self.begin_motion(
            seconds(HoldContextMenuMetrics::CLOSE_RESPONSE * HoldContextMenuMetrics::SOURCE_RESTORE_DELAY_FACTOR + 0.2),
            window,
            cx,
        );
        self.return_center = self.presenter.read(cx).return_center(self.request.id);
        if self.appeared && !self.request.is_centered {
            let current = self
                .request
                .source_lift
                .as_ref()
                .and_then(|source_lift| source_lift.lifted_center_in_screen())
                .or_else(|| self.source_probe.center_in_screen());
            if let Some(current) = current {
                let target = self.return_center.unwrap_or(self.request.initial_source_center);
                self.return_shift = Some(point(target.x - current.x, target.y - current.y));
            }
        }
        self.is_dismissing = true;
        self.close_began_at = Some(RiseClock::monotonic());
        HoldContextMenuPresenter::note_dismiss_began(cx);
        ChatPerformanceTrace::shared().record("hold_menu.close_begin", snapshot_field(&self.request));
        self.menu_fully_opened = false;
        self.release_selection_armed = false;
        self.backdrop_revealed = false;
        self.emoji_picker_appearance.set_in(0.0, close_spring(), RiseClock::monotonic(), cx);
        self.appeared = false;
        let deadline = seconds(
            HoldContextMenuMetrics::CLOSE_RESPONSE * HoldContextMenuMetrics::SOURCE_RESTORE_DELAY_FACTOR + 1.0,
        );
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(deadline).await;
            let _ = this.update_in(cx, |this, window, cx| this.force_finish_dismiss(window, cx));
        })
        .detach();
        let block_limit = seconds(HoldContextMenuMetrics::CLOSE_TOUCH_BLOCK_LIMIT);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(block_limit).await;
            let _ = this.update_in(cx, |_, _, cx| cx.notify());
        })
        .detach();
        if self.presenter.read(cx).closes_without_motion() {
            self.defer_source_hand_back(window, cx);
            self.backdrop_close_finished = true;
            self.source_landing_finished = true;
            self.source_return_finished = true;
            self.finish_dismiss_if_ready(window, cx);
            return;
        }
        cx.notify();
    }

    fn force_finish_dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismissal_finished || !self.is_dismissing || !self.is_active(cx) {
            return;
        }
        ChatPerformanceTrace::shared().record("hold_menu.close_forced", snapshot_field(&self.request));
        tracing::warn!(
            target: "riseonly::X1161",
            backdrop = self.backdrop_close_finished,
            landing = self.source_landing_finished,
            returned = self.source_return_finished,
            "menu close forced by deadline"
        );
        self.backdrop_close_finished = true;
        self.source_landing_finished = true;
        self.source_return_finished = true;
        self.finish_dismiss_if_ready(window, cx);
    }

    fn source_motion_completed(&mut self, needs_scroll: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_active(cx) || !self.is_dismissing {
            return;
        }
        if self.source_handed_back {
            self.source_return_finished = true;
        } else {
            self.source_landing_finished = true;
            if needs_scroll || self.request.is_centered {
                self.source_return_finished = true;
            }
        }
        self.finish_dismiss_if_ready(window, cx);
    }

    fn finish_dismiss_if_ready(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismissal_finished
            || !self.is_active(cx)
            || !self.source_landing_finished
            || !self.backdrop_close_finished
        {
            return;
        }
        if !self.source_handed_back {
            self.hand_back_source(window, cx);
            if self.request.source_lift.is_some() || self.source_copy_hidden.get() {
                self.source_return_finished = true;
            }
        }
        if !self.source_return_finished {
            return;
        }
        self.dismissal_finished = true;
        ChatPerformanceTrace::shared().record("hold_menu.close_end", snapshot_field(&self.request));
        let on_dismiss = Rc::clone(&self.on_dismiss);
        on_dismiss(window, cx);
    }

    fn hand_back_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.source_handed_back {
            return;
        }
        self.source_handed_back = true;
        HoldContextMenuPresenter::restore_source(self.request.id, cx);
        self.source_opacity.set_in(
            0.0,
            MotionSpec::curve(CubicBezier::EASE_OUT, seconds(0.10)),
            RiseClock::monotonic(),
            cx,
        );
        cx.notify();
    }

    fn defer_source_hand_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.source_handed_back {
            return;
        }
        self.source_handed_back = true;
        HoldContextMenuPresenter::defer_source_restore(cx);
        let restore = Rc::clone(&self.request.on_restore_source);
        let source_window = HoldContextMenuWindowPresenter::shared(cx).read(cx).source_window();
        match source_window {
            Some(handle) if handle != window.window_handle() => {
                let restore_now = Rc::clone(&restore);
                let scheduled = handle.update(cx, move |_, window, _| {
                    window.on_next_frame(move |_, cx| restore(cx));
                });
                if scheduled.is_err() {
                    let app: &mut App = cx;
                    restore_now(app);
                }
            }
            _ => window.on_next_frame(move |_, cx| restore(cx)),
        }
    }

    fn record_resolved_layout(&self, menu_size: Size<Pixels>, window: &Window) {
        let viewport_height = f32::from(window.viewport_size().height);
        let available_height = (viewport_height - 12.0).max(0.0);
        let source_size = self.source_size();
        let menu_height = f32::from(menu_size.height);
        let needs_scroll = source_size.height + menu_height + HoldContextMenuMetrics::MENU_TO_SOURCE_GAP > available_height;
        let flag = |value: bool| if value { "1" } else { "0" }.to_owned();
        ChatPerformanceTrace::shared().record(
            "hold_menu.layout",
            HashMap::from([
                ("available_height".to_owned(), format!("{available_height:.2}")),
                ("menu_height".to_owned(), format!("{menu_height:.2}")),
                ("needs_scroll".to_owned(), flag(needs_scroll)),
                ("snapshot".to_owned(), flag(self.request.snapshot.is_some())),
                ("starts_at_bottom".to_owned(), flag(needs_scroll)),
                ("source_height".to_owned(), format!("{:.2}", source_size.height)),
            ]),
        );
    }

    fn estimated_emoji_size(&self) -> Size<Pixels> {
        let slots = self.request.emojis.len() + usize::from(self.shows_emoji_expand());
        size(
            px(slots as f32 * HoldContextMenuMetrics::EMOJI_SLOT_WIDTH
                + HoldContextMenuMetrics::EMOJI_BAR_HORIZONTAL_PADDING * 2.0),
            px(HoldContextMenuMetrics::EMOJI_SLOT_HEIGHT + HoldContextMenuMetrics::EMOJI_BAR_VERTICAL_PADDING * 2.0),
        )
    }

    fn compute_layout(
        &self,
        in_size: Size<Pixels>,
        safe_top: f32,
        safe_bottom: f32,
        cx: &App,
    ) -> HoldContextMenuOverlayLayout {
        let width = f32::from(in_size.width);
        let height = f32::from(in_size.height);
        let edge = HoldContextMenuMetrics::EDGE_INSET;
        let top_limit = safe_top + 6.0;
        let bottom_limit = height - safe_bottom - 6.0;
        let available = bottom_limit - top_limit;

        let source_frame = self.request.source_frame;
        let source_w = f32::from(source_frame.size.width);
        let source_h = self.source_size().height;
        let menu_w = if self.menu_panel_size.width > Pixels::ZERO {
            f32::from(self.menu_panel_size.width)
        } else {
            HoldContextMenuMetrics::MENU_MIN_WIDTH
        };
        let actions = self.resolved_actions(cx);
        let measured_menu_h = HoldContextMenuOverflowPreviewPolicy::resolved_menu_height(
            f32::from(self.menu_panel_size.height),
            &actions,
            self.accessory_height(),
        );
        let menu_h = measured_menu_h.min(self.menu_maximum_height());
        let emoji_size = self.estimated_emoji_size();
        let emoji_w = f32::from(emoji_size.width);
        let emoji_h = f32::from(emoji_size.height);

        let menu_gap = HoldContextMenuMetrics::MENU_TO_SOURCE_GAP;
        let emoji_gap = HoldContextMenuMetrics::EMOJI_TO_SOURCE_GAP;

        let source_x = if self.request.is_centered { (width - source_w) / 2.0 } else { f32::from(source_frame.left()) };
        let (mut menu_x, emoji_x) = match self.request.alignment {
            HoldContextMenuAlignment::Leading => (
                clamp(source_x, edge, width - edge - menu_w),
                clamp(source_x, edge, width - edge - emoji_w),
            ),
            HoldContextMenuAlignment::Trailing => {
                let trailing_edge = source_x + source_w;
                (
                    clamp(trailing_edge - menu_w, edge, width - edge - menu_w),
                    clamp(trailing_edge - emoji_w, edge, width - edge - emoji_w),
                )
            }
        };
        if let Some(anchor) = self.presenter.read(cx).menu_anchor() {
            let lower = source_x.max(edge);
            let upper = (source_x + source_w).min(width - edge) - menu_w;
            if !self.request.is_centered && source_w > menu_w && upper > lower {
                menu_x = clamp(f32::from(anchor.x), lower, upper);
            }
        }

        let picker_width = (width - edge * 2.0).min(HoldContextMenuMetrics::PICKER_MAX_WIDTH);
        let picker_left = if self.request.is_centered {
            (width - picker_width) / 2.0
        } else {
            match self.request.alignment {
                HoldContextMenuAlignment::Leading => clamp(source_x, edge, width - edge - picker_width),
                HoldContextMenuAlignment::Trailing => {
                    clamp(source_x + source_w - picker_width, edge, width - edge - picker_width)
                }
            }
        };
        let picker_x = picker_left + picker_width / 2.0;
        let picker_top_limit = top_limit.max(safe_top + edge);
        let picker_available = bottom_limit - picker_top_limit;
        let visible_source = source_h.min(110.0);
        let mut picker_height = HoldContextMenuMetrics::PICKER_MAX_HEIGHT
            .min(picker_available - HoldContextMenuMetrics::PICKER_TO_SOURCE_GAP - visible_source);
        picker_height = picker_height.max(HoldContextMenuMetrics::PICKER_MIN_HEIGHT.min(picker_available - 60.0));

        let opens_at_cursor = !self.request.is_centered && self.request.controller_preview.is_none();
        if let Some(layout) = self.peek_beside_cursor_layout(
            HoldContextMenuOverlayCursorFrame { width, edge, top_limit, bottom_limit, menu_w, menu_h },
            (emoji_x, picker_x, picker_top_limit, picker_width, picker_height),
            cx,
        ) {
            return layout;
        }
        if let Some(anchor) = self.presenter.read(cx).menu_anchor().filter(|_| opens_at_cursor) {
            let (anchor_x, anchor_y) = (f32::from(anchor.x), f32::from(anchor.y));
            let source_y = f32::from(source_frame.top());
            let shows_bar = self.request.show_emoji_list;
            let bar_band = if shows_bar { emoji_h + menu_gap } else { 0.0 };
            let block_w = if shows_bar { menu_w.max(emoji_w) } else { menu_w };
            let block_h = bar_band + menu_h;
            let opens_left = anchor_x + block_w > width - edge;
            let opens_up = anchor_y + block_h > bottom_limit;
            let block_x = clamp(
                if opens_left { anchor_x - block_w } else { anchor_x },
                edge,
                (width - edge - block_w).max(edge),
            );
            let block_y = clamp(
                if opens_up { anchor_y - block_h } else { anchor_y },
                top_limit,
                (bottom_limit - block_h).max(top_limit),
            );
            let (menu_x, emoji_x) = if opens_left {
                (block_x + block_w - menu_w, block_x + block_w - emoji_w)
            } else {
                (block_x, block_x)
            };
            return HoldContextMenuOverlayLayout {
                emoji_x,
                emoji_y: block_y,
                source_x,
                source_y,
                menu_x,
                menu_y: block_y + bar_band,
                needs_scroll: false,
                scroll_content_height: 0.0,
                picker_x,
                picker_y: picker_top_limit,
                picker_width,
                picker_height,
                expanded_source_y: source_y,
                scroll_content_height_expanded: 0.0,
                initial_scroll_offset_y: 0.0,
                emoji_overlap: false,
                at_cursor: true,
            };
        }

        if source_h + menu_h + menu_gap > available {
            return Self::scroll_layout(HoldContextMenuOverlayScrollInput {
                emoji_x,
                source_x,
                menu_x,
                emoji_h,
                source_h,
                menu_h,
                show_emoji: self.request.show_emoji_list,
                top_inset: HoldContextMenuOverflowPreviewPolicy::safe_scroll_inset(
                    HoldContextMenuMetrics::SCROLL_VERTICAL_INSET,
                    safe_top,
                ),
                bottom_inset: HoldContextMenuOverflowPreviewPolicy::safe_scroll_inset(
                    HoldContextMenuMetrics::SCROLL_VERTICAL_INSET,
                    safe_bottom,
                ),
                viewport_height: height,
                picker_x,
                picker_width,
                picker_height,
            });
        }

        let mut source_y = if self.request.is_centered {
            top_limit + (available - source_h - menu_gap - menu_h) / 2.0
        } else {
            f32::from(source_frame.top())
        };
        let mut menu_y = source_y + source_h + menu_gap;
        let mut emoji_y = source_y - emoji_gap - emoji_h;
        let mut emoji_overlap = false;

        let menu_bottom = menu_y + menu_h;
        let menu_above_y = source_y - menu_gap - menu_h;
        let tap_anchored = !self.request.is_centered
            && !self.request.show_emoji_list
            && self.request.controller_preview.is_none();
        let flips_above = tap_anchored && menu_above_y >= top_limit;
        if menu_bottom > bottom_limit && flips_above {
            menu_y = menu_above_y;
        } else if menu_bottom > bottom_limit && tap_anchored && self.request.source_lift.is_none() {
            let room_below = bottom_limit - (source_y + source_h + menu_gap);
            let room_above = source_y - menu_gap - top_limit;
            menu_y = if room_below >= room_above { bottom_limit - menu_h } else { top_limit };
            menu_y = menu_y.max(top_limit);
        } else if menu_bottom > bottom_limit {
            let shift = menu_bottom - bottom_limit;
            source_y -= shift;
            menu_y -= shift;
            emoji_y -= shift;
        }

        if self.request.show_emoji_list && emoji_y < top_limit {
            let need_down = top_limit - emoji_y;
            let menu_fits_after_down = menu_y + menu_h + need_down <= bottom_limit;
            if menu_fits_after_down {
                source_y += need_down;
                menu_y += need_down;
                emoji_y += need_down;
            } else {
                emoji_overlap = true;
                emoji_y = source_y + HoldContextMenuMetrics::EMOJI_OVERLAP_INSET;
            }
        }

        if source_y < top_limit {
            let need_down = top_limit - source_y;
            if menu_y + menu_h + need_down <= bottom_limit {
                source_y += need_down;
                menu_y += need_down;
                if !emoji_overlap {
                    emoji_y += need_down;
                }
            }
        }

        if emoji_overlap {
            emoji_y = clamp(emoji_y, top_limit, bottom_limit - emoji_h);
        }

        let picker_gap = HoldContextMenuMetrics::PICKER_TO_SOURCE_GAP;
        let expanded_source_y = source_y.max(picker_top_limit + picker_height + picker_gap);
        let picker_y = expanded_source_y - picker_gap - picker_height;

        HoldContextMenuOverlayLayout {
            emoji_x,
            emoji_y,
            source_x,
            source_y,
            menu_x,
            menu_y,
            needs_scroll: false,
            scroll_content_height: 0.0,
            picker_x,
            picker_y,
            picker_width,
            picker_height,
            expanded_source_y,
            scroll_content_height_expanded: 0.0,
            initial_scroll_offset_y: 0.0,
            emoji_overlap,
            at_cursor: false,
        }
    }

    fn peek_beside_cursor_layout(
        &self,
        frame: HoldContextMenuOverlayCursorFrame,
        picker: (f32, f32, f32, f32, f32),
        cx: &App,
    ) -> Option<HoldContextMenuOverlayLayout> {
        self.request.controller_preview.as_ref()?;
        let anchor = self.presenter.read(cx).menu_anchor()?;
        let HoldContextMenuOverlayCursorFrame { width, edge, top_limit, bottom_limit, menu_w, menu_h } = frame;
        let (emoji_x, picker_x, picker_y, picker_width, picker_height) = picker;
        let source = self.source_size();
        let gap = HoldContextMenuMetrics::MENU_TO_SOURCE_GAP;
        let (anchor_x, anchor_y) = (f32::from(anchor.x), f32::from(anchor.y));
        let menu_x = clamp(
            if anchor_x + menu_w <= width - edge { anchor_x } else { anchor_x - menu_w },
            edge,
            width - edge - menu_w,
        );
        let menu_y = clamp(
            if anchor_y + menu_h <= bottom_limit { anchor_y } else { anchor_y - menu_h },
            top_limit,
            (bottom_limit - menu_h).max(top_limit),
        );
        let right_x = menu_x + menu_w + gap;
        let left_x = menu_x - gap - source.width;
        let source_x = if right_x + source.width <= width - edge {
            right_x
        } else if left_x >= edge {
            left_x
        } else {
            return None;
        };
        let source_y = clamp(menu_y, top_limit, (bottom_limit - source.height).max(top_limit));
        Some(HoldContextMenuOverlayLayout {
            emoji_x,
            emoji_y: source_y,
            source_x,
            source_y,
            menu_x,
            menu_y,
            needs_scroll: false,
            scroll_content_height: 0.0,
            picker_x,
            picker_y,
            picker_width,
            picker_height,
            expanded_source_y: source_y,
            scroll_content_height_expanded: 0.0,
            initial_scroll_offset_y: 0.0,
            emoji_overlap: false,
            at_cursor: true,
        })
    }

    fn scroll_layout(input: HoldContextMenuOverlayScrollInput) -> HoldContextMenuOverlayLayout {
        let menu_gap = HoldContextMenuMetrics::MENU_TO_SOURCE_GAP;
        let emoji_gap = HoldContextMenuMetrics::EMOJI_TO_SOURCE_GAP;

        let source_y = input.top_inset + if input.show_emoji { input.emoji_h + emoji_gap } else { 0.0 };
        let emoji_y = input.top_inset;
        let menu_y = source_y + input.source_h + menu_gap;
        let scroll_content_height = menu_y + input.menu_h + input.bottom_inset;

        let expanded_source_y = source_y;

        HoldContextMenuOverlayLayout {
            emoji_x: input.emoji_x,
            emoji_y,
            source_x: input.source_x,
            source_y,
            menu_x: input.menu_x,
            menu_y,
            needs_scroll: true,
            scroll_content_height,
            picker_x: input.picker_x,
            picker_y: input.top_inset,
            picker_width: input.picker_width,
            picker_height: input.picker_height,
            expanded_source_y,
            scroll_content_height_expanded: scroll_content_height,
            initial_scroll_offset_y: HoldContextMenuOverflowPreviewPolicy::initial_scroll_offset(
                scroll_content_height,
                input.viewport_height,
            ),
            emoji_overlap: false,
            at_cursor: false,
        }
    }

    fn app_listener0(&self, cx: &Context<Self>, f: fn(&mut Self, &mut Window, &mut Context<Self>)) -> Rc<dyn Fn(&mut App)> {
        let this = cx.entity().downgrade();
        let window_handle = self.window_handle;
        Rc::new(move |cx: &mut App| {
            let this = this.clone();
            cx.defer(move |cx| {
                window_handle
                    .update(cx, |_, window, cx| {
                        this.update(cx, |this, cx| f(this, window, cx)).ok();
                    })
                    .ok();
            });
        })
    }

    fn app_listener1(
        &self,
        cx: &Context<Self>,
        f: fn(&mut Self, String, &mut Window, &mut Context<Self>),
    ) -> Rc<dyn Fn(String, &mut App)> {
        let this = cx.entity().downgrade();
        let window_handle = self.window_handle;
        Rc::new(move |value: String, cx: &mut App| {
            let this = this.clone();
            cx.defer(move |cx| {
                window_handle
                    .update(cx, |_, window, cx| {
                        this.update(cx, |this, cx| f(this, value, window, cx)).ok();
                    })
                    .ok();
            });
        })
    }
}

/// inputs of the overflow layout
struct HoldContextMenuOverlayScrollInput {
    emoji_x: f32,
    source_x: f32,
    menu_x: f32,
    emoji_h: f32,
    source_h: f32,
    menu_h: f32,
    show_emoji: bool,
    top_inset: f32,
    bottom_inset: f32,
    viewport_height: f32,
    picker_x: f32,
    picker_width: f32,
    picker_height: f32,
}

impl Focusable for HoldContextMenuOverlay {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HoldContextMenuOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let layout = self.compute_layout(viewport, 0.0, 0.0, cx);
        let now = RiseClock::monotonic();
        if self.is_dismissing
            && !self.dismissal_finished
            && let Some(began) = self.close_began_at
            && TopPresenter::top_presented_at(window, cx).is_some_and(|presented| presented >= began)
        {
            self.close_began_at = None;
            self.source_opacity.set_in(0.0, None::<MotionSpec>, now, cx);
            cx.defer_in(window, |this, window, cx| this.force_finish_dismiss(window, cx));
        }

        let occludes = !self.is_dismissing || self.presenter.read(cx).swallows_touches_while_closing();
        let mut root = div()
            .id("hold-context-menu-overlay")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .absolute()
            .top_0()
            .left_0()
            .w(viewport.width)
            .h(viewport.height)
            .when(occludes, |root| root.occlude())
            .on_action(cx.listener(|this, _: &DismissMenu, window, cx| {
                tracing::warn!(target: "riseonly::X2255", "menu dismiss key");
                this.dismiss(window, cx);
            }))
            .on_scroll_wheel(|_, _, cx| {
                if HoldContextMenuScrollLock::is_locked() {
                    cx.stop_propagation();
                }
            })
            .child(self.backdrop(layout.at_cursor, cx));

        if layout.needs_scroll || (self.request.is_centered && self.request.controller_preview.is_none()) {
            if layout.needs_scroll && !self.scroll_anchored {
                self.scroll_anchored = true;
                self.scroll_handle.set_offset(point(px(0.0), px(-layout.initial_scroll_offset_y)));
            }
            let content_height = if !layout.needs_scroll {
                f32::from(viewport.height)
            } else if self.emoji_picker_expanded {
                layout.scroll_content_height_expanded
            } else {
                layout.scroll_content_height
            };
            let dismiss_target = div()
                .id("hold-menu-overflow-dismiss")
                .absolute()
                .inset_0()
                .on_click(cx.listener(|this, _, window, cx| this.dismiss_from_backdrop(window, cx)));
            let content = div()
                .relative()
                .w(viewport.width)
                .h(px(content_height))
                .child(dismiss_target)
                .child(self.content_stack(layout, viewport, now, window, cx));
            let (mask_top, mask_height) = self.overflow_preview_mask(layout, viewport);
            HoldContextMenuTouchTracker::set_menu_scroll_region(layout.needs_scroll.then(|| {
                Bounds::new(point(px(0.0), px(mask_top)), size(viewport.width, px(mask_height)))
            }));
            let mut scroll = div()
                .id("hold-menu-overflow-scroll")
                .absolute()
                .left_0()
                .top(px(-mask_top))
                .w(viewport.width)
                .h(viewport.height);
            if layout.needs_scroll {
                scroll = scroll.overflow_y_scroll().track_scroll(&self.scroll_handle);
            }
            let band = div()
                .absolute()
                .left_0()
                .top(px(mask_top))
                .w(viewport.width)
                .h(px(mask_height))
                .overflow_hidden()
                .child(scroll.child(content));
            let band_on_top = layout.needs_scroll && !self.lift_landed && !self.is_dismissing;
            let mut band = Some(band);
            if !band_on_top {
                root = root.children(band.take());
            }
            if layout.needs_scroll {
                root = root.children(self.reaction_layers(layout, now, window, cx));
                root = root.child(self.action_list(layout, layout.initial_scroll_offset_y, cx));
            }
            root = root.children(band);
        } else {
            HoldContextMenuTouchTracker::set_menu_scroll_region(None);
            root = root.child(self.content_stack(layout, viewport, now, window, cx));
        }
        let scheme = if self.request.prefers_dark_chrome
            || ThemeStore::shared(cx).read(cx).window_interface_style() == RiseThemeAppearance::Dark
        {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        };
        env_scope::<ColorSchemeKey>(Some(scheme)).child(root)
    }
}

impl Drop for HoldContextMenuOverlay {
    fn drop(&mut self) {
        self.motion_cadence.stop();
        HoldContextMenuTouchTracker::set_menu_scroll_region(None);
    }
}

/// what changed on the presenter since the last look
struct HoldContextMenuInteractionChange {
    dragged: bool,
    released: Option<HoldContextMenuReleaseEvent>,
    dismissal_requested: Option<HoldContextMenuRequestID>,
}

/// pointer samples reach only this observer
/// the expensive overlay updates when the selected row changes
/// and not for every point the pointer reports
struct HoldContextMenuInteractionObserver {
    drag_location: Option<Point<Pixels>>,
    pending_release: Option<HoldContextMenuReleaseEvent>,
    dismissal_requested_id: Option<HoldContextMenuRequestID>,
}

impl HoldContextMenuInteractionObserver {
    fn new(presenter: &HoldContextMenuPresenter) -> Self {
        Self {
            drag_location: presenter.drag_location,
            pending_release: presenter.pending_release,
            dismissal_requested_id: presenter.dismissal_requested_id(),
        }
    }

    fn observe(&mut self, presenter: &HoldContextMenuPresenter) -> HoldContextMenuInteractionChange {
        let dragged = presenter.drag_location != self.drag_location;
        self.drag_location = presenter.drag_location;
        let released = (presenter.pending_release != self.pending_release)
            .then(|| presenter.pending_release)
            .flatten();
        self.pending_release = presenter.pending_release;
        let requested = presenter.dismissal_requested_id();
        let dismissal_requested = (requested != self.dismissal_requested_id).then_some(requested).flatten();
        self.dismissal_requested_id = requested;
        HoldContextMenuInteractionChange { dragged, released, dismissal_requested }
    }
}
