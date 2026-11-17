use gpui::{
    App, Context, Entity, FontWeight, IntoElement, ParentElement, Render, RenderOnce, Styled, Subscription, Window,
    div,
};
use rise_i18n::tr;
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::binding::Binding;

use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::purchase_celebration::{PurchaseCelebration, PurchaseCelebrationSheet};
use crate::core::widgets::overlay::subscription_success_store::SubscriptionSuccessStore;

const PREMIUM_PRODUCT_NAME: &str = "Riseonly Premium";

const MARK_FRAME_SHARE: f32 = 0.83;

pub struct SubscriptionSuccessHost {
    store: Entity<SubscriptionSuccessStore>,
    _store_observation: Subscription,
}

impl SubscriptionSuccessHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = SubscriptionSuccessStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            _store_observation: store_observation,
        }
    }
}

impl Render for SubscriptionSuccessHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let read = self.store.clone();
        let write = self.store.clone();
        let done = self.store.clone();
        SubscriptionSuccessSheet::new(
            Binding::new(
                move |cx| read.read(cx).is_visible(),
                move |is_presented, _, cx| {
                    if !is_presented {
                        write.update(cx, |store, cx| store.hide(cx));
                    }
                },
            ),
            move |_, cx| done.update(cx, |store, cx| store.hide(cx)),
        )
    }
}

/// after premium lands (verified purchase or redeemed promo)
/// sparks style celebration in premium purple with haptics, in a sheet as tall as its content
#[derive(IntoElement)]
pub struct SubscriptionSuccessSheet {
    is_presented: Binding<bool>,
    on_dismiss: std::rc::Rc<dyn Fn(&mut Window, &mut App)>,
}

impl SubscriptionSuccessSheet {
    pub fn new(is_presented: Binding<bool>, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            is_presented,
            on_dismiss: std::rc::Rc::new(on_dismiss),
        }
    }
}

impl RenderOnce for SubscriptionSuccessSheet {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_dismiss = self.on_dismiss;
        div().size_full().child(PurchaseCelebrationSheet::new(
            PurchaseCelebration::Premium,
            self.is_presented,
            tr("subscription_success_continue"),
            move |window, cx| on_dismiss(window, cx),
            |_, _| {
                LottieView::new(PremiumDiamond::ANIMATION)
                    .size(PurchaseCelebration::ICON_TIP_TO_TIP / MARK_FRAME_SHARE)
                    .looping(false)
                    .speed(0.0)
            },
            |_, _| SubscriptionSuccessLines,
        ))
    }
}

#[derive(IntoElement)]
struct SubscriptionSuccessLines;

impl RenderOnce for SubscriptionSuccessLines {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div().px_6().child(
                    MainText::new(tr("subscription_success_title"))
                        .px(22.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::BOLD),
                ),
            )
            .child(
                div().pt_2().child(
                    MainText::new(PREMIUM_PRODUCT_NAME)
                        .px(20.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD),
                ),
            )
            .child(
                div()
                    .pt_1p5()
                    .px_6()
                    .child(SecondaryText::new(tr("subscription_status_active")).px(14.0).tac(TextAlignT::Center)),
            )
    }
}
