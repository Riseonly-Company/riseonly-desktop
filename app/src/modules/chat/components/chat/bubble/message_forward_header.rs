use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, ElementId, FontWeight, HighlightStyle, Hsla, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, StyledText, Window, div, px,
};
use rise_i18n::tr;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::swiftui::stacks::{hstack, spacer_min_w};

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::stores::message::message_actions::message_types::MessageForwardInfo;

const MAX_HEADER_WIDTH: f32 = 280.0;
const LOGO_SIZE: f32 = 16.0;
const LOGO_SPACING: f32 = 6.0;

pub type MessageForwardHeaderOpenSource = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct MessageForwardHeader {
    id: ElementId,
    info: MessageForwardInfo,
    is_own: bool,
    on_open_source: Option<MessageForwardHeaderOpenSource>,
    max_width: Option<f32>,
}

impl MessageForwardHeader {
    pub fn new(id: impl Into<ElementId>, info: MessageForwardInfo) -> Self {
        Self { id: id.into(), info, is_own: false, on_open_source: None, max_width: None }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn on_open_source(mut self, on_open_source: impl Into<Option<MessageForwardHeaderOpenSource>>) -> Self {
        self.on_open_source = on_open_source.into();
        self
    }

    /// bubble content width. gpui wraps text at its last measure so the cap has to reach the text in points
    /// else a wide probe lays the line past a narrow video edge
    pub fn max_width(mut self, max_width: Option<f32>) -> Self {
        self.max_width = max_width.filter(|width| *width > 0.0);
        self
    }

    fn header(&self, title: &str, logo: Option<&str>, accent: Hsla) -> gpui::Div {
        let prefix = format!("{} ", tr("chat_msg_forwarded_from"));
        let title_range = prefix.len()..prefix.len() + title.len();
        let text = StyledText::new(SharedString::from(format!("{prefix}{title}"))).with_highlights([(
            title_range,
            HighlightStyle { font_weight: Some(FontWeight::MEDIUM), ..Default::default() },
        )]);
        let logo = logo.filter(|logo| !logo.is_empty());
        let row_width = self.max_width.map_or(MAX_HEADER_WIDTH, |width| width.min(MAX_HEADER_WIDTH));
        let text_width = (row_width - if logo.is_some() { LOGO_SIZE + LOGO_SPACING } else { 0.0 }).max(0.0);
        hstack(LOGO_SPACING)
            .items_start()
            .w_full()
            .max_w(px(row_width))
            .when_some(logo, |row, logo| {
                row.child(
                    div().pt(px(1.0)).child(
                        UserLogoUi::new(ElementId::NamedChild(Arc::new(self.id.clone()), "logo".into()))
                            .source(SharedString::from(logo.to_owned()))
                            .name(SharedString::from(title.to_owned()))
                            .size(LOGO_SIZE),
                    ),
                )
            })
            .child(
                div()
                    .flex_shrink()
                    .min_w_0()
                    .max_w(px(text_width))
                    .text_size(px(14.0))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(accent)
                    .line_clamp(2)
                    .text_ellipsis()
                    .child(text),
            )
            .child(spacer_min_w(0.0))
    }
}

impl RenderOnce for MessageForwardHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let accent = if self.is_own { theme.content_on(theme.primary._100).opacity(0.85) } else { theme.primary._100 };
        let title = self.info.from_chat_title.clone().unwrap_or_else(|| self.info.sender_name.clone());
        let logo = self.info.from_chat_logo.clone().or_else(|| self.info.sender_logo.clone());
        let header = self.header(&title, logo.as_deref(), accent);

        match self.on_open_source.clone() {
            Some(on_open_source) => header
                .cursor_pointer()
                .bubble_tap_target_priority(self.id.clone(), move |window: &mut Window, cx: &mut App| {
                    on_open_source(window, cx)
                })
                .into_any_element(),
            None => header.into_any_element(),
        }
    }
}
