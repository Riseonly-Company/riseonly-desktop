use std::rc::Rc;

use gpui::{
    App, Context, Entity, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Render, RenderOnce,
    ScrollWheelEvent, Subscription, Window, div, prelude::*, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::button_ui::ButtonUi;
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::maintenance::maintenance_gate::{MaintenanceGate, MaintenanceSnapshot};
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};

const ENTER_KEY: &str = "enter";

const PRESENCE_DURATION: f64 = 0.22;
const SCRIM_OPACITY: f32 = 0.82;
const CARD_MAX_WIDTH: f32 = 460.0;
const CARD_CORNER_RADIUS: f32 = 22.0;
const ARTWORK_SIDE: f32 = 148.0;
const ARTWORK_CORNER_RADIUS: f32 = 16.0;

/// action for a modal press, button or Esc or Enter
type ModalAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// modal shown while the product is closed
/// keyed to the window, comes back on the next cold launch until the product opens
pub struct MaintenanceHost {
    gate: Entity<MaintenanceGate>,
    dismissed_revision: Option<i64>,
    presence: Presence<MaintenanceSnapshot>,
    focus_handle: FocusHandle,
    focus_before: Option<FocusHandle>,
    _gate_observation: Subscription,
    _theme_observation: Subscription,
}

impl MaintenanceHost {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let gate = MaintenanceGate::shared(cx);
        let gate_observation = cx.observe_in(&gate, window, |host, gate, window, cx| {
            if !gate.read(cx).snapshot().is_closed {
                host.dismissed_revision = None;
            }
            host.sync_focus(window, cx);
            cx.notify();
        });
        let theme_store = ThemeStore::shared(cx);
        let theme_observation = cx.observe(&theme_store, |_, _, cx| cx.notify());
        let shown = {
            let snapshot = gate.read(cx).snapshot();
            snapshot.is_closed.then(|| snapshot.clone())
        };
        let mut host = Self {
            gate,
            dismissed_revision: None,
            presence: Presence::new(shown).with_transition(Transition::opacity()),
            focus_handle: cx.focus_handle(),
            focus_before: None,
            _gate_observation: gate_observation,
            _theme_observation: theme_observation,
        };
        host.sync_focus(window, cx);
        host
    }

    fn is_presented(&self, cx: &App) -> bool {
        let snapshot = self.gate.read(cx).snapshot();
        snapshot.is_closed && self.dismissed_revision != Some(snapshot.revision)
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismissed_revision = Some(self.gate.read(cx).snapshot().revision);
        self.sync_focus(window, cx);
        cx.notify();
    }

    fn sync_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_presented(cx) {
            if !self.focus_handle.contains_focused(window, cx) {
                self.focus_before = window.focused(cx);
                window.focus(&self.focus_handle, cx);
            }
        } else if let Some(previous) = self.focus_before.take() {
            if self.focus_handle.contains_focused(window, cx) {
                window.focus(&previous, cx);
            }
        }
    }
}

impl Render for MaintenanceHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_presented = self.is_presented(cx);
        let shown = is_presented.then(|| self.gate.read(cx).snapshot().clone());
        let now = RiseClock::monotonic();
        self.presence.set_in(shown, ease_out(PRESENCE_DURATION), now, cx);

        let host = cx.entity().downgrade();
        let on_dismiss: ModalAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            host.update(cx, |host, cx| host.dismiss(window, cx)).ok();
        });
        let gate = self.gate.clone();
        let on_retry: ModalAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            gate.update(cx, |gate, cx| gate.refresh_now(cx));
        });
        let focus_handle = is_presented.then(|| self.focus_handle.clone());

        let modal = self.presence.render_transitioned(now, window, |snapshot| {
            div().absolute().inset_0().child(
                MaintenanceModal::new(snapshot.clone(), on_dismiss, on_retry)
                    .is_interactive(is_presented)
                    .focus_handle(focus_handle),
            )
        });

        div().size_full().relative().children(modal)
    }
}

#[derive(IntoElement)]
pub struct MaintenanceModal {
    snapshot: MaintenanceSnapshot,
    on_dismiss: ModalAction,
    on_retry: ModalAction,
    is_interactive: bool,
    focus_handle: Option<FocusHandle>,
}

impl MaintenanceModal {
    pub fn new(snapshot: MaintenanceSnapshot, on_dismiss: ModalAction, on_retry: ModalAction) -> Self {
        Self {
            snapshot,
            on_dismiss,
            on_retry,
            is_interactive: true,
            focus_handle: None,
        }
    }

