use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, IntoElement, RenderOnce, Window};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;

/// stands where the sharers own screen share would render
/// drawing your own capture inside that screen repeats forever, hall of mirrors
/// viewers still get the real share
#[derive(IntoElement)]
pub struct RiseRTCOwnScreenShareWall {
    id: ElementId,
    theme: AppTheme,
    compact: bool,
    on_stop: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl RiseRTCOwnScreenShareWall {
    pub fn new(id: impl Into<ElementId>, theme: AppTheme) -> Self {
        Self { id: id.into(), theme, compact: false, on_stop: None }
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    pub fn on_stop(mut self, on_stop: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_stop = Some(Rc::new(on_stop));
        self
    }
}

impl RenderOnce for RiseRTCOwnScreenShareWall {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let compact = self.compact;
        let stop_button = match (compact, self.on_stop) {
            (false, Some(on_stop)) => {
                let content_color = theme.content_on(theme.primary._100);
                Some(
                    div().pt(px(6.0)).child(
                        ButtonUi::new(self.id)
                            .background_color(theme.primary._100)
                            .height(42.0)
                            .fit_content(true)
                            .padding_horizontal(18.0)
                            .gap(8.0)
                            .on_press(move |window, cx| on_stop(window, cx))
                            .children(IconUi::sized("rectangle.on.rectangle.slash", px(14.0), content_color, false))
                            .child(MainText::new(tr("rtc_stop_stream")).px(14.0).font_weight(FontWeight::SEMIBOLD)),
                    ),
                )
            }
            _ => None,
        };

        div().size_full().flex().items_center().justify_center().bg(theme.bg._000).child(
            vstack(if compact { 6.0 } else { 12.0 })
                .w_full()
                .items_center()
                .px(px(if compact { 8.0 } else { 32.0 }))
                .children(IconUi::sized(
                    "rectangle.on.rectangle",
                    px(if compact { 30.0 } else { 76.0 }),
                    theme.text.secondary,
                    false,
                ))
                .child({
                    let title = MainText::new(tr("rtc_own_screen_share_title"))
                        .w_full()
                        .px(if compact { 12.0 } else { 17.0 })
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary)
                        .number_of_lines(if compact { 3 } else { 0 });
                    if compact { title.minimum_scale_factor(0.6) } else { title }
                })
                .when(!compact, |stack| {
                    stack.child(
                        div().max_w(px(320.0)).child(
                            MainText::new(tr("rtc_own_screen_share_body"))
                                .px(14.0)
                                .tac(TextAlignT::Center)
                                .color(theme.text.secondary),
                        ),
                    )
                })
                .children(stop_button),
        )
    }
}
