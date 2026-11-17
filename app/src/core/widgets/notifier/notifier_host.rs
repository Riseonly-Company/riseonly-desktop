use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnchoredPositionMode, App, BoxShadow, Bounds, Context, DispatchPhase, ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseUpEvent, ParentElement, PathBuilder, Pixels, Render, RenderOnce, ScrollDelta, ScrollWheelEvent,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, TouchPhase, Window, anchored, canvas, deferred, div, point, px,
};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_theme::{AppTheme, ThemeStore, alpha};
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::{LiquidChrome, LiquidChromeModifiers};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec, Spring, duration_from_seconds};
use rise_ui::swiftui::rich_text::{AttributeContainer, AttributedString, RichFont, RichText};
use rise_ui::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, vstack};

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::navigations::bottom_chrome_insets::{BottomChromeInsets, BottomChromePolicy};
use crate::core::widgets::notifier::notifier_markup_hooks::notifier_markup_hooks;
use crate::core::widgets::notifier::notifier_progress::{
    NotifierProgressLiveRing, NotifierProgressStore, NotifierProgressStoreEntry, NotifierProgressTitle,
};
use crate::core::widgets::notifier::notifier_store::{
    NotifierAuthor, NotifierConfig, NotifierIcon, NotifierStore, NotifierUndoPolicy,
};

const STACK_SPACING: f32 = 8.0;
pub const EDGE_PADDING: f32 = 8.0;
const ITEMS_MOTION: MotionSpec = MotionSpec::curve(CubicBezier::EASE_OUT, Duration::from_millis(200));
const DISMISS_MOTION: MotionSpec = MotionSpec::curve(CubicBezier::EASE_OUT, Duration::from_millis(160));
const RETURN_MOTION: MotionSpec = MotionSpec::spring(Spring::new(0.3, 0.78));
const TRANSITION_TRAVEL: f32 = 80.0;
const DRAG_MINIMUM_DISTANCE: f32 = 8.0;
const DISMISSED_DRAG_OFFSET: f32 = -160.0;
const DRAG_PREDICTION_SECONDS: f32 = 0.25;
const WHEEL_IDLE_END: Duration = Duration::from_millis(120);
const ISLAND_MAX_WIDTH: f32 = 560.0;
const ISLAND_MIN_WIDTH: f32 = 300.0;
const ISLAND_SIDE_PADDING: f32 = 14.0;
const GLYPH_SIDE: f32 = 30.0;
const CLOSE_BUTTON_SIDE: f32 = 20.0;
const CLOSE_BUTTON_OUTSET: f32 = 6.0;
const OVERLAY_HIDE_DELAY: Duration = Duration::from_millis(450);
const TRANSITION_RECHECK_PAUSES_MS: [u64; 4] = [200, 250, 350, 700];
const BODY_POINT_SIZE: f32 = 17.0;

/// every mounted notifier host, oldest first only the newest draws
/// otherwise one toast got drawn by two hosts and blinked
pub struct NotifierHostStack {
    tokens: Vec<Uuid>,
    retired: HashSet<Uuid>,
    overlay_token: Option<Uuid>,
}

impl NotifierHostStack {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            tokens: Vec::new(),
            retired: HashSet::new(),
            overlay_token: None,
        }
    }

    pub fn tokens(&self) -> &[Uuid] {
        &self.tokens
    }

    pub fn retired(&self) -> &HashSet<Uuid> {
        &self.retired
    }

    pub fn overlay_token(&self) -> Option<Uuid> {
        self.overlay_token
    }

    pub fn topmost(&self) -> Option<Uuid> {
        self.overlay_token
            .or_else(|| self.tokens.iter().rev().find(|token| !self.retired.contains(*token)).copied())
    }

    pub fn set_overlay(&mut self, token: Option<Uuid>, cx: &mut Context<Self>) {
        if self.overlay_token == token {
            return;
        }
        self.overlay_token = token;
        cx.notify();
    }

    pub fn push(&mut self, token: Uuid, cx: &mut Context<Self>) {
        if self.tokens.last() == Some(&token) {
            return;
        }
        self.tokens.retain(|known| *known != token);
        self.tokens.push(token);
        cx.notify();
    }

    pub fn remove(&mut self, token: Uuid, cx: &mut Context<Self>) {
        let had_token = self.tokens.contains(&token);
        if had_token {
            self.tokens.retain(|known| *known != token);
        }
        let was_retired = self.retired.remove(&token);
        if had_token || was_retired {
            cx.notify();
        }
    }

    pub fn retire(&mut self, token: Uuid, cx: &mut Context<Self>) {
        if self.retired.insert(token) {
            cx.notify();
        }
    }

    pub fn reinstate(&mut self, token: Uuid, cx: &mut Context<Self>) {
        if self.retired.remove(&token) {
            cx.notify();
        }
    }
}

/// notifier layer above every presentation
/// desktop root view already draws the overlay host above every sheet so that host is the window
pub struct NotifierWindowPresenter {
    is_hidden: bool,
    pending_hide: Option<Task<()>>,
    last_chrome_top: Option<Option<f32>>,
    transition_recheck: Option<Task<()>>,
    centre_column: Option<Bounds<Pixels>>,
}