    pub fn is_interactive(mut self, is_interactive: bool) -> Self {
        self.is_interactive = is_interactive;
        self
    }

    /// focus handle that gets Esc and Enter, the host focuses it while the modal is up
    pub fn focus_handle(mut self, focus_handle: Option<FocusHandle>) -> Self {
        self.focus_handle = focus_handle;
        self
    }

    /// bundled animation by default, the network is the one thing surely down while closed
    /// a remote reference is still honoured
    fn artwork(snapshot: &MaintenanceSnapshot, theme: &AppTheme) -> impl IntoElement {
        let artwork = if let Some(name) = snapshot.builtin_animation_name() {
            LottieView::new(name).size(ARTWORK_SIDE).looping(true).into_any_element()
        } else if snapshot.media_kind.is_remote() && !snapshot.media_ref.is_empty() {
            div()
                .size(px(ARTWORK_SIDE))
                .child(
                    CleverImageUi::new(snapshot.media_ref.clone())
                        .content_mode(ContentMode::Fit)
                        .corner_radius(ARTWORK_CORNER_RADIUS)
                        .persist_to_disk(true),
                )
                .into_any_element()
        } else {
            div()
                .size(px(ARTWORK_SIDE))
                .continuous_corners(ARTWORK_CORNER_RADIUS)
                .bg(theme.bg._300)
                .into_any_element()
        };
        div()
            .h(px(ARTWORK_SIDE))
            .flex()
            .items_center()
            .justify_center()
            .child(artwork)
    }
}

impl RenderOnce for MaintenanceModal {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let snapshot = self.snapshot;

        let on_retry = self.on_retry;
        let on_dismiss = self.on_dismiss;
        let press: ModalAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            on_retry(window, cx);
            on_dismiss(window, cx);
        });

        let copy = vstack(8.)
            .child(
                MainText::new(snapshot.title.clone())
                    .px(22.)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD),
            )
            .child(SecondaryText::new(snapshot.body.clone()).px(14.).tac(TextAlignT::Center))
            .child(
                MainText::new(snapshot.reopening_text())
                    .px(15.)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .margin_top(2.),
            )
            .child(
                SecondaryText::new(tr("maintenance_offline_hint"))
                    .px(13.)
                    .tac(TextAlignT::Center)
                    .mt(4.),
            )
            .px(px(18.))
            .pt(px(14.));

        let button_press = press.clone();
        let button = div().w_full().px(px(18.)).pt(px(18.)).child(
            ButtonUi::new("maintenance-modal-dismiss")
                .background_color(theme.primary._100)
                .height(50.)
                .on_press(move |window, cx| button_press(window, cx))
                .child(
                    MainText::new(tr("maintenance_modal_dismiss"))
                        .px(16.)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.content_on(theme.primary._100)),
                ),
        );

        let content = vstack(0.)
            .w_full()
            .pt(px(22.))
            .pb(px(16.))
            .bg(theme.bg._200)
            .clip_shape(Shape::continuous(CARD_CORNER_RADIUS))
            .child(Self::artwork(&snapshot, &theme))
            .child(copy)
            .child(button);

        let card = div()
            .id("maintenance-modal-card")
            .relative()
            .w_full()
            .child(content)
            .child(centred_stroke(
                theme.bg._400,
                1.,
                Shape::continuous(CARD_CORNER_RADIUS),
            ))
            .when(self.is_interactive, |card| card.occlude());

        let card_frame = div()
            .w_full()
            .max_w(px(CARD_MAX_WIDTH))
            .px(px(20.))
            .child(card);

        let scrim = div()
            .id("maintenance-modal-scrim")
            .absolute()
            .inset_0()
            .bg(theme.bg._100.opacity(SCRIM_OPACITY))
            .when(self.is_interactive, |scrim| {
                scrim
                    .occlude()
                    .on_scroll_wheel(|_: &ScrollWheelEvent, _, cx| cx.stop_propagation())
            });

        let key_press = press;
        div()
            .id("maintenance-modal")
            .size_full()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .when_some(self.focus_handle, |modal, focus_handle| {
                let escape_press = key_press.clone();
                modal
                    .track_focus(&focus_handle)
                    .layer_keys(LayerCancel::dismiss(move |window, cx| escape_press(window, cx)))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        let confirms = event.keystroke.key == ENTER_KEY
                            && !event.keystroke.modifiers.modified()
                            && focus_handle.is_focused(window);
                        if confirms {
                            key_press(window, cx);
                        }
                        cx.stop_propagation();
                    })
            })
            .child(scrim)
            .child(card_frame)
    }
}
