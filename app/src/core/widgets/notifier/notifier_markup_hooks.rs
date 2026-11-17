use gpui::{AnyElement, ElementId, Hsla};
use rise_ui::swiftui::rich_text::AttributedString;

use crate::core::feature_hooks::FeatureHookSlot;

/// draws the markup body of a banner, formatted text lives above the notifier
pub struct NotifierMarkupHooks {
    pub preview_text: fn(ElementId, String, f32, Hsla, usize, Option<AttributedString>) -> AnyElement,
}

pub(crate) static NOTIFIER_MARKUP_HOOKS: FeatureHookSlot<NotifierMarkupHooks> = FeatureHookSlot::new("NOTIFIER_MARKUP_HOOKS");

pub fn set_notifier_markup_hooks(hooks: NotifierMarkupHooks) {
    let _ = NOTIFIER_MARKUP_HOOKS.set(hooks);
}

pub fn notifier_markup_hooks() -> Option<&'static NotifierMarkupHooks> {
    NOTIFIER_MARKUP_HOOKS.get()
}