impl NotifierWindowPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            is_hidden: true,
            pending_hide: None,
            last_chrome_top: None,
            transition_recheck: None,
            centre_column: None,
        }
    }

    pub fn is_hidden(&self) -> bool {
        self.is_hidden
    }

    pub fn centre_column(&self) -> Option<Bounds<Pixels>> {
        self.centre_column
    }

    pub fn set_centre_column(bounds: Option<Bounds<Pixels>>, cx: &mut App) {
        let presenter = Self::shared(cx);
        presenter.update(cx, |presenter, cx| {
            if presenter.centre_column != bounds {
                presenter.centre_column = bounds;
                cx.notify();
            }
        });
    }

    /// why: a progress banner outlives the screen it was sent from
    /// tab bar coming back moved no observed value so the island kept old clearance, this rechecks it
    pub fn recheck_bottom_chrome(cx: &mut App) {
        let presenter = Self::shared(cx);
        if presenter.read(cx).is_hidden {
            return;
        }
        let bottom_chrome = BottomChromeInsets::shared(cx);
        let top = bottom_chrome
            .read(cx)
            .visible_top_edge_in_window(BottomChromePolicy::NOTIFIER_EXCLUSION, true);
        let changed = presenter.update(cx, |presenter, _| {
            if presenter.last_chrome_top == Some(top) {
                return false;
            }
            presenter.last_chrome_top = Some(top);
            true
        });
        if changed {
            bottom_chrome.update(cx, |insets, cx| insets.chrome_layout_did_change(cx));
        }
    }

    pub fn recheck_bottom_chrome_after_transition(cx: &mut App) {
        let presenter = Self::shared(cx);
        presenter.update(cx, |presenter, cx| {
            let task = cx.spawn(async move |_, cx| {
                for pause in TRANSITION_RECHECK_PAUSES_MS {
                    cx.background_executor().timer(Duration::from_millis(pause)).await;
                    cx.update(|cx| Self::recheck_bottom_chrome(cx));
                }
            });
            presenter.transition_recheck = Some(task);
        });
    }

    pub fn sync(has_items: bool, cx: &mut App) {
        let presenter = Self::shared(cx);
        presenter.update(cx, |presenter, cx| {
            presenter.pending_hide = None;
            if has_items {
                if presenter.is_hidden {
                    presenter.is_hidden = false;
                    cx.notify();
                }
                return;
            }
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(OVERLAY_HIDE_DELAY).await;
                let _ = cx.update(|cx| {
                    let is_empty = NotifierStore::shared(cx).read(cx).items().is_empty();
                    if !is_empty {
                        return;
                    }
                    let _ = this.update(cx, |presenter, cx| {
                        presenter.pending_hide = None;
                        presenter.is_hidden = true;
                        cx.notify();
                    });
                });
            });
            presenter.pending_hide = Some(task);
        });
    }
}

pub enum NotifierSwipeDismissPolicy {}

impl NotifierSwipeDismissPolicy {
    pub fn should_dismiss(translation_height: f32, predicted_end_translation_height: f32) -> bool {
        translation_height <= -34.0 || predicted_end_translation_height <= -88.0
    }
}

struct DepartingIsland {
    config: NotifierConfig,
    progress: Animated<f32>,
}

pub struct NotifierHost {
    store: Entity<NotifierStore>,
    chrome_top: Option<f32>,
    host_stack: Entity<NotifierHostStack>,
    progress: Entity<NotifierProgressStore>,
    token: Uuid,
    is_overlay: bool,
    is_window_scoped: bool,
    host_bounds: Option<Bounds<Pixels>>,
    clearance: Animated<f32>,
    clearance_stale: bool,
    presented: Vec<NotifierConfig>,
    appearances: HashMap<Uuid, Animated<f32>>,
    departing: Vec<DepartingIsland>,
    logged_item: Option<Uuid>,
    _subscriptions: Vec<Subscription>,
}

