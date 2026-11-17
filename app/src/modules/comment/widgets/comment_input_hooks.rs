use gpui::{AnyElement, App};

use crate::core::feature_hooks::FeatureHookSlot;

/// what the comments sheet asks the comment composer
pub struct CommentInputHooks {
    pub composer: fn(&'static str, Box<dyn Fn(bool, f64, &mut App)>, Box<dyn Fn(f32, &mut App)>) -> AnyElement,
    pub preview_composer: fn(&'static str, Option<String>) -> AnyElement,
}

pub(crate) static COMMENT_INPUT_HOOKS: FeatureHookSlot<CommentInputHooks> = FeatureHookSlot::new("COMMENT_INPUT_HOOKS");

pub fn set_comment_input_hooks(hooks: CommentInputHooks) {
    let _ = COMMENT_INPUT_HOOKS.set(hooks);
}

pub fn comment_input_hooks() -> Option<&'static CommentInputHooks> {
    COMMENT_INPUT_HOOKS.get()
}
