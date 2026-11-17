use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

/// what the comments sheet asks the right panel of the front window
pub struct CommentsSheetAsideHooks {
    pub steps_back: fn(&App) -> bool,
    pub docks_comments: fn(&App) -> bool,
}

pub(crate) static COMMENTS_SHEET_ASIDE_HOOKS: FeatureHookSlot<CommentsSheetAsideHooks> =
    FeatureHookSlot::new("COMMENTS_SHEET_ASIDE_HOOKS");

pub fn set_comments_sheet_aside_hooks(hooks: CommentsSheetAsideHooks) {
    let _ = COMMENTS_SHEET_ASIDE_HOOKS.set(hooks);
}

pub fn comments_sheet_aside_hooks() -> Option<&'static CommentsSheetAsideHooks> {
    COMMENTS_SHEET_ASIDE_HOOKS.get()
}