impl NotifierHost {
    pub const LAYER_PRIORITY: usize = 2011;

    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_token(None, false, cx)
    }

    /// notifier host for a token, overlay host is the one the root draws above every sheet
    /// while it lives it is the only host that draws
    pub fn with_token(token: Option<Uuid>, is_overlay: bool, cx: &mut Context<Self>) -> Self {
        Self::build(token, is_overlay, false, cx)
    }

    pub fn window_scoped(cx: &mut Context<Self>) -> Self {
        Self::build(None, false, true, cx)
    }

    fn build(token: Option<Uuid>, is_overlay: bool, is_window_scoped: bool, cx: &mut Context<Self>) -> Self {
        let token = token.unwrap_or_else(Uuid::new_v4);
        let store = NotifierStore::shared(cx);
        let bottom_chrome = BottomChromeInsets::shared(cx);
        let chrome_top = if is_window_scoped {
            None
        } else {
            bottom_chrome.read(cx).visible_top_edge_in_window(BottomChromePolicy::NOTIFIER_EXCLUSION, true)
        };
        let host_stack = NotifierHostStack::shared(cx);
        let progress = NotifierProgressStore::shared(cx);
        let release_stack = host_stack.clone();
        let subscriptions = vec![
            cx.observe(&store, |host, store, cx| {
                let items = store.read(cx).items().to_vec();
                let was_empty = host.presented.is_empty();
                host.items_changed(items, cx);
                let is_empty = host.presented.is_empty();
                if was_empty != is_empty {
                    NotifierWindowPresenter::sync(!is_empty, cx);
                }
                cx.notify();
            }),
            cx.observe(&bottom_chrome, |host, chrome, cx| {
                if host.is_window_scoped {
                    return;
                }
                let top = chrome.read(cx).visible_top_edge_in_window(BottomChromePolicy::NOTIFIER_EXCLUSION, true);
                if host.chrome_top != top {
                    host.chrome_top = top;
                    if host.presented.is_empty() && host.departing.is_empty() {
                        host.clearance_stale = true;
                        return;
                    }
                    cx.notify();
                }
            }),
            cx.observe(&host_stack, |_, _, cx| cx.notify()),
            cx.observe(&progress, |_, _, cx| cx.notify()),
            cx.on_release(move |host: &mut Self, cx: &mut App| {
                let token = host.token;
                let is_overlay = host.is_overlay;
                release_stack.update(cx, |stack, cx| {
                    stack.remove(token, cx);
                    if is_overlay && stack.overlay_token() == Some(token) {
                        stack.set_overlay(None, cx);
                    }
                });
            }),
        ];
        let presented = store.read(cx).items().to_vec();
        if !is_window_scoped {
            host_stack.update(cx, |stack, cx| {
                stack.push(token, cx);
                if is_overlay {
                    stack.set_overlay(Some(token), cx);
                }
            });
        }
        NotifierWindowPresenter::sync(!presented.is_empty(), cx);
        Self {
            store,
            chrome_top,
            host_stack,
            progress,
            token,
            is_overlay,
            is_window_scoped,
            host_bounds: None,
            clearance: Animated::new(0.0),
            clearance_stale: false,
            presented,
            appearances: HashMap::new(),
            departing: Vec::new(),
            logged_item: None,
            _subscriptions: subscriptions,
        }
    }

    fn items_changed(&mut self, items: Vec<NotifierConfig>, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        let reduce = reduce_motion(cx);
        for previous in &self.presented {
            if items.iter().any(|item| item.id == previous.id) {
                continue;
            }
            self.appearances.remove(&previous.id);
            let mut progress = Animated::new(0.0);
            progress.set(1.0, ITEMS_MOTION, now, reduce);
            self.departing.push(DepartingIsland { config: previous.clone(), progress });
        }
        for item in &items {
            if self.presented.iter().any(|previous| previous.id == item.id) {
                continue;
            }
            let mut appearance = Animated::new(0.0);
            appearance.set(1.0, ITEMS_MOTION, now, reduce);
            self.appearances.insert(item.id, appearance);
        }
        self.presented = items;
    }

    pub fn token(&self) -> Uuid {
        self.token
    }

    pub fn is_overlay(&self) -> bool {
        self.is_overlay
    }

    fn window_drag_band(cx: &mut App) -> Pixels {
        ThemeStore::shared(cx).read(cx).current_theme().shell.window_drag_height
    }

    fn island(&self, item: &NotifierConfig, cx: &mut Context<Self>) -> NotifierIsland {
        let id = item.id;
        let store = self.store.clone();
        let activate_store = store.clone();
        let undo_store = store.clone();
        let hover_store = store.clone();
        NotifierIsland {
            config: item.clone(),
            progress: self.progress.read(cx).entry(id).cloned(),
            progress_store: self.progress.clone(),
            on_activate: Rc::new(move |_: &mut Window, cx: &mut App| activate_store.update(cx, |store, cx| store.activate(id, cx))),
            on_undo: Rc::new(move |_: &mut Window, cx: &mut App| undo_store.update(cx, |store, cx| store.undo(id, cx))),
            on_dismiss: Rc::new(move |_: &mut Window, cx: &mut App| store.update(cx, |store, cx| store.dismiss(id, cx))),
            on_hover: Rc::new(move |hovered: bool, cx: &mut App| {
                hover_store.update(cx, |store, cx| {
                    if hovered {
                        store.hold(id);
                    } else {
                        store.release_hold(id, cx);
                    }
                });
            }),
            theme: ThemeStore::shared(cx).read(cx).current_theme().clone(),
        }
    }

    /// why: every toast keeps one place at the top center with a constant gap
    /// position and offset from the caller no longer move it
    fn placed(island: impl IntoElement, travel: f32, opacity: f32) -> gpui::Div {
        div().w_full().flex().justify_center().relative().top(px(travel)).opacity(opacity).child(island)
    }
}

