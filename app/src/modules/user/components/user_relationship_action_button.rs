use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, Window};
use rise_i18n::tr;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::modules::user::components::user_relationship_list_contract::UserRelationshipRowMetrics;
use crate::modules::user::stores::user::user_actions::user_types::UserListRelationship;

#[derive(IntoElement)]
pub struct UserRelationshipActionButton {
    id: ElementId,
    relationship: UserListRelationship,
    is_pending: bool,
    action: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl UserRelationshipActionButton {
    pub fn new(
        id: impl Into<ElementId>,
        relationship: UserListRelationship,
        is_pending: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), relationship, is_pending, action: Rc::new(action) }
    }
}

impl RenderOnce for UserRelationshipActionButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = self.relationship.follow_button_state();
        let title = tr(state.title_key());
        let is_primary = state.is_primary();
        let action = self.action;

        div().flex_none().child(
            LiquidGlassButton::new(self.id, LiquidGlassButtonShape::Capsule)
                .horizontal_padding(10.0)
                .vertical_padding(0.0)
                .tint(is_primary.then(|| theme.primary._100.opacity(0.12)))
                .disabled(self.is_pending)
                .accessibility_label(Some(title.clone()))
                .on_press(move |window, cx| action(window, cx))
                .child(
                    div()
                        .min_h(px(UserRelationshipRowMetrics::relationship_button_minimum_height()))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            MainText::new(title)
                                .px(13.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(if is_primary { theme.primary._100 } else { theme.text.primary })
                                .number_of_lines(1)
                                .minimum_scale_factor(0.85),
                        ),
                ),
        )
    }
}
