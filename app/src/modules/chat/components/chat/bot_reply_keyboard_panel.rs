use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, FontWeight, SharedString, Window};
use rise_i18n::tr;
use rise_theme::alpha;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::material::Material;
use rise_ui::swiftui::prelude::*;

use crate::modules::chat::stores::message::message_actions::message_types::MessageBotKeyboardButton;

pub type BotReplyKeyboardPress = Rc<dyn Fn(&MessageBotKeyboardButton, &mut Window, &mut App)>;
pub type BotReplyKeyboardClose = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct BotReplyKeyboardPanel {
    pub id: ElementId,
    pub rows: Vec<Vec<MessageBotKeyboardButton>>,
    pub on_press: BotReplyKeyboardPress,
    pub on_close: BotReplyKeyboardClose,
}

impl BotReplyKeyboardPanel {
    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }
}

impl RenderOnce for BotReplyKeyboardPanel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let on_close = self.on_close.clone();
        let header = hstack(8.)
            .w_full()
            .children(IconUi::sized("keyboard.fill", px(12.), theme.primary._100, true))
            .child(
                MainText::new(tr("bot_reply_keyboard"))
                    .px(12.)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.secondary),
            )
            .child(spacer())
            .child(
                plain(self.child_id("close"))
                    .on_press(move |window, cx| on_close(window, cx))
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("xmark", px(11.), theme.text.secondary, false)),
            );

        let row_count = self.rows.len();
        let mut stack = vstack(6.).w_full();
        for (row_index, row) in self.rows.iter().enumerate() {
            let mut line = hstack(6.).w_full();
            for (column_index, button) in row.iter().enumerate() {
                let on_press = self.on_press.clone();
                let pressed = button.clone();
                let shape = Shape::Capsule;
                line = line.child(
                    plain(self.child_id(format!("button-{row_index}-{column_index}")))
                        .on_press(move |window, cx| on_press(&pressed, window, cx))
                        .flex_1()
                        .min_w_0()
                        .relative()
                        .child(
                            hstack(5.)
                                .w_full()
                                .min_h(px(42.))
                                .justify_center()
                                .padding_edges(EdgeSet::HORIZONTAL, 10.)
                                .bg(alpha(theme.bg._300, 0.92))
                                .clip_shape(shape)
                                .child(
                                    MainText::new(button.text.clone())
                                        .px(14.)
                                        .font_weight(FontWeight::MEDIUM)
                                        .color(theme.text.primary)
                                        .number_of_lines(2),
                                )
                                .when(button.web_app_url.is_some(), |label| {
                                    label.children(IconUi::sized(
                                        "rectangle.on.rectangle.angled",
                                        px(10.),
                                        theme.primary._100,
                                        false,
                                    ))
                                }),
                        )
                        .child(centred_stroke(alpha(theme.border._200, 0.55), 0.5, shape)),
                );
            }
            stack = stack.child(line);
        }

        let height = (row_count as f32 * 48.0).min(210.0);
        let scroller = div().id(self.child_id("rows")).w_full().h(px(height)).overflow_y_scroll().child(stack);

        vstack(7.)
            .w_full()
            .padding_edges(EdgeSet::HORIZONTAL, 12.)
            .padding_edges(EdgeSet::TOP, 8.)
            .padding_edges(EdgeSet::BOTTOM, 9.)
            .bg(alpha(theme.bg._100, Material::UltraThin.painted_opacity()))
            .child(header)
            .child(scroller)
    }
}
