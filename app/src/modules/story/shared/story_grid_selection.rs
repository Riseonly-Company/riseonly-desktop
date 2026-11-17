use gpui::{App, Context, Entity};
use rise_canon::RiseGuideSignal;
use rise_platform::haptics::selection_changed;

use crate::core::guides::guide_facts::GuideFacts;
use crate::modules::story::shared::story_guide_sandbox_hooks::story_guide_sandbox_hooks;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;

/// multi selection state of a stories grid, a presenter with no server data
/// singleton, three consumers in different trees
/// cleared on profile or tab switch, ids are per author
pub struct StoryGridSelection {
    is_active: bool,
    selected_ids: Vec<i64>,
    owner_user_id: String,
    scope: StoryProfileScope,
}

impl StoryGridSelection {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            is_active: false,
            selected_ids: Vec::new(),
            owner_user_id: String::new(),
            scope: StoryProfileScope::Profile,
        }
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    /// real ids are positive, a stories lesson practice ids are negative and may be picked too
    fn is_selectable(story_id: i64, cx: &App) -> bool {
        story_id > 0 || (story_guide_sandbox_hooks().owns_story)(story_id, cx)
    }

    fn report_guide_facts(&self, cx: &mut App) {
        GuideFacts::set(RiseGuideSignal::StoriesSelecting, self.is_active, cx);
        GuideFacts::set(
            RiseGuideSignal::StoriesSelectedSeveral,
            self.is_active && self.selected_ids.len() >= 2,
            cx,
        );
    }

    fn set_is_active(&mut self, is_active: bool, cx: &mut App) {
        self.is_active = is_active;
        self.report_guide_facts(cx);
    }

    fn set_selected_ids(&mut self, selected_ids: Vec<i64>, cx: &mut App) {
        self.selected_ids = selected_ids;
        self.report_guide_facts(cx);
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn selected_ids(&self) -> &[i64] {
        &self.selected_ids
    }

    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    pub fn scope(&self) -> StoryProfileScope {
        self.scope
    }

    pub fn count(&self) -> usize {
        self.selected_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selected_ids.is_empty()
    }

    pub fn is_active_for(&self, owner_user_id: &str, scope: StoryProfileScope) -> bool {
        self.is_active && self.owner_user_id == owner_user_id && self.scope == scope
    }

    pub fn contains(&self, story_id: i64) -> bool {
        self.selected_ids.contains(&story_id)
    }

    /// ordinal shown in the circle, 1 based like the media picker
    pub fn order(&self, of_story_id: i64) -> Option<usize> {
        self.selected_ids.iter().position(|id| *id == of_story_id).map(|index| index + 1)
    }

    pub fn begin(
        &mut self,
        owner_user_id: &str,
        scope: StoryProfileScope,
        with_story_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        if self.owner_user_id != owner_user_id || self.scope != scope {
            self.set_selected_ids(Vec::new(), cx);
        }
        self.owner_user_id = owner_user_id.to_owned();
        self.scope = scope;
        self.set_is_active(true, cx);
        if let Some(story_id) = with_story_id
            && Self::is_selectable(story_id, cx)
            && !self.selected_ids.contains(&story_id)
        {
            let mut selected_ids = self.selected_ids.clone();
            selected_ids.push(story_id);
            self.set_selected_ids(selected_ids, cx);
        }
        let _ = selection_changed();
        cx.notify();
    }

    pub fn toggle(&mut self, story_id: i64, cx: &mut Context<Self>) {
        if !self.is_active || !Self::is_selectable(story_id, cx) {
            return;
        }
        let mut selected_ids = self.selected_ids.clone();
        if let Some(index) = selected_ids.iter().position(|id| *id == story_id) {
            selected_ids.remove(index);
        } else {
            selected_ids.push(story_id);
        }
        self.set_selected_ids(selected_ids, cx);
        let _ = selection_changed();
        cx.notify();
    }

    pub fn end(&mut self, cx: &mut Context<Self>) {
        if !self.is_active {
            return;
        }
        self.set_is_active(false, cx);
        self.set_selected_ids(Vec::new(), cx);
        cx.notify();
    }

    pub fn prune(&mut self, against_available_ids: &std::collections::HashSet<i64>, cx: &mut Context<Self>) {
        if !self.is_active {
            return;
        }
        let kept: Vec<i64> = self
            .selected_ids
            .iter()
            .copied()
            .filter(|id| against_available_ids.contains(id))
            .collect();
        if kept.len() == self.selected_ids.len() {
            return;
        }
        self.set_selected_ids(kept, cx);
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.set_is_active(false, cx);
        self.set_selected_ids(Vec::new(), cx);
        self.owner_user_id = String::new();
        self.scope = StoryProfileScope::Profile;
        cx.notify();
    }
}
