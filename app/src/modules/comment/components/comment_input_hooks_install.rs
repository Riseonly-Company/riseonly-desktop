use gpui::IntoElement;

use crate::modules::comment::components::comment_input::CommentInput;
use crate::modules::comment::widgets::comment_input_hooks::{CommentInputHooks, set_comment_input_hooks};

pub fn install_comment_input_hooks() {
    set_comment_input_hooks(CommentInputHooks {
        composer: |id, on_keyboard_visibility_change, on_picker_lift_change| {
            CommentInput::new(id)
                .on_keyboard_visibility_change(on_keyboard_visibility_change)
                .on_picker_lift_change(on_picker_lift_change)
                .into_any_element()
        },
        preview_composer: |id, preview_text| CommentInput::new(id).preview_text(preview_text).into_any_element(),
    });
}
