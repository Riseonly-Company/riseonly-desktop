use gpui::IntoElement;

use crate::modules::user::components::profile_content::pages::goals::goals::Goals;
use crate::modules::user::components::profile_content::profile_content_goals_hooks::{
    ProfileContentGoalsHooks, set_profile_content_goals_hooks,
};

pub fn install_profile_content_goals_hooks() {
    set_profile_content_goals_hooks(ProfileContentGoalsHooks {
        goals_page: |id, is_own_profile, foreign_goals| {
            Goals::new(id).is_own_profile(is_own_profile).foreign_goals(foreign_goals).into_any_element()
        },
    });
}
