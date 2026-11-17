use std::rc::Rc;
use std::sync::OnceLock;

use gpui::{App, Window};

/// runs when the user stories turn out to be gone
pub type UserLogoStoryUnavailable = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct UserLogoStoryHooks {
    pub has_active_stories: fn(user_id: Option<&str>, cx: &mut App) -> bool,
    pub has_unseen_stories: fn(user_id: Option<&str>, cx: &mut App) -> bool,
    pub has_known_stories_state: fn(user_id: Option<&str>, cx: &mut App) -> bool,
    pub load_stories_state: fn(user_ids: Vec<String>, cx: &mut App),
    pub open_stories: fn(
        for_user_id: &str,
        on_unavailable: UserLogoStoryUnavailable,
        window: &mut Window,
        cx: &mut App,
    ),
}

static USER_LOGO_STORY_HOOKS: OnceLock<UserLogoStoryHooks> = OnceLock::new();

/// installs the hooks once, a second call keeps the first hooks
pub fn set_user_logo_story_hooks(hooks: UserLogoStoryHooks) {
    let _ = USER_LOGO_STORY_HOOKS.set(hooks);
}

pub fn user_logo_story_hooks() -> Option<&'static UserLogoStoryHooks> {
    USER_LOGO_STORY_HOOKS.get()
}
