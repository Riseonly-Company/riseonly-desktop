use gpui::{AnyElement, ElementId};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::user::stores::user::user_actions::user_types::GoalItem;

/// goals tab page of the profile, built by the profile pages
pub struct ProfileContentGoalsHooks {
    pub goals_page: fn(ElementId, bool, Vec<GoalItem>) -> AnyElement,
}

pub(crate) static PROFILE_CONTENT_GOALS_HOOKS: FeatureHookSlot<ProfileContentGoalsHooks> =
    FeatureHookSlot::new("PROFILE_CONTENT_GOALS_HOOKS");

pub fn set_profile_content_goals_hooks(hooks: ProfileContentGoalsHooks) {
    let _ = PROFILE_CONTENT_GOALS_HOOKS.set(hooks);
}

pub fn profile_content_goals_hooks() -> Option<&'static ProfileContentGoalsHooks> {
    PROFILE_CONTENT_GOALS_HOOKS.get()
}
