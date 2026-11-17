use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, Context, Entity, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Point, RenderOnce, Styled, Subscription, Window, WindowId, black, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle, SelectionFeedbackGenerator};
use rise_ui::hold_context_menu_ui::hold_context_menu_action_list::{HoldContextMenuActionList, install_key_bindings};
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::modifiers::ViewModifier;
use rise_ui::swiftui::motion::{MotionSpec, Spring};
use rise_ui::swiftui::transition::{Presence, Transition, UnitPoint};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::rise_feature_gate::RiseFeatureGate;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;

const SESSION_TAP_WINDOW: f64 = 0.4;

/// menu with the same destinations as the more screen, opened from the more item of the bottom bar
/// secondary click opens it, hover and arrow keys highlight
pub enum MoreTabBarMenu {}

impl MoreTabBarMenu {
    pub const MENU_GAP: f32 = 16.0;

    pub const PROFILE_ACTION_ID: &'static str = "more_tab_menu_profile";
    pub const MUSIC_ACTION_ID: &'static str = "more_tab_menu_music";
    pub const VACANCIES_ACTION_ID: &'static str = "more_tab_menu_vacancies";
    pub const MAP_ACTION_ID: &'static str = "more_tab_menu_map";

    pub fn actions() -> Vec<HoldContextMenuAction> {
        let all = vec![
            HoldContextMenuAction::new(Self::PROFILE_ACTION_ID, tr("more_tab_profile"))
                .system_image("person.crop.circle"),
            HoldContextMenuAction::new(Self::MUSIC_ACTION_ID, tr("more_tab_music")).system_image("music.note"),
            HoldContextMenuAction::new(Self::VACANCIES_ACTION_ID, tr("more_tab_vacancies"))
                .system_image("briefcase.fill"),
            HoldContextMenuAction::new(Self::MAP_ACTION_ID, tr("more_tab_map")).system_image("map.fill"),
        ];
        all.into_iter()
            .filter(|action| RiseFeatureGate::VACANCIES || action.id != Self::VACANCIES_ACTION_ID)
            .collect()
    }

    pub fn route(action_id: &str) -> Option<RootRoute> {
        match action_id {
            Self::PROFILE_ACTION_ID => Some(RootRoute::MyProfile),
            Self::MUSIC_ACTION_ID => Some(RootRoute::MusicTab),
            Self::VACANCIES_ACTION_ID => RiseFeatureGate::VACANCIES.then_some(RootRoute::VacancyList),
            Self::MAP_ACTION_ID => Some(RootRoute::WorldMap),
            _ => None,
        }
    }
}

/// state of the one open menu
/// the anchor remembers its window so only that window overlay draws the panel
pub struct MoreTabBarMenuState {
    anchor: Option<Bounds<Pixels>>,
    anchor_window: Option<WindowId>,
    highlighted_action_id: Option<String>,
    pub action_frames: HashMap<String, Bounds<Pixels>>,
    selection: SelectionFeedbackGenerator,
    impact: ImpactFeedbackGenerator,
    closed_at: Option<Duration>,
    released_at: Option<Duration>,
}

impl MoreTabBarMenuState {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            anchor: None,
            anchor_window: None,
            highlighted_action_id: None,
            action_frames: HashMap::new(),
            selection: SelectionFeedbackGenerator::new(),
            impact: ImpactFeedbackGenerator::new(ImpactStyle::Medium),
            closed_at: None,
            released_at: None,
        }
    }

    pub fn anchor(&self) -> Option<Bounds<Pixels>> {
        self.anchor
    }

    pub fn highlighted_action_id(&self) -> Option<&str> {
        self.highlighted_action_id.as_deref()
    }

    pub fn is_open(&self) -> bool {
        self.anchor.is_some()
    }

    fn within_session_tap_window(since: Option<Duration>) -> bool {
        since.is_some_and(|at| RiseClock::monotonic().saturating_sub(at).as_secs_f64() < SESSION_TAP_WINDOW)
    }

    pub fn swallows_tap(&self) -> bool {
        self.is_open() || Self::within_session_tap_window(self.closed_at)
    }

    pub fn owns_tab_selection(&self) -> bool {
        self.is_open() || Self::within_session_tap_window(self.released_at)
    }

    pub fn open(&mut self, anchor: Bounds<Pixels>, window: WindowId, cx: &mut Context<Self>) {
        if self.anchor.is_some() {
            return;
        }
        self.action_frames.clear();
        self.highlighted_action_id = None;
        self.anchor = Some(anchor);
        self.anchor_window = Some(window);
        let _ = self.selection.prepare();
        let _ = self.impact.impact_occurred();
        cx.notify();
    }

    pub fn update_drag(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) {
        if self.anchor.is_none() {
            return;
        }
        let hit = self.hit_test(point);
        self.highlight(hit, cx);
    }

    /// hover and arrow keys name the row directly
    pub fn highlight(&mut self, action_id: Option<String>, cx: &mut Context<Self>) {
        if self.anchor.is_none() || action_id == self.highlighted_action_id {
            return;
        }
        let changed_to_row = action_id.is_some();
        self.highlighted_action_id = action_id;
        if changed_to_row {
            let _ = self.selection.selection_changed();
        }
        cx.notify();
    }

    /// returns the chosen item and closes the menu
    /// a press outside the panel is a cancel
    pub fn release(&mut self, point: Option<Point<Pixels>>, cx: &mut Context<Self>) -> Option<String> {
        self.anchor?;
        let chosen = point.and_then(|point| self.hit_test(point));
        self.released_at = Some(RiseClock::monotonic());
        self.close(cx);
        chosen
    }

    pub fn release_action(&mut self, action_id: String, cx: &mut Context<Self>) -> Option<String> {
        self.anchor?;
        self.released_at = Some(RiseClock::monotonic());
        self.close(cx);
        Some(action_id)
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.anchor.is_some() {
            self.closed_at = Some(RiseClock::monotonic());
        }
        self.anchor = None;
        self.anchor_window = None;
        self.highlighted_action_id = None;
        self.action_frames.clear();
        cx.notify();
    }

    fn anchor_in(&self, window: WindowId) -> Option<Bounds<Pixels>> {
        self.anchor.filter(|_| self.anchor_window == Some(window))
    }

    fn hit_test(&self, point: Point<Pixels>) -> Option<String> {
        self.action_frames
            .iter()
            .find(|(_, frame)| frame.contains(&point))
            .map(|(action_id, _)| action_id.clone())
    }
}

