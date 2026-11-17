use gpui::{App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, Styled, Window, px};
use rise_i18n::{FormatArg, tr_args};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::stacks::hstack;
use std::rc::Rc;

use crate::modules::user::shared::social_platform::SocialLink;

type SocialLinksPreviewRowTap = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct SocialLinksPreviewRow {
    id: ElementId,
    pub links: Vec<SocialLink>,
    pub on_tap: SocialLinksPreviewRowTap,
}

impl SocialLinksPreviewRow {
    pub fn new(
        id: impl Into<ElementId>,
        links: Vec<SocialLink>,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), links, on_tap: Rc::new(on_tap) }
    }
}

impl RenderOnce for SocialLinksPreviewRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let first = self.links.first();
        let extra = self.links.len().saturating_sub(1);
        let on_tap = self.on_tap.clone();

        plain(self.id).w_full().on_press(move |window, cx| on_tap(window, cx)).child(
            hstack(9.0)
                .w_full()
                .children(IconUi::sized("link", px(17.0), theme.text.primary, false))
                .children(first.map(|first| {
                    MainText::new(first.display_url())
                        .px(13.0)
                        .tac(TextAlignT::Left)
                        .font_weight(FontWeight::BOLD)
                        .number_of_lines(1)
                }))
                .children((extra > 0).then(|| {
                    MainText::new(tr_args("social_links_and_more", &[FormatArg::Int(extra as i64)]))
                        .px(13.0)
                        .tac(TextAlignT::Left)
                        .color(theme.text.secondary)
                        .number_of_lines(1)
                })),
        )
    }
}
