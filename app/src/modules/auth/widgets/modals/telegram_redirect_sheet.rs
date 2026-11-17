use std::rc::Rc;

use gpui::{App, FontWeight, Hsla, SharedString, Window, rgb, white};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::open_url::open_url;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::modules::auth::widgets::modals::auth_sheet_metrics::AuthSheetMetrics;

/// why the bot is opened, same conversation so only the title changes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TelegramRedirectSheetPurpose {
    #[default]
    Registration,
    PasswordReset,
}

impl TelegramRedirectSheetPurpose {
    pub const fn title_key(self) -> &'static str {
        match self {
            Self::Registration => "tg_redirect_title",
            Self::PasswordReset => "tg_redirect_reset_title",
        }
    }
}

#[derive(IntoElement)]
pub struct TelegramRedirectSheet {
    bot_username: SharedString,
    phone_payload: SharedString,
    purpose: TelegramRedirectSheetPurpose,
    on_enter_code: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl TelegramRedirectSheet {
    pub const PRESENTATION_HEIGHT: f32 = 400.0;

    pub fn new(bot_username: impl Into<SharedString>, phone_payload: impl Into<SharedString>) -> Self {
        Self {
            bot_username: bot_username.into(),
            phone_payload: phone_payload.into(),
            purpose: TelegramRedirectSheetPurpose::default(),
            on_enter_code: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn purpose(mut self, purpose: TelegramRedirectSheetPurpose) -> Self {
        self.purpose = purpose;
        self
    }

    pub fn on_enter_code(mut self, on_enter_code: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_enter_code = Rc::new(on_enter_code);
        self
    }

    pub fn telegram_blue() -> Hsla {
        rgb(0x229ED9).into()
    }

    fn tg_url(&self) -> Option<Url> {
        Url::from_string(&format!("tg://resolve?domain={}&start={}", self.bot_username, self.phone_payload))
    }

    fn web_url(&self) -> Option<Url> {
        Url::from_string(&format!("https://t.me/{}?start={}", self.bot_username, self.phone_payload))
    }

    fn open_telegram(tg_url: Option<&Url>, web_url: Option<&Url>, cx: &mut App) {
        if tg_url.is_some_and(|url| open_url(url, cx)) {
            return;
        }
        if let Some(url) = web_url {
            open_url(url, cx);
        }
    }
}

impl RenderOnce for TelegramRedirectSheet {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let tg_url = self.tg_url();
        let web_url = self.web_url();
        let on_enter_code = self.on_enter_code;

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
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(72.0))
                            .rounded_full()
                            .bg(Self::telegram_blue())
                            .children(IconUi::sized("paperplane.fill", px(30.0), white(), true)),
                    )
                    .child(
                        MainText::new(tr(self.purpose.title_key()))
                            .px(19.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD)
                            .w_full(),
                    )
                    .child(
                        SecondaryText::new(tr("tg_redirect_subtitle"))
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .line_height(19.0)
                            .w_full(),
                    ),
            )
            .child(
                vstack(AuthSheetMetrics::BUTTON_SPACING)
                    .w_full()
                    .flex_none()
                    .child(
                        ButtonUi::new("telegram-redirect-open")
                            .height(AuthSheetMetrics::BUTTON_HEIGHT)
                            .gap(8.0)
                            .on_press(move |_, cx| Self::open_telegram(tg_url.as_ref(), web_url.as_ref(), cx))
                            .children(IconUi::sized(
                                "arrow.up.right.square.fill",
                                px(17.0),
                                theme.content_on(theme.primary._100),
                                false,
                            ))
                            .child(MainText::new(tr("tg_redirect_open_btn")).font_weight(FontWeight::SEMIBOLD)),
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .child(
                                ButtonUi::new("telegram-redirect-got-code")
                                    .background_color(theme.bg._200)
                                    .height(AuthSheetMetrics::BUTTON_HEIGHT)
                                    .on_press(move |window, cx| on_enter_code(window, cx))
                                    .child(
                                        MainText::new(tr("tg_redirect_got_code_btn"))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(theme.text.primary),
                                    ),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(theme.border._100),
                            ),
                    ),
            )
    }
}