fn open_spring() -> MotionSpec {
    MotionSpec::spring(Spring::new(HoldContextMenuMetrics::OPEN_RESPONSE, HoldContextMenuMetrics::OPEN_DAMPING))
}

fn push_chosen(chosen: Option<String>, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
    let Some(route) = chosen.as_deref().and_then(MoreTabBarMenu::route) else {
        return;
    };
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

struct MoreTabBarMenuOverlayState {
    panel_presence: Presence<Bounds<Pixels>>,
    dim_presence: Presence<()>,
    bounds: Bounds<Pixels>,
    focus_handle: FocusHandle,
    previous_focus: Option<FocusHandle>,
    _menu_state_subscription: Subscription,
}

impl MoreTabBarMenuOverlayState {
    fn new(state: Entity<MoreTabBarMenuState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        install_key_bindings(cx);
        let window_id = window.window_handle().window_id();
        let subscription = cx.observe_in(&state, window, move |this, state, window, cx| {
            let anchor = state.read(cx).anchor_in(window_id);
            this.apply_anchor(anchor, window, cx);
            cx.notify();
        });
        cx.on_release(move |_, cx| state.update(cx, |state, cx| state.close(cx))).detach();
        Self {
            panel_presence: Presence::new(None).with_transition(
                Transition::scale_anchored(HoldContextMenuMetrics::INITIAL_PANEL_SCALE, UnitPoint::BOTTOM_TRAILING)
                    .combined(Transition::opacity()),
            ),
            dim_presence: Presence::new(None).with_transition(Transition::opacity()),
            bounds: Bounds::default(),
            focus_handle: cx.focus_handle(),
            previous_focus: None,
            _menu_state_subscription: subscription,
        }
    }

    fn apply_anchor(&mut self, anchor: Option<Bounds<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        let was_present = self.panel_presence.is_present();
        if anchor.is_some() && !was_present {
            self.previous_focus = window.focused(cx);
            window.focus(&self.focus_handle, cx);
        } else if anchor.is_none() && was_present {
            match self.previous_focus.take() {
                Some(previous) => window.focus(&previous, cx),
                None => window.blur(),
            }
        }
        let now = RiseClock::monotonic();
        self.panel_presence.set_in(anchor, open_spring(), now, cx);
        self.dim_presence.set_in(anchor.map(|_| ()), open_spring(), now, cx);
    }
}

/// the menu panel and its dim, takes part in hit testing
/// reads the navigation store from the environment
#[derive(IntoElement, Default)]
pub struct MoreTabBarMenuOverlay;

impl MoreTabBarMenuOverlay {
    pub fn new() -> Self {
        Self
    }

    fn panel(
        max_width: f32,
        highlighted_action_id: Option<String>,
        focus_handle: FocusHandle,
        state: &Entity<MoreTabBarMenuState>,
        nav_store: &Option<Entity<NavigationStore>>,
    ) -> HoldContextMenuActionList {
        let on_select = {
            let state = state.clone();
            let nav_store = nav_store.clone();
            Rc::new(move |action_id: String, _: &mut Window, cx: &mut App| {
                let chosen = state.update(cx, |state, cx| state.release_action(action_id, cx));
                push_chosen(chosen, &nav_store, cx);
            })
        };
        let on_highlight = {
            let state = state.clone();
            Rc::new(move |action_id: Option<String>, _: &mut Window, cx: &mut App| {
                state.update(cx, |state, cx| state.highlight(action_id, cx));
            })
        };
        let on_action_frames = {
            let state = state.clone();
            Rc::new(move |frames: HashMap<String, Bounds<Pixels>>, _: &mut Window, cx: &mut App| {
                state.update(cx, |state, _| state.action_frames = frames);
            })
        };
        let on_dismiss = {
            let state = state.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                state.update(cx, |state, cx| state.close(cx));
            })
        };
        HoldContextMenuActionList::new(
            "more-tab-bar-menu-actions",
            MoreTabBarMenu::actions(),
            highlighted_action_id,
            on_select,
        )
        .max_width(max_width)
        .on_highlight(on_highlight)
        .on_action_frames(on_action_frames)
        .on_dismiss(on_dismiss)
        .focus_handle(focus_handle)
    }
}

