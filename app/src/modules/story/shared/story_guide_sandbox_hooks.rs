use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_wire_types::{HighlightDTO, StoryAuthorDTO, StoryDTO};

pub struct StoryGuideSandboxHooks {
    pub owns_story: fn(id: i64, cx: &App) -> bool,
    pub owns_stories: fn(ids: &[i64], cx: &App) -> bool,
    pub owns_highlight: fn(id: i64, cx: &App) -> bool,
    pub owns_close_friends: fn(cx: &App) -> bool,
    pub profile_stories: fn(owner_id: &str, scope: StoryProfileScope, cx: &App) -> Option<Vec<StoryDTO>>,
    pub archive_stories: fn(ids: &[i64], archived: bool, cx: &mut App),
    pub highlight_stories: fn(highlight_id: i64, cx: &App) -> Option<Vec<StoryDTO>>,
    pub highlights: fn(owner_id: &str, real: Vec<HighlightDTO>, cx: &App) -> Vec<HighlightDTO>,
    pub user_stories: fn(owner_id: &str, cx: &App) -> Option<Vec<StoryDTO>>,
    pub add_stories: fn(ids: &[i64], to_highlight: i64, cx: &mut App),
    pub remove_story: fn(story_id: i64, from_highlight: i64, cx: &mut App),
    pub delete_stories: fn(ids: &[i64], cx: &mut App),
    pub create_highlight: fn(title: &str, story_ids: &[i64], cx: &mut App) -> Option<HighlightDTO>,
    pub update_highlight: fn(id: i64, title: Option<&str>, cover_story_id: Option<i64>, cx: &mut App),
    pub delete_highlight: fn(id: i64, cx: &mut App),
    pub close_friends_suggestion: fn(cx: &App) -> Option<StoryAuthorDTO>,
}

pub(crate) static HOOKS: FeatureHookSlot<StoryGuideSandboxHooks> = FeatureHookSlot::new("HOOKS");

static UNINSTALLED: StoryGuideSandboxHooks = StoryGuideSandboxHooks {
    owns_story: |_, _| false,
    owns_stories: |_, _| false,
    owns_highlight: |_, _| false,
    owns_close_friends: |_| false,
    profile_stories: |_, _, _| None,
    archive_stories: |_, _, _| {},
    highlight_stories: |_, _| None,
    highlights: |_, real, _| real,
    user_stories: |_, _| None,
    add_stories: |_, _, _| {},
    remove_story: |_, _, _| {},
    delete_stories: |_, _| {},
    create_highlight: |_, _, _| None,
    update_highlight: |_, _, _, _| {},
    delete_highlight: |_, _| {},
    close_friends_suggestion: |_| None,
};

/// installs the hooks as a whole,
/// a second call is refused and hands them back so the first installer wins
pub fn set_story_guide_sandbox_hooks(hooks: StoryGuideSandboxHooks) -> Result<(), StoryGuideSandboxHooks> {
    HOOKS.set(hooks)
}

pub fn story_guide_sandbox_hooks() -> &'static StoryGuideSandboxHooks {
    HOOKS.get().unwrap_or(&UNINSTALLED)
}
