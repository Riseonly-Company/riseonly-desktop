use gpui::IntoElement;

use crate::core::ui::formatted_preview_text::FormattedPreviewText;
use crate::core::widgets::notifier::notifier_markup_hooks::{NotifierMarkupHooks, set_notifier_markup_hooks};

pub fn install_notifier_markup_hooks() {
    set_notifier_markup_hooks(NotifierMarkupHooks {
        preview_text: |id, markup, font_size, color, line_limit, lead| {
            FormattedPreviewText::new(id, markup, font_size, color, line_limit, lead)
                .tail_ellipsis()
                .into_any_element()
        },
    });
}
