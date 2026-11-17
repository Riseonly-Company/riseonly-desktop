use gpui::App;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::story::shared::story_guide_sandbox_hooks::{StoryGuideSandboxHooks, set_story_guide_sandbox_hooks};
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_wire_types::{HighlightDTO, StoryAuthorDTO, StoryDTO};

pub fn install_story_guide_sandbox_hooks() {
    let _ = set_story_guide_sandbox_hooks(StoryGuideSandboxHooks {
        owns_story,
        owns_stories,
        owns_highlight,
        owns_close_friends,
        profile_stories,
        archive_stories,
        highlight_stories,
        highlights,
        user_stories,
        add_stories,
        remove_story,
        delete_stories,
        create_highlight,
        update_highlight,
        delete_highlight,
        close_friends_suggestion,
    });
}

fn owns_story(id: i64, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_story(id))
}

fn owns_stories(ids: &[i64], cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_stories(ids))
}

fn owns_highlight(id: i64, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_highlight(id))
}

fn owns_close_friends(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_close_friends())
}

fn profile_stories(owner_id: &str, scope: StoryProfileScope, cx: &App) -> Option<Vec<StoryDTO>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).profile_stories(owner_id, scope, cx))
}

fn archive_stories(ids: &[i64], archived: bool, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.archive_stories(ids, archived, cx));
}

fn highlight_stories(highlight_id: i64, cx: &App) -> Option<Vec<StoryDTO>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).highlight_stories(highlight_id))
}

fn highlights(owner_id: &str, real: Vec<HighlightDTO>, cx: &App) -> Vec<HighlightDTO> {
    match GuideSandbox::existing(cx) {
        Some(sandbox) => sandbox.read(cx).highlights(owner_id, real, cx),
        None => real,
    }
}

fn user_stories(owner_id: &str, cx: &App) -> Option<Vec<StoryDTO>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).user_stories(owner_id, cx))
}

fn add_stories(ids: &[i64], to_highlight: i64, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.add_stories(ids, to_highlight, cx));
}

fn remove_story(story_id: i64, from_highlight: i64, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.remove_story(story_id, from_highlight, cx));
}

fn delete_stories(ids: &[i64], cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_stories(ids, cx));
}

fn create_highlight(title: &str, story_ids: &[i64], cx: &mut App) -> Option<HighlightDTO> {
    GuideSandbox::shared(cx)
        .update(cx, |sandbox, cx| sandbox.create_highlight(title.to_owned(), story_ids.to_vec(), cx))
}

fn update_highlight(id: i64, title: Option<&str>, cover_story_id: Option<i64>, cx: &mut App) {
    GuideSandbox::shared(cx)
        .update(cx, |sandbox, cx| sandbox.update_highlight(id, title.map(str::to_owned), cover_story_id, cx));
}

fn delete_highlight(id: i64, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_highlight(id, cx));
}

fn close_friends_suggestion(cx: &App) -> Option<StoryAuthorDTO> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).close_friends_suggestion())
}
