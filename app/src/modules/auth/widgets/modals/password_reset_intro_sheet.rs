use std::rc::Rc;

use gpui::{App, FontWeight, SharedString, Window};
use rise_i18n::tr;
use rise_ui::button_ui::ButtonUi;
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::modules::auth::widgets::modals::auth_sheet_metrics::AuthSheetMetrics;

#[derive(IntoElement)]
pub struct PasswordResetIntroSheet {
    phone_number: SharedString,
    is_submitting: bool,
    on_continue: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl PasswordResetIntroSheet {
    pub const PRESENTATION_HEIGHT: f32 = 340.0;

    pub fn new(phone_number: impl Into<SharedString>) -> Self {
        Self {
            phone_number: phone_number.into(),
            is_submitting: false,
            on_continue: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn is_submitting(mut self, is_submitting: bool) -> Self {
        self.is_submitting = is_submitting;
        self
    }

    pub fn on_continue(mut self, on_continue: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_continue = Rc::new(on_continue);
        self
    }
}

impl RenderOnce for PasswordResetIntroSheet {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_continue = self.on_continue;

        vstack(0.0)
            .size_full()
            .px(px(AuthSheetMetrics::HORIZONTAL_PADDING))
            .pt(px(AuthSheetMetrics::TOP_PADDING))
            .pb(px(AuthSheetMetrics::BOTTOM_PADDING))
            .child(
                vstack(14.0)
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .justify_center()
                    .child(LottieView::new("help").size(104.0).speed(0.9))
                    .child(
                        MainText::new(tr("auth_reset_password_title"))
                            .px(19.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD)
                            .w_full(),
                    )
                    .child(
                        SecondaryText::new(tr("auth_reset_password_code_subtitle"))
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .line_height(19.0)
                            .w_full(),
                    )
                    .child(
                        MainText::new(self.phone_number)
                            .px(16.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD)
                            .w_full(),
                    ),
            )
            .child(
                ButtonUi::new("password-reset-intro-continue")
                    .is_pending(self.is_submitting)
                    .height(AuthSheetMetrics::BUTTON_HEIGHT)
                    .on_press(move |window, cx| on_continue(window, cx))
                    .child(MainText::new(tr("auth_reset_password_continue")).font_weight(FontWeight::SEMIBOLD)),
            )
    }
}
