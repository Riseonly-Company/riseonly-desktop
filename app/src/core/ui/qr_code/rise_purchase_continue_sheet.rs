use std::sync::Arc;

use gpui::{App, ElementId, FontWeight, Hsla, RenderImage, SharedString, Window};
use rise_i18n::tr;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::NativeBottomSheetPresentationHost;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::qr_code::rise_phone_handoff_code::RisePhoneHandoffCode;

pub struct RisePurchaseContinueMetrics;

impl RisePurchaseContinueMetrics {
    pub const CODE_SIDE: f32 = 200.0;
    pub const HEADER_HEIGHT: f32 = 52.0;
    pub const CLOSE_INSET: f32 = 12.0;
    pub const TITLE_INSET: f32 = 48.0;
    pub const TEXT_INSET: f32 = 24.0;
}

/// what the dialog says, the title and the hint under the code
/// an optional line naming what is bought like a plan, price and recipient
/// and the link the code carries
#[derive(Clone, IntoElement)]
pub struct RisePurchaseContinue {
    id: ElementId,
    title: SharedString,
    hint: SharedString,
    detail: Option<SharedString>,
    link: SharedString,
    code: Option<Arc<RenderImage>>,
}

impl RisePurchaseContinue {
    /// a purchase, continue purchase over the code of the public site link
    /// hint names what the phone app opens to
    pub fn purchase(id: impl Into<ElementId>, hint: impl Into<SharedString>) -> Self {
        Self::new(id, tr("purchase_continue_title"), hint)
    }

    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>, hint: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            hint: hint.into(),
            detail: None,
            code: None,
            link: SharedString::from(AppEnvironment::current().public_site_url().absolute_string().to_owned()),
        }
    }

    /// phone link the code carries like the sparks, gift or premium paths
    /// the phone opens the same screen and its own store payment
    /// without one the code keeps the public site
    pub fn link(mut self, link: Option<RiseonlyPublicLink>) -> Self {
        if let Some(url) = link.and_then(|link| link.web_url(AppEnvironment::current())) {
            self.link = SharedString::from(url.absolute_string().to_owned());
        }
        self
    }

    /// the link the code carries, for an owner that rasters the code before it shows the dialog
    pub fn code_link(&self) -> SharedString {
        self.link.clone()
    }

    /// a code rastered ahead with RisePhoneHandoffCode::raster, so the dialog opens with it in place
    pub fn code_image(mut self, image: Option<Arc<RenderImage>>) -> Self {
        self.code = image;
        self
    }

    pub fn detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail.filter(|detail| !detail.is_empty()).map(SharedString::from);
        self
    }

    /// modal over everything else, open while is_presented holds
    /// closing it sets the binding to false which is where the owner clears its phone handoff
    pub fn sheet(self, is_presented: Binding<bool>, background: impl Into<Hsla>) -> NativeBottomSheetPresentationHost {
        let id = self.id.clone();
        NativeBottomSheetPresentationHost::new((id, "purchase-continue-sheet"), is_presented, move |_, _| self.clone())
            .presentation_drag_indicator(false)
            .presentation_background(background)
            .compact(true)
    }

    fn header(&self, close: DismissAction) -> Div {
        div()
            .relative()
            .w_full()
            .h(px(RisePurchaseContinueMetrics::HEADER_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div().w_full().px(px(RisePurchaseContinueMetrics::TITLE_INSET)).child(
                    MainText::new(self.title.to_string())
                        .px(16.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .tac(TextAlignT::Center)
                        .line_limit(1)
                        .w_full(),
                ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(RisePurchaseContinueMetrics::CLOSE_INSET))
                    .flex()
                    .items_center()
                    .child(
                        PageHeaderIconButton::new((self.id.clone(), "close"), "xmark", move |window, cx| {
                            close.call_as_function(window, cx);
                        })
                        .font_size(15.0),
                    ),
            )
    }
}

impl RenderOnce for RisePurchaseContinue {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let close = dismiss(cx);
        let header = self.header(close);
        let text_inset = px(RisePurchaseContinueMetrics::TEXT_INSET);
        vstack(14.0)
            .w_full()
            .pb(px(24.0))
            .child(header)
            .child(RisePhoneHandoffCode::new(
                (self.id.clone(), "code"),
                self.link.clone(),
                RisePurchaseContinueMetrics::CODE_SIDE,
            )
            .prerendered(self.code.clone()))
            .children(self.detail.clone().map(|detail| {
                div().w_full().px(text_inset).child(
                    MainText::new(detail.to_string())
                        .px(15.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .monospaced_digits(true)
                        .tac(TextAlignT::Center)
                        .w_full(),
                )
            }))
            .child(
                div()
                    .w_full()
                    .px(text_inset)
                    .child(SecondaryText::new(self.hint.to_string()).px(13.0).tac(TextAlignT::Center).w_full()),
            )
    }
}