impl Render for NotifierHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let is_topmost = self.is_window_scoped || self.host_stack.read(cx).topmost() == Some(self.token);
        let items = self.store.read(cx).items().to_vec();

        let clearance_target = self.host_bounds.map_or(0.0, |bounds| {
            BottomChromePolicy::clearance(
                f32::from(bounds.bottom()),
                f32::from(bounds.size.height),
                self.chrome_top,
                BottomChromePolicy::NOTIFIER_GAP,
            )
        });
        if std::mem::take(&mut self.clearance_stale) {
            self.clearance.jump(clearance_target);
        }
        self.clearance.set(clearance_target, BottomChromePolicy::clearance_motion(), now, reduce_motion(cx));
        let clearance = self.clearance.frame_value(now, window);

        self.departing.retain(|departing| departing.progress.is_running(now));
        self.appearances.retain(|_, appearance| appearance.is_running(now));

        let host = cx.entity().downgrade();
        let bounds_probe = measure("notifier-host-bounds", move |bounds, _, cx| {
            let _ = host.update(cx, |host, cx| {
                if host.host_bounds != Some(bounds) {
                    host.host_bounds = Some(bounds);
                    cx.notify();
                }
            });
        });
        if !is_topmost && self.is_overlay && !items.is_empty() {
            let topmost = self.host_stack.read(cx).topmost();
            tracing::warn!(target: "riseonly::X1428", ?topmost, items = items.len(), "overlay notifier is not topmost");
        }
        if !is_topmost {
            return div().id("notifier-host").size_full().child(bounds_probe);
        }

        let mut top = vstack(STACK_SPACING).w_full();
        for item in &items {
            let appear = self.appearances.get(&item.id).map_or(1.0, |appearance| appearance.frame_value(now, window));
            top = top.child(Self::placed(self.island(item, cx), -TRANSITION_TRAVEL * (1.0 - appear), appear));
        }
        for departing in &self.departing {
            let gone = departing.progress.frame_value(now, window);
            top = top.child(Self::placed(self.island(&departing.config, cx), -TRANSITION_TRAVEL * gone, 1.0 - gone));
        }

        let window_size = window.viewport_size();
        let window_top = f32::from(Self::window_drag_band(cx)) + EDGE_PADDING;
        if !self.appearances.is_empty() || !self.departing.is_empty() {
            let appear: Vec<f32> = self.appearances.values().map(|appearance| appearance.value(now)).collect();
            tracing::warn!(
                target: "riseonly::X2032",
                window = ?window.window_handle().window_id(),
                active = window.is_window_active(),
                scoped = self.is_window_scoped,
                ?appear,
                departing = self.departing.len(),
                "notifier motion frame"
            );
        }
        if (self.is_overlay || self.is_window_scoped) && items.first().map(|item| item.id) != self.logged_item {
            self.logged_item = items.first().map(|item| item.id);
            tracing::warn!(
                target: "riseonly::X1428",
                window = ?window.window_handle().window_id(),
                scoped = self.is_window_scoped,
                item = ?self.logged_item,
                is_topmost,
                host_bounds = ?self.host_bounds,
                clearance,
                "overlay notifier drew items"
            );
        }
        if self.is_overlay || self.is_window_scoped {
            let window_width = f32::from(window_size.width);
            let (top_shift, centre_shift) = self.host_bounds.map_or((window_top, 0.0), |bounds| {
                let host_centre = f32::from(bounds.left()) + f32::from(bounds.size.width) / 2.0;
                (window_top - f32::from(bounds.top()), window_width / 2.0 - host_centre)
            });
            return div()
                .id("notifier-host")
                .size_full()
                .flex()
                .flex_col()
                .child(top.relative().top(px(top_shift)).left(px(centre_shift)))
                .child(bounds_probe);
        }
        let islands = div()
            .w(window_size.width)
            .h(window_size.height)
            .flex()
            .flex_col()
            .child(top.relative().top(px(window_top)));
        div()
            .id("notifier-host")
            .size_full()
            .child(
                deferred(
                    anchored()
                        .position_mode(AnchoredPositionMode::Window)
                        .position(point(px(0.), px(0.)))
                        .child(islands),
                )
                .with_priority(Self::LAYER_PRIORITY),
            )
            .child(bounds_probe)
    }
}

#[derive(IntoElement)]
struct NotifierIsland {
    config: NotifierConfig,
    progress: Option<NotifierProgressStoreEntry>,
    progress_store: Entity<NotifierProgressStore>,
    on_activate: Rc<dyn Fn(&mut Window, &mut App)>,
    on_undo: Rc<dyn Fn(&mut Window, &mut App)>,
    on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
    on_hover: Rc<dyn Fn(bool, &mut App)>,
    theme: AppTheme,
}

struct NotifierIslandState {
    drag_offset: Animated<f32>,
    press_y: Option<f32>,
    is_dragging: bool,
    last_sample: Option<(f32, Duration)>,
    velocity: f32,
    wheel_translation: f32,
    wheel_generation: u64,
    wheel_inertia_tail: bool,
    wheel_idle: Option<Task<()>>,
    is_hovered: bool,
}

impl NotifierIsland {
    fn is_avatar_notification(&self) -> bool {
        matches!(self.config.icon, NotifierIcon::Avatar { .. })
    }

    fn has_button(&self) -> bool {
        self.config.undo.is_some() || self.progress.is_some()
    }

    fn corner_radius(&self) -> f32 {
        if self.has_button() {
            return 26.0;
        }
        if self.is_avatar_notification() { 28.0 } else { 20.0 }
    }

    fn horizontal_padding(&self) -> f32 {
        if self.has_button() {
            return 16.0;
        }
        if self.is_avatar_notification() { 12.0 } else { 16.0 }
    }

    fn vertical_padding(&self) -> f32 {
        if self.has_button() {
            return 12.0;
        }
        if self.is_avatar_notification() { 10.0 } else { 13.0 }
    }

    fn drag_opacity(drag_offset: f32) -> f32 {
        (1.0 - drag_offset.abs() / 180.0).max(0.25)
    }

