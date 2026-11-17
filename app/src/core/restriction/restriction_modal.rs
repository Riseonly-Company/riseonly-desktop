use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Context, ElementId, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, RenderOnce, Styled, Subscription, Window, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::button_ui::ButtonUi;
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec};
use rise_ui::swiftui::stacks::vstack;
use rise_ui::swiftui::transition::TransitionModifiers;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::core::restriction::restriction_store::{RestrictionPrompt, RestrictionStore};
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::ExperimentalBottomSheetPresentationHost;

const REVEAL_DURATION: Duration = Duration::from_millis(220);
const CLOSE_DURATION: Duration = Duration::from_millis(160);
const REVEAL_MOTION: MotionSpec = MotionSpec::curve(CubicBezier::EASE_OUT, REVEAL_DURATION);
const CLOSE_MOTION: MotionSpec = MotionSpec::curve(CubicBezier::EASE_IN, CLOSE_DURATION);
const HIDDEN_CARD_SCALE: f32 = 0.94;
const CARD_CORNER_RADIUS: f32 = 22.0;
const CARD_MAX_WIDTH: f32 = 460.0;

/// shows the restriction modal over whatever is on screen
/// draws nothing in the root stack, the modal goes up from the top of the presented
/// chain because a comment can be refused while the comments sheet is open
pub struct RestrictionHost {
    store: Entity<RestrictionStore>,
    _subscription: Subscription,
}

impl RestrictionHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = RestrictionStore::shared(cx);
        let subscription = cx.observe(&store, |_, _, cx| cx.notify());
        Self { store, _subscription: subscription }
    }
}

impl Render for RestrictionHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prompt = self.store.read(cx).prompt().cloned();
        let presented_prompt = prompt.clone();
        let is_presented = prompt.is_some();
        let dismiss_store = self.store.clone();
        let content_store = self.store.clone();

        let host = ExperimentalBottomSheetPresentationHost::new(
            "restriction-host",
            Binding::new(
                move |_| is_presented,
                move |is_presented, _, cx| {
                    if !is_presented {
                        dismiss_store.update(cx, |store, cx| store.dismiss(None, cx));
                    }
                },
            ),
            move |_, _| {
                let Some(prompt) = presented_prompt.clone() else {
                    return div().into_any_element();
                };
                let prompt_id = prompt.id;
                let store = content_store.clone();
                RestrictionModal {
                    prompt,
                    on_dismiss: Rc::new(move |_: &mut Window, cx: &mut App| store.update(cx, |store, cx| store.dismiss(Some(prompt_id), cx))),
                }
                .into_any_element()
            },
        )
        .dismissal_delay(0.0);

        div().size_0().child(host)
    }
}

#[derive(IntoElement)]
pub struct RestrictionModal {
    pub prompt: RestrictionPrompt,
    pub on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
}

struct RestrictionModalState {
    is_shown: bool,
    shown: Animated<f32>,
    revealed_prompt: Option<Uuid>,
    focus_handle: FocusHandle,
}

impl RestrictionModal {
    fn reveal(state: &Entity<RestrictionModalState>, prompt_id: Uuid, window: &mut Window, cx: &mut App) {
        let now = RiseClock::monotonic();
        let reduce = reduce_motion(cx);
        let focus_handle = state.update(cx, |state, cx| {
            state.revealed_prompt = Some(prompt_id);
            state.is_shown = true;
            state.shown.set(1.0, REVEAL_MOTION, now, reduce);
            cx.notify();
            state.focus_handle.clone()
        });
        window.defer(cx, move |window, cx| window.focus(&focus_handle, cx));
    }

    fn close(
        state: &Entity<RestrictionModalState>,
        on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !state.read(cx).is_shown {
            return;
        }
        let now = RiseClock::monotonic();
        let reduce = reduce_motion(cx);
        state.update(cx, |state, cx| {
            state.is_shown = false;
            state.shown.set(0.0, CLOSE_MOTION, now, reduce);
            cx.notify();
        });
        let delay = if reduce { Duration::ZERO } else { CLOSE_DURATION };
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(delay).await;
                let _ = cx.update(|window, cx| on_dismiss(window, cx));
            })
            .detach();
    }

    fn card(&self, theme: &AppTheme, state: &Entity<RestrictionModalState>) -> gpui::Div {
        let close_state = state.clone();
        let on_dismiss = self.on_dismiss.clone();
        vstack(0.0)
            .w_full()
            .max_w(px(CARD_MAX_WIDTH))
            .pt(px(22.0))
            .pb(px(16.0))
            .bg(theme.bg._200)
            .rounded(px(CARD_CORNER_RADIUS))
            .border_1()
            .border_color(theme.bg._400)
            .overflow_hidden()
            .occlude()
            .child(
                div()
                    .h(px(132.0))
                    .flex()
                    .justify_center()
                    .child(LottieView::new(self.prompt.animation.clone()).size(132.0).looping(true)),
            )
            .child(
                vstack(8.0)
                    .w_full()
                    .px(px(18.0))
                    .pt(px(12.0))
                    .child(
                        MainText::new(self.prompt.title.clone())
                            .px(22.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD),
                    )
                    .child(SecondaryText::new(self.prompt.subtitle.clone()).px(15.0).tac(TextAlignT::Center)),
            )
            .child(
                div().w_full().px(px(18.0)).pt(px(18.0)).child(
                    ButtonUi::new("restriction-modal-ok")
                        .background_color(theme.primary._100)
                        .height(50.0)
                        .on_press(move |window, cx| Self::close(&close_state, on_dismiss.clone(), window, cx))
                        .child(
                            MainText::new(tr("restriction_modal_ok"))
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.content_on(theme.primary._100)),
                        ),
                ),
            )
    }
}

impl RenderOnce for RestrictionModal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let state = window.use_keyed_state(ElementId::Name("restriction-modal".into()), cx, |_, cx| {
            RestrictionModalState {
                is_shown: false,
                shown: Animated::new(0.0),
                revealed_prompt: None,
                focus_handle: cx.focus_handle(),
            }
        });
        if state.read(cx).revealed_prompt != Some(self.prompt.id) {
            Self::reveal(&state, self.prompt.id, window, cx);
        }

        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let shown = state.read(cx).shown.frame_value(now, window);
        let focus_handle = state.read(cx).focus_handle.clone();
        let scale = HIDDEN_CARD_SCALE + (1.0 - HIDDEN_CARD_SCALE) * shown;

        let scrim_state = state.clone();
        let scrim_dismiss = self.on_dismiss.clone();
        let key_state = state.clone();
        let key_dismiss = self.on_dismiss.clone();

        div()
            .id("restriction-modal")
            .track_focus(&focus_handle)
            .absolute()
            .inset_0()
            .occlude()
            .opacity(shown)
            .layer_keys(LayerCancel::dismiss(move |window, cx| {
                Self::close(&key_state, key_dismiss.clone(), window, cx);
            }))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(theme.bg._100.opacity(0.82))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        Self::close(&scrim_state, scrim_dismiss.clone(), window, cx);
                    }),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(20.0))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(CARD_MAX_WIDTH))
                            .flex()
                            .justify_center()
                            .child(self.card(&theme, &state).scale_effect(scale)),
                    ),
            )
    }
}
