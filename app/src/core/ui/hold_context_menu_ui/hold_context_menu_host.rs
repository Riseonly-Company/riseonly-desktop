use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Render, StyleRefinement, Styled, Subscription, Task, Window, div,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_theme::{RiseThemeAppearance, ThemeStore};

use crate::core::ui::hold_context_menu_ui::hold_context_menu_overlay::HoldContextMenuOverlay;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::{
    HoldContextMenuPresenter, HoldContextMenuRequestID,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_touch_tracker::HoldContextMenuTouchTracker;

/// menu layer of one window
pub struct HoldContextMenuHost {
    window_handle: AnyWindowHandle,
    overlay: Option<(HoldContextMenuRequestID, Entity<HoldContextMenuOverlay>)>,
    _subscriptions: Vec<Subscription>,
}

impl HoldContextMenuHost {
    pub const LAYER_PRIORITY: usize = 2010;

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let window_handle = window.window_handle();
        let presenter = HoldContextMenuPresenter::shared(cx);
        let window_presenter = HoldContextMenuWindowPresenter::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&presenter, |_, _, cx| cx.notify()),
            cx.observe(&window_presenter, |_, _, cx| cx.notify()),
            cx.observe(&theme_store, |_, _, cx| HoldContextMenuWindowPresenter::sync_appearance(cx)),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    return;
                }
                let handle = this.window_handle;
                cx.defer(move |cx| HoldContextMenuWindowPresenter::scene_will_deactivate(handle, cx));
            }),
            cx.observe_window_bounds(window, |this, _, cx| {
                let handle = this.window_handle;
                cx.defer(move |cx| HoldContextMenuWindowPresenter::scene_will_deactivate(handle, cx));
            }),
            cx.on_release(move |_, cx| HoldContextMenuWindowPresenter::scene_did_disconnect(window_handle, cx)),
        ];
        Self { window_handle, overlay: None, _subscriptions: subscriptions }
    }
}

impl Render for HoldContextMenuHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let presenter = HoldContextMenuPresenter::shared(cx);
        let (request, swallows_while_closing, touch_block_deadline) = {
            let presenter = presenter.read(cx);
            (
                presenter.active_request().cloned(),
                presenter.swallows_touches_while_closing(),
                presenter.touch_block_deadline(),
            )
        };
        let window_presenter = HoldContextMenuWindowPresenter::shared(cx);
        let (shown_here, interaction_enabled, menu_focus) = {
            let window_presenter = window_presenter.read(cx);
            (
                window_presenter.window == Some(self.window_handle),
                window_presenter.is_user_interaction_enabled,
                window_presenter.menu_focus.clone(),
            )
        };

        let request = request.filter(|_| shown_here);
        match &request {
            Some(request) if self.overlay.as_ref().is_some_and(|(id, _)| *id == request.id) => {}
            Some(request) => {
                let id = request.id;
                let request = request.clone();
                let on_dismiss: Rc<dyn Fn(&mut Window, &mut App)> =
                    Rc::new(move |_, cx| HoldContextMenuPresenter::dismiss_if_active(id, cx));
                tracing::warn!(target: "riseonly::X1351", id = ?id, "menu host mounts overlay");
                let overlay = cx.new(|cx| HoldContextMenuOverlay::new(request, on_dismiss, window, cx));
                self.overlay = Some((id, overlay));
            }
            None => self.overlay = None,
        }
        if shown_here {
            let mounted = self.overlay.as_ref().map(|(id, _)| *id);
            if window_presenter.read(cx).mounted_request != mounted {
                window_presenter.update(cx, |window_presenter, _| window_presenter.mounted_request = mounted);
            }
        }

        let blocks_closed_menu =
            request.is_none() && shown_here && interaction_enabled && RiseClock::monotonic() < touch_block_deadline;
        let blocks_touches = blocks_closed_menu || (request.is_some() && swallows_while_closing);

        let mut layer = div().id("hold-context-menu-host").absolute().inset_0().size_full().track_focus(&menu_focus);
        if let Some((_, overlay)) = &self.overlay {
            layer = layer.child(overlay.clone().cached(StyleRefinement::default().size_full()));
        }
        if blocks_touches {
            layer = layer.child(
                div()
                    .id("hold-context-menu-touch-block")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| {
                        tracing::warn!(
                            target: "riseonly::X1161",
                            closed = blocks_closed_menu,
                            "menu touch block took a press"
                        );
                    }),
            );
        }
        layer.child(HoldContextMenuTouchTracker::observer(self.window_handle))
    }
}