    /// island backing comes from the theme so text color has to as well, white text vanished on light theme
    fn text_content(&self) -> gpui::Div {
        let theme = &self.theme;
        let mut column = vstack(2.0).align(HorizontalAlignment::Leading).flex_1().min_w(px(0.0));
        let content = self.config.content.as_ref().filter(|content| !content.is_empty());
        let id = self.config.id.uuid_string();
        if self.is_avatar_notification() {
            column = column.child(
                MainText::new(self.config.title.clone())
                    .px(BODY_POINT_SIZE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary)
                    .number_of_lines(1),
            );
            let lead = self.config.author.as_ref().map(|author| Self::author_lead(author, theme));
            if let Some(content) = content {
                if self.config.content_is_markup
                    && let Some(hooks) = notifier_markup_hooks()
                {
                    column = column.child((hooks.preview_text)(
                        ElementId::Name(format!("notifier-markup-{id}").into()),
                        content.clone(),
                        BODY_POINT_SIZE,
                        theme.text.secondary,
                        2,
                        lead,
                    ));
                } else {
                    let text = lead.unwrap_or_default() + AttributedString::new(content.clone());
                    column = column.child(
                        Self::body_text(theme.text.secondary, 2)
                            .child(RichText::new(ElementId::Name(format!("notifier-content-{id}").into()), text)),
                    );
                }
            } else if let Some(lead) = lead {
                column = column.child(
                    Self::body_text(theme.text.primary, 1)
                        .child(RichText::new(ElementId::Name(format!("notifier-lead-{id}").into()), lead)),
                );
            }
        } else if let Some(template) = self.progress.as_ref().and_then(|progress| progress.title_template.clone()) {
            column = column.child(NotifierProgressTitle {
                id: self.config.id,
                template,
                color: theme.text.primary,
                number_of_lines: 5,
            });
        } else {
            column = column.child(
                MainText::new(self.config.title.clone())
                    .px(15.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary)
                    .number_of_lines(if self.has_button() { 5 } else { 3 }),
            );
            if let Some(content) = content {
                column = column.child(
                    MainText::new(content.clone())
                        .px(13.0)
                        .color(theme.text.secondary)
                        .number_of_lines(8),
                );
            }
        }
        column
    }

    /// body text with a line limit, around an attributed string
    fn body_text(color: Hsla, line_limit: usize) -> gpui::Div {
        let mut text = div().w_full().min_w(px(0.0)).swiftui_font(Font::body()).text_color(color).text_ellipsis();
        text.text_style().line_clamp = Some(line_limit);
        text
    }

    fn author_lead(author: &NotifierAuthor, theme: &AppTheme) -> AttributedString {
        let tag = author.tag.as_deref().map(|tag| tag.trim_matches('@')).unwrap_or_default();
        let title = if author.name.is_empty() && !tag.is_empty() {
            format!("@{tag}")
        } else {
            author.name.clone()
        };
        AttributedString::with_attributes(
            format!("{title}: "),
            AttributeContainer {
                font: Some(RichFont::system_weight(BODY_POINT_SIZE, FontWeight::SEMIBOLD)),
                foreground_color: Some(theme.text.primary),
                ..AttributeContainer::default()
            },
        )
    }

    fn author_badge(&self, author: &NotifierAuthor) -> gpui::Div {
        div()
            .absolute()
            .right(px(-3.0))
            .bottom(px(-3.0))
            .p(px(1.5))
            .rounded_full()
            .bg(LiquidChrome::surface_fill(&self.theme, 0))
            .child(
                UserLogoUi::new(ElementId::Name(format!("notifier-author-{}", self.config.id.uuid_string()).into()))
                    .source(author.avatar_url.clone().map(SharedString::from))
                    .name(Some(SharedString::from(author.name.clone())))
                    .user_id(author.user_id.clone().map(SharedString::from))
                    .size(20.0)
                    .show_border(false)
                    .show_story_ring(false)
                    .open_story_on_press(false),
            )
    }

    fn icon(&self) -> gpui::AnyElement {
        if self.progress.is_some() {
            return NotifierProgressLiveRing { id: self.config.id }.into_any_element();
        }
        if let Some(undo) = self.config.undo.as_ref().filter(|undo| undo.shows_countdown) {
            return NotifierUndoRing {
                id: ElementId::Name(
                    format!("notifier-undo-ring-{}-{}", self.config.id.uuid_string(), undo.deadline.ms()).into(),
                ),
                deadline: undo.deadline,
                window: self.config.duration,
                ink: self.theme.text.primary,
            }
            .into_any_element();
        }
        self.icon_glyph()
    }

    fn icon_glyph(&self) -> gpui::AnyElement {
        match &self.config.icon {
            NotifierIcon::None | NotifierIcon::Lottie(_) => self.type_glyph(),
            NotifierIcon::Emoji(value) => div()
                .size(px(GLYPH_SIDE))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(24.0))
                .child(SharedString::from(value.clone()))
                .into_any_element(),
            NotifierIcon::System(name) => Self::glyph(name, self.config.r#type.accent_color()),
            NotifierIcon::Avatar { source, name, user_id } => {
                let mut avatar = div().relative().child(
                    UserLogoUi::new(ElementId::Name(format!("notifier-avatar-{}", self.config.id.uuid_string()).into()))
                        .source(source.clone().map(SharedString::from))
                        .name(name.clone().map(SharedString::from))
                        .user_id(user_id.clone().map(SharedString::from))
                        .size(48.0)
                        .show_border(false)
                        .show_story_ring(false)
                        .open_story_on_press(false),
                );
                if let Some(author) = self.config.author.as_ref() {
                    avatar = avatar.child(self.author_badge(author));
                }
                avatar.into_any_element()
            }
        }
    }

