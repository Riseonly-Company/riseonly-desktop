use gpui::{AnyElement, App, FontWeight, IntoElement, ParentElement, RenderOnce, Styled, Window, black, div, px, white};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::frame::{EdgeSet, SwiftUiFrame};
use rise_ui::swiftui::shapes::{SwiftUiShapes, pass_through};
use rise_ui::swiftui::stacks::{Alignment, hstack, zstack};

use crate::modules::post::stores::post_actions::post_types::PostModerationBanner;

#[derive(IntoElement)]
pub struct PostModerationDisabledOverlay;

impl RenderOnce for PostModerationDisabledOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        pass_through().size_full().child(
            zstack(Alignment::Center)
                .size_full()
                .child(div().size_full().bg(black().opacity(0.45)))
                .child(
                    div()
                        .p(px(8.0))
                        .circle()
                        .bg(black().opacity(0.42))
                        .children(IconUi::sized("eye.slash.fill", px(15.0), white(), false)),
                ),
        )
    }
}

/// grid tile mark for a post only its author sees
/// eye for a hidden post, a short word for one under review or rejected
/// the full sentence is on the post itself
#[derive(IntoElement)]
pub struct PostModerationGridOverlay {
    pub banner: PostModerationBanner,
}

impl PostModerationGridOverlay {
    pub fn new(banner: PostModerationBanner) -> Self {
        Self { banner }
    }
}

impl RenderOnce for PostModerationGridOverlay {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let banner = self.banner;
        let Some(badge_key) = banner.badge_key() else {
            return PostModerationDisabledOverlay.into_any_element();
        };
        let error = ThemeStore::shared(cx).read(cx).current_theme().semantic.error_100;
        let capsule_fill = if banner.is_error() { error.opacity(0.9) } else { black().opacity(0.42) };
        let badge: AnyElement = hstack(4.0)
            .children(IconUi::sized(banner.symbol_name(), px(11.0), white(), false))
            .child(
                MainText::new(tr(badge_key))
                    .px(11.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(white())
                    .number_of_lines(1),
            )
            .padding_edges(EdgeSet::HORIZONTAL, 8.0)
            .padding_edges(EdgeSet::VERTICAL, 5.0)
            .capsule()
            .bg(capsule_fill)
            .into_any_element();
        pass_through()
            .size_full()
            .child(
                zstack(Alignment::Center)
                    .size_full()
                    .child(div().size_full().bg(black().opacity(if banner.is_error() { 0.45 } else { 0.25 })))
                    .child(div().padding_edges(EdgeSet::HORIZONTAL, 6.0).child(badge)),
            )
            .into_any_element()
    }
}