/// which window shows the menu layer and whether that layer takes clicks
/// commands are associated functions on app so a dismiss started here can call back in
pub struct HoldContextMenuWindowPresenter {
    window: Option<AnyWindowHandle>,
    source_window: Option<AnyWindowHandle>,
    is_user_interaction_enabled: bool,
    override_user_interface_style: Option<RiseThemeAppearance>,
    menu_focus: FocusHandle,
    source_focus: Option<FocusHandle>,
    touch_block_generation: u64,
    touch_block_task: Option<Task<()>>,
    mounted_request: Option<HoldContextMenuRequestID>,
    _memory_warning_observer: RiseSubscription,
    _memory_warning_task: Task<()>,
}

impl HoldContextMenuWindowPresenter {
    fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let observer = RiseAppEvents::default_center()
            .observe(UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(());
            });
        let task = cx.spawn(async move |_, cx| {
            while inbox.next().await.is_some() {
                cx.update(Self::release_for_memory_pressure);
            }
        });
        Self {
            window: None,
            source_window: None,
            is_user_interaction_enabled: false,
            override_user_interface_style: None,
            menu_focus: cx.focus_handle(),
            source_focus: None,
            touch_block_generation: 0,
            touch_block_task: None,
            mounted_request: None,
            _memory_warning_observer: observer,
            _memory_warning_task: task,
        }
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    /// focus of the menu layer, holding it means the menu window is key
    /// the overlay focuses it for arrow keys enter and esc
    pub fn menu_focus(cx: &mut App) -> FocusHandle {
        Self::shared(cx).read(cx).menu_focus.clone()
    }

    pub fn has_mounted_overlay(id: HoldContextMenuRequestID, cx: &mut App) -> bool {
        Self::shared(cx).read(cx).mounted_request == Some(id)
    }

    pub fn override_user_interface_style(&self) -> Option<RiseThemeAppearance> {
        self.override_user_interface_style
    }

    /// shows the menu layer from the source window
    pub fn show(source_window: Option<AnyWindowHandle>, cx: &mut App) {
        let Some(source_window) = source_window else { return };
        let style = Self::resolved_interface_style(cx);
        let this = Self::shared(cx);
        let reused = this.update(cx, |this, cx| {
            if this.window != Some(source_window) {
                return false;
            }
            this.source_window = Some(source_window);
            this.override_user_interface_style = Some(style);
            this.is_user_interaction_enabled = true;
            cx.notify();
            true
        });
        if reused {
            return;
        }

        Self::release_window(cx);
        this.update(cx, |this, cx| {
            this.source_window = Some(source_window);
            this.override_user_interface_style = Some(style);
            this.is_user_interaction_enabled = true;
            this.window = Some(source_window);
            cx.notify();
        });
        HoldContextMenuTouchTracker::install_in_menu_window(source_window);
    }

    /// theme style, or dark for a menu that asks for dark chrome like the media viewer
    fn resolved_interface_style(cx: &mut App) -> RiseThemeAppearance {
        let prefers_dark_chrome = HoldContextMenuPresenter::shared(cx)
            .read(cx)
            .active_request()
            .is_some_and(|request| request.prefers_dark_chrome);
        if prefers_dark_chrome {
            RiseThemeAppearance::Dark
        } else {
            ThemeStore::shared(cx).read(cx).window_interface_style()
        }
    }

    /// window the menu layer was shown from if any
    pub fn source_window(&self) -> Option<AnyWindowHandle> {
        self.source_window
    }

    pub fn sync_appearance(cx: &mut App) {
        let style = Self::resolved_interface_style(cx);
        Self::shared(cx).update(cx, |this, cx| {
            if this.window.is_some() && this.override_user_interface_style != Some(style) {
                this.override_user_interface_style = Some(style);
                cx.notify();
            }
        });
    }

    pub fn remember_source_focus(window: &mut Window, cx: &mut App) {
        let focused = window.focused(cx);
        let this = Self::shared(cx);
        let menu_focus = this.read(cx).menu_focus.clone();
        if menu_focus.contains_focused(window, cx) {
            return;
        }
        this.update(cx, |this, _| this.source_focus = focused);
    }

    /// a fresh click can focus the menu layer but a held press stays in the source
    /// hand keyboard focus back before an action focuses or presents source ui
    pub fn restore_source_key_window(window: &mut Window, cx: &mut App) {
        let (menu_focus, is_source) = {
            let this = Self::shared(cx);
            let this = this.read(cx);
            (
                this.menu_focus.clone(),
                this.window.is_some() && this.source_window == Some(window.window_handle()),
            )
        };
        if is_source && menu_focus.contains_focused(window, cx) {
            let source_focus = Self::shared(cx).update(cx, |this, _| this.source_focus.take());
            match source_focus {
                Some(handle) => window.focus(&handle, cx),
                None => window.blur(),
            }
        }
    }

    /// same hand back from a call that holds no window
    /// runs once the current update has released the window
    fn restore_source_key_window_deferred(cx: &mut App) {
        let Some(source_window) = Self::shared(cx).read(cx).source_window else { return };
        cx.defer(move |cx| {
            let _ = source_window.update(cx, |_, window, cx| Self::restore_source_key_window(window, cx));
        });
    }

    /// hides the layer, the deadline is on the RiseClock monotonic scale and zero blocks nothing
    /// layer stays mounted, the empty host swallows presses until the deadline and then passes them through
    pub fn hide(blocking_touches_until: Duration, cx: &mut App) {
        Self::restore_source_key_window_deferred(cx);
        Self::shared(cx).update(cx, |this, cx| {
            this.touch_block_generation = this.touch_block_generation.wrapping_add(1);
            let remaining = blocking_touches_until.saturating_sub(RiseClock::monotonic());
            if remaining.is_zero() {
                this.is_user_interaction_enabled = false;
                this.touch_block_task = None;
                cx.notify();
                return;
            }
            let generation = this.touch_block_generation;
            this.touch_block_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(remaining).await;
                let menu_is_open = cx.update(HoldContextMenuPresenter::has_active_request);
                if menu_is_open {
                    return;
                }
                let _ = this.update(cx, |this, cx| {
                    if this.touch_block_generation != generation {
                        return;
                    }
                    this.is_user_interaction_enabled = false;
                    cx.notify();
                });
            }));
            cx.notify();
        });
    }

    fn scene_will_deactivate(scene: AnyWindowHandle, cx: &mut App) {
        if Self::shared(cx).read(cx).window != Some(scene) {
            return;
        }
        tracing::warn!(target: "riseonly::X2254", active = HoldContextMenuPresenter::has_active_request(cx), "menu closes on window deactivate or resize");
        if HoldContextMenuPresenter::has_active_request(cx) {
            HoldContextMenuPresenter::dismiss(cx);
        } else {
            Self::hide(Duration::ZERO, cx);
        }
    }

    fn scene_did_disconnect(scene: AnyWindowHandle, cx: &mut App) {
        if Self::shared(cx).read(cx).window != Some(scene) {
            return;
        }
        if HoldContextMenuPresenter::has_active_request(cx) {
            HoldContextMenuPresenter::dismiss(cx);
        }
        Self::release_window(cx);
    }

    fn release_for_memory_pressure(cx: &mut App) {
        if HoldContextMenuPresenter::has_active_request(cx) {
            HoldContextMenuPresenter::dismiss(cx);
        }
        Self::release_window(cx);
    }

    fn release_window(cx: &mut App) {
        Self::restore_source_key_window_deferred(cx);
        Self::shared(cx).update(cx, |this, cx| {
            this.is_user_interaction_enabled = false;
            this.touch_block_task = None;
            this.window = None;
            this.source_window = None;
            cx.notify();
        });
    }
}

pub trait HoldContextMenuHostModifiers: IntoElement + Sized {
    fn mounts_hold_context_menu_host(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let host = window.use_keyed_state("hold-context-menu-host", cx, HoldContextMenuHost::new);
        div().relative().size_full().child(self).child(host).into_any_element()
    }
}

impl<E: IntoElement> HoldContextMenuHostModifiers for E {}