    fn type_glyph(&self) -> gpui::AnyElement {
        Self::glyph(self.config.r#type.default_symbol(), self.config.r#type.accent_color())
    }

    fn glyph(symbol: &str, tint: Hsla) -> gpui::AnyElement {
        let slot = div().size(px(GLYPH_SIDE)).flex().items_center().justify_center();
        match IconUi::sized(symbol, px(22.0), tint, symbol.ends_with(".fill")) {
            Some(glyph) => slot.child(glyph).into_any_element(),
            None => slot.into_any_element(),
        }
    }

    fn end_drag(
        state: &Entity<NotifierIslandState>,
        translation: f32,
        on_dismiss: &Rc<dyn Fn(&mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let now = RiseClock::monotonic();
        let reduce = reduce_motion(cx);
        let velocity = state.read(cx).velocity;
        let predicted = translation + velocity * DRAG_PREDICTION_SECONDS;
        let should_dismiss = NotifierSwipeDismissPolicy::should_dismiss(translation, predicted);
        state.update(cx, |state, cx| {
            state.press_y = None;
            state.is_dragging = false;
            state.last_sample = None;
            state.velocity = 0.0;
            state.wheel_translation = 0.0;
            state.wheel_generation = state.wheel_generation.wrapping_add(1);
            if should_dismiss {
                state.drag_offset.set(DISMISSED_DRAG_OFFSET, DISMISS_MOTION, now, reduce);
            } else {
                state.drag_offset.set(0.0, RETURN_MOTION, now, reduce);
            }
            cx.notify();
        });
        if should_dismiss {
            on_dismiss(window, cx);
        }
    }
}

impl RenderOnce for NotifierIsland {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let state = window.use_keyed_state(
            ElementId::Name(format!("notifier-island-{}", self.config.id.uuid_string()).into()),
            cx,
            |_, _| NotifierIslandState {
                drag_offset: Animated::new(0.0),
                press_y: None,
                is_dragging: false,
                last_sample: None,
                velocity: 0.0,
                wheel_translation: 0.0,
                wheel_generation: 0,
                wheel_inertia_tail: false,
                wheel_idle: None,
                is_hovered: false,
            },
        );
        let drag_offset = state.read(cx).drag_offset.frame_value(now, window);
        let corner_radius = self.corner_radius();
        let theme = &self.theme;

        let island_id = self.config.id;
        let mut row = hstack(12.0).w_full().child(self.icon()).child(self.text_content());
        if let Some(undo) = self.config.undo.as_ref() {
            let release_undo = self.on_undo.clone();
            let capture_undo = self.on_undo.clone();
            let capture_pressed = window.use_keyed_state(
                ElementId::Name(format!("notifier-undo-capture-{}", island_id.uuid_string()).into()),
                cx,
                |_, _| false,
            );
            let undo_catcher = canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let down_pressed = capture_pressed.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                        if phase != DispatchPhase::Capture
                            || event.button != MouseButton::Left
                            || !bounds.contains(&event.position)
                        {
                            return;
                        }
                        tracing::warn!(target: "riseonly::X971", "island undo capture down at {:?}", event.position);
                        down_pressed.update(cx, |pressed, _| *pressed = true);
                        cx.stop_propagation();
                    });
                    let up_pressed = capture_pressed.clone();
                    let undo = capture_undo.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                            return;
                        }
                        let was_pressed = up_pressed.update(cx, |pressed, _| std::mem::replace(pressed, false));
                        if !was_pressed || !bounds.contains(&event.position) {
                            return;
                        }
                        tracing::warn!(target: "riseonly::X971", "island undo capture release");
                        cx.stop_propagation();
                        undo(window, cx);
                    });
                },
            )
            .absolute()
            .inset_0();
            row = row.child(
                div()
                    .id(ElementId::Name(format!("notifier-undo-wrap-{}", island_id.uuid_string()).into()))
                    .relative()
                    .on_mouse_down(MouseButton::Left, |event, _, cx| {
                        tracing::warn!(target: "riseonly::X971", "island undo down at {:?}", event.position);
                        cx.stop_propagation();
                    })
                    .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                        tracing::warn!(target: "riseonly::X971", "island undo release");
                        cx.stop_propagation();
                        release_undo(window, cx);
                    })
                    .child(NotifierUndoButton {
                        id: ElementId::Name(format!("notifier-undo-{}", island_id.uuid_string()).into()),
                        title: undo.action_title.clone(),
                        color: theme.primary._100,
                        on_undo: self.on_undo.clone(),
                    })
                    .child(undo_catcher),
            );
        } else if let Some(action_title) = self.progress.as_ref().and_then(|progress| progress.action_title.clone()) {
            let cancel_store = self.progress_store.clone();
            let cancel: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
                cancel_store.update(cx, |store, cx| store.cancel(island_id, cx));
            });
            let press_cancel = cancel.clone();
            row = row.child(
                div()
                    .id(ElementId::Name(format!("notifier-progress-cancel-wrap-{}", island_id.uuid_string()).into()))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                        tracing::warn!(target: "riseonly::X807", "island cancel release");
                        cx.stop_propagation();
                        cancel(cx);
                    })
                    .child(NotifierUndoButton {
                        id: ElementId::Name(format!("notifier-progress-cancel-{}", island_id.uuid_string()).into()),
                        title: action_title,
                        color: theme.primary._100,
                        on_undo: Rc::new(move |_: &mut Window, cx: &mut App| press_cancel(cx)),
                    }),
            );
        }
        let surface = row
            .px(px(self.horizontal_padding()))
            .py(px(self.vertical_padding()))
            .bg(LiquidChrome::surface_fill(&theme, 0))
            .rounded(px(corner_radius))
            .liquid_chrome_rounded_rectangle_with(corner_radius, 0.12, None, self.config.action.is_some());

        let has_undo = self.config.undo.is_some();
        let has_progress = self.progress.is_some();
        let progress_store = self.progress_store.clone();
        let hide_progress: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            if has_progress {
                progress_store.update(cx, |store, _| store.hide(island_id));
            }
        });
        let tap_hide = hide_progress.clone();
        let user_dismiss = |on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>| -> Rc<dyn Fn(&mut Window, &mut App)> {
            let hide = hide_progress.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                hide(cx);
                on_dismiss(window, cx);
            })
        };
        let down_state = state.clone();
        let move_state = state.clone();
        let up_state = state.clone();
        let out_state = state.clone();
        let wheel_state = state.clone();
        let on_activate = self.on_activate.clone();
        let up_dismiss = user_dismiss(self.on_dismiss.clone());
        let out_dismiss = user_dismiss(self.on_dismiss.clone());
        let wheel_dismiss = user_dismiss(self.on_dismiss.clone());
        let close_dismiss = user_dismiss(self.on_dismiss.clone());
        let hover_state = state.clone();
        let on_hover = self.on_hover.clone();
        let is_hovered = state.read(cx).is_hovered;
        let close_button = is_hovered.then(|| {
            div()
                .id(ElementId::Name(format!("notifier-close-wrap-{}", island_id.uuid_string()).into()))
                .absolute()
                .top(px(-CLOSE_BUTTON_OUTSET))
                .left(px(-CLOSE_BUTTON_OUTSET))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    plain(ElementId::Name(format!("notifier-close-{}", island_id.uuid_string()).into()))
                        .on_press(move |window, cx| close_dismiss(window, cx))
                        .size(px(CLOSE_BUTTON_SIDE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(LiquidChrome::surface_fill(&theme, 0))
                        .border_1()
                        .border_color(alpha(theme.text.primary, 0.18))
                        .children(IconUi::sized("xmark", px(10.0), theme.text.primary, false)),
                )
        });

        div()
            .id(ElementId::Name(format!("notifier-island-hit-{}", island_id.uuid_string()).into()))
            .min_w(px(ISLAND_MIN_WIDTH + ISLAND_SIDE_PADDING * 2.0))
            .max_w(px(ISLAND_MAX_WIDTH + ISLAND_SIDE_PADDING * 2.0))
            .px(px(ISLAND_SIDE_PADDING))
            .occlude()
            .child(
                div()
                    .w_full()
                    .relative()
                    .top(px(drag_offset))
                    .opacity(Self::drag_opacity(drag_offset))
                    .rounded(px(corner_radius))
                    .shadow(vec![BoxShadow {
                        color: gpui::black().opacity(0.28),
                        offset: point(px(0.0), px(8.0)),
                        blur_radius: px(18.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .child(surface)
                    .children(close_button),
            )
            .on_hover(move |hovered: &bool, _, cx| {
                let hovered = *hovered;
                hover_state.update(cx, |state, cx| {
                    if state.is_hovered != hovered {
                        state.is_hovered = hovered;
                        cx.notify();
                    }
                });
                on_hover(hovered, cx);
            })
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                if has_undo {
                    tracing::warn!(target: "riseonly::X971", "island down outside undo at {:?}", event.position);
                }
                let y = f32::from(event.position.y);
                down_state.update(cx, |state, _| {
                    state.press_y = Some(y);
                    state.is_dragging = false;
                    state.last_sample = Some((y, RiseClock::monotonic()));
                    state.velocity = 0.0;
                });
            })
            .on_mouse_move(move |event, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let y = f32::from(event.position.y);
                move_state.update(cx, |state, cx| {
                    let Some(press_y) = state.press_y else {
                        return;
                    };
                    let translation = y - press_y;
                    if !state.is_dragging && translation.abs() < DRAG_MINIMUM_DISTANCE {
                        return;
                    }
                    state.is_dragging = true;
                    let sampled_at = RiseClock::monotonic();
                    if let Some((last_y, last_at)) = state.last_sample {
                        let elapsed = sampled_at.saturating_sub(last_at).as_secs_f32();
                        if elapsed > 0.0 {
                            state.velocity = (y - last_y) / elapsed;
                        }
                    }
                    state.last_sample = Some((y, sampled_at));
                    state.drag_offset.jump(translation.min(0.0));
                    cx.notify();
                });
            })
            .on_mouse_up(MouseButton::Left, move |event, window, cx| {
                let (press_y, is_dragging) = {
                    let state = up_state.read(cx);
                    (state.press_y, state.is_dragging)
                };
                let Some(press_y) = press_y else {
                    return;
                };
                if is_dragging {
                    Self::end_drag(&up_state, f32::from(event.position.y) - press_y, &up_dismiss, window, cx);
                    return;
                }
                up_state.update(cx, |state, _| state.press_y = None);
                if !has_undo {
                    tap_hide(cx);
                    on_activate(window, cx);
                }
            })
            .on_mouse_up_out(MouseButton::Left, move |event, window, cx| {
                let (press_y, is_dragging) = {
                    let state = out_state.read(cx);
                    (state.press_y, state.is_dragging)
                };
                let Some(press_y) = press_y else {
                    return;
                };
                if is_dragging {
                    Self::end_drag(&out_state, f32::from(event.position.y) - press_y, &out_dismiss, window, cx);
                } else {
                    out_state.update(cx, |state, _| state.press_y = None);
                }
            })
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                let is_pixels = matches!(event.delta, ScrollDelta::Pixels(_));
                match event.touch_phase {
                    TouchPhase::Started => wheel_state.update(cx, |state, _| state.wheel_inertia_tail = false),
                    TouchPhase::Moved if is_pixels && wheel_state.read(cx).wheel_inertia_tail => return,
                    TouchPhase::Cancelled => {
                        wheel_state.update(cx, |state, _| {
                            state.wheel_inertia_tail = false;
                            state.wheel_idle = None;
                        });
                        Self::end_drag(&wheel_state, 0.0, &wheel_dismiss, window, cx);
                        return;
                    }
                    _ => {}
                }
                let delta = -f32::from(event.delta.pixel_delta(window.line_height()).y).abs();
                let translation = if is_pixels {
                    wheel_state.read(cx).wheel_translation + delta
                } else if delta < 0.0 {
                    DISMISSED_DRAG_OFFSET
                } else {
                    wheel_state.read(cx).wheel_translation
                };
                let finished = matches!(event.touch_phase, TouchPhase::Ended);
                if finished {
                    wheel_state.update(cx, |state, _| {
                        state.wheel_inertia_tail = is_pixels;
                        state.wheel_idle = None;
                    });
                }
                if finished || NotifierSwipeDismissPolicy::should_dismiss(translation, translation) {
                    Self::end_drag(&wheel_state, translation, &wheel_dismiss, window, cx);
                    return;
                }
                let generation = wheel_state.update(cx, |state, cx| {
                    state.wheel_translation = translation;
                    state.wheel_generation = state.wheel_generation.wrapping_add(1);
                    state.drag_offset.jump(translation.min(0.0));
                    cx.notify();
                    state.wheel_generation
                });
                let weak = wheel_state.downgrade();
                let idle_dismiss = wheel_dismiss.clone();
                let executor = cx.background_executor().clone();
                let idle = window.spawn(cx, async move |window_cx| {
                    executor.timer(WHEEL_IDLE_END).await;
                    let _ = window_cx.update(|window, cx| {
                        let Some(state) = weak.upgrade() else {
                            return;
                        };
                        let (current, translation) = {
                            let state = state.read(cx);
                            (state.wheel_generation, state.wheel_translation)
                        };
                        if current == generation && translation != 0.0 {
                            Self::end_drag(&state, translation, &idle_dismiss, window, cx);
                        }
                    });
                });
                wheel_state.update(cx, |state, _| state.wheel_idle = Some(idle));
                cx.stop_propagation();
            })
    }
}