impl RenderOnce for MoreTabBarMenuOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = MoreTabBarMenuState::shared(cx);
        let overlay = {
            let state = state.clone();
            window.use_keyed_state("more-tab-bar-menu-overlay", cx, move |window, cx| {
                MoreTabBarMenuOverlayState::new(state, window, cx)
            })
        };
        let nav_store = env::<NavigationStoreKey>(cx);
        let window_id = window.window_handle().window_id();
        let now = RiseClock::monotonic();

        let menu = state.read(cx);
        let is_open = menu.anchor_in(window_id).is_some();
        let highlighted_action_id = menu.highlighted_action_id.clone();

        let overlay_state = overlay.read(cx);
        let bounds = overlay_state.bounds;
        let focus_handle = overlay_state.focus_handle.clone();
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);

        let dim = overlay_state.dim_presence.render_transitioned(now, window, |_| {
            let dim = div().absolute().inset_0().bg(black().opacity(0.22));
            if !is_open {
                return dim;
            }
            let press = {
                let state = state.clone();
                let nav_store = nav_store.clone();
                Rc::new(move |event: &MouseDownEvent, cx: &mut App| {
                    let chosen = state.update(cx, |state, cx| state.release(Some(event.position), cx));
                    push_chosen(chosen, &nav_store, cx);
                })
            };
            let secondary_press = press.clone();
            dim.occlude()
                .on_mouse_down(MouseButton::Left, move |event, _, cx| press(event, cx))
                .on_mouse_down(MouseButton::Right, move |event, _, cx| secondary_press(event, cx))
        });

        let panel = overlay_state.panel_presence.render_transitioned(now, window, |anchor| {
            let max_width =
                HoldContextMenuMetrics::MENU_MAX_WIDTH.min(width - HoldContextMenuMetrics::EDGE_INSET * 2.0);
            let trailing = HoldContextMenuMetrics::EDGE_INSET
                .max(width - f32::from(anchor.right() - bounds.origin.x));
            let bottom = HoldContextMenuMetrics::EDGE_INSET
                .max(height - f32::from(anchor.top() - bounds.origin.y) + MoreTabBarMenu::MENU_GAP);
            div()
                .absolute()
                .right(px(trailing))
                .bottom(px(bottom))
                .occlude()
                .child(MoreTabBarMenuOverlay::panel(max_width, highlighted_action_id, focus_handle, &state, &nav_store))
        });

        let measured = overlay.clone();
        div()
            .absolute()
            .inset_0()
            .child(measure("more-tab-bar-menu-overlay-bounds", move |bounds, _, cx| {
                measured.update(cx, |overlay, cx| {
                    overlay.bounds = bounds;
                    cx.notify();
                });
            }))
            .children(dim)
            .children(panel)
    }
}

struct ItemFrame {
    frame: Bounds<Pixels>,
}

/// same menu on the more button of the bottom bar
/// secondary click opens it, primary click still switches tabs
pub struct MoreTabBarMenuHoldModifier {
    pub is_enabled: bool,
}

impl MoreTabBarMenuHoldModifier {
    pub fn new(is_enabled: bool) -> Self {
        Self { is_enabled }
    }
}

impl ViewModifier for MoreTabBarMenuHoldModifier {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        if !self.is_enabled {
            return content;
        }
        let item = window.use_keyed_state("more-tab-bar-menu-hold-item", cx, |_, _| ItemFrame {
            frame: Bounds::default(),
        });
        let measured = item.clone();
        div()
            .relative()
            .flex_1()
            .flex()
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                let frame = item.read(cx).frame;
                let presenter = HoldContextMenuPresenter::shared(cx);
                if presenter.read(cx).active_request().is_some() || frame.size.width <= Pixels::ZERO {
                    return;
                }
                let window_id = window.window_handle().window_id();
                MoreTabBarMenuState::shared(cx).update(cx, |state, cx| state.open(frame, window_id, cx));
            })
            .child(content)
            .child(measure("more-tab-bar-menu-hold-item-frame", move |frame, _, cx| {
                measured.update(cx, |item, _| item.frame = frame);
            }))
            .into_any_element()
    }
}