#[derive(IntoElement)]
struct NotifierUndoRing {
    id: ElementId,
    deadline: Date,
    window: TimeInterval,
    ink: Hsla,
}

impl RenderOnce for NotifierUndoRing {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let reduce = reduce_motion(cx);
        let deadline = self.deadline;
        let progress_state = window.use_keyed_state(self.id.clone(), cx, move |_, cx| {
            let mut progress = Animated::new(1.0_f32);
            let remaining = deadline.time_interval_since_now().max(0.0);
            if remaining <= 0.05 {
                progress.jump(0.0);
            } else {
                let linear = MotionSpec::curve(CubicBezier::LINEAR, duration_from_seconds(remaining));
                progress.set(0.0, linear, RiseClock::monotonic(), reduce_motion(cx));
            }
            progress
        });

        let wall_now = Date::now();
        let seconds_left = NotifierUndoPolicy::seconds_remaining(&deadline, &wall_now);
        let progress = if reduce {
            NotifierUndoPolicy::ring_progress(&deadline, &wall_now, self.window) as f32
        } else {
            progress_state.read(cx).frame_value(now, window)
        };
        if reduce && seconds_left > 0 {
            window.request_animation_frame();
        }

        let ink = self.ink;
        div()
            .size(px(GLYPH_SIDE))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .child(
                canvas(|_, _, _| (), move |bounds, (), window, _| paint_ring(bounds, progress, ink, window))
                    .absolute()
                    .inset_0(),
            )
            .child(
                MainText::new(seconds_left.to_string())
                    .px(13.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .monospaced_digits(true)
                    .color(ink),
            )
    }
}

const RING_LINE_WIDTH: f32 = 2.0;
const RING_SEGMENTS: usize = 96;

fn paint_ring(bounds: Bounds<Pixels>, progress: f32, ink: Hsla, window: &mut Window) {
    let center = bounds.center();
    let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0 - RING_LINE_WIDTH / 2.0;
    let arc = |from: f32, to: f32| -> Option<gpui::Path<Pixels>> {
        let steps = ((RING_SEGMENTS as f32) * (to - from)).ceil().max(1.0) as usize;
        let mut builder = PathBuilder::stroke(px(RING_LINE_WIDTH));
        for step in 0..=steps {
            let fraction = from + (to - from) * step as f32 / steps as f32;
            let angle = fraction * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let at = point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()));
            if step == 0 {
                builder.move_to(at);
            } else {
                builder.line_to(at);
            }
        }
        builder.build().ok()
    };
    if let Some(track) = arc(0.0, 1.0) {
        window.paint_path(track, ink.opacity(0.24));
    }
    let progress = progress.clamp(0.0, 1.0);
    if progress > 0.0
        && let Some(remaining) = arc(1.0 - progress, 1.0)
    {
        window.paint_path(remaining, ink);
    }
}

#[derive(IntoElement)]
struct NotifierUndoButton {
    id: ElementId,
    title: String,
    color: Hsla,
    on_undo: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl RenderOnce for NotifierUndoButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_undo = self.on_undo.clone();
        plain(self.id).on_press(move |window, cx| on_undo(window, cx)).child(
            div().py(px(6.0)).pl(px(8.0)).child(
                MainText::new(self.title)
                    .px(16.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(self.color)
                    .number_of_lines(1),
            ),
        )
    }
}
