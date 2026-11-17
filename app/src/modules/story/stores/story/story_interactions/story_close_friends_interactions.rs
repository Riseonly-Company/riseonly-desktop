use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::{RiseGuideSignal, RiseTimingCanon};
use rise_core::foundation::foundation_date::RiseClock;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::story::shared::story_guide_sandbox_hooks::story_guide_sandbox_hooks;
use crate::modules::story::stores::story::story_actions::story_actions::{
    StoryActionDoneHandler, StoryActionErrorHandler, StoryActionTextHandler, StoryActionsStore,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryAuthorDTO;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

impl StoryInteractionsStore {
    pub fn close_friends_limit() -> i64 {
        StoryActionsStore::close_friends_limit()
    }

    pub fn load_close_friends(&mut self, force: bool, on_error: Option<StoryActionDoneHandler>, cx: &mut Context<Self>) {
        let key = StoryServicesStore::close_friends_cache_key();
        if !force
            && Self::service(cx).update(cx, |service, cx| service.close_friends_loaded(cx))
            && let Some(loaded_at) = self.profile_stories_last_loaded_at.get(&key)
            && RiseClock::now().time_interval_since_1970() - loaded_at
                < RiseTimingCanon::freshness_seconds("profile_stories")
        {
            return;
        }
        self.profile_stories_last_loaded_at
            .insert(key.clone(), RiseClock::now().time_interval_since_1970());
        cx.notify();
        let this = cx.weak_entity();
        let failure: StoryActionErrorHandler = Rc::new(move |_: &anyhow::Error, cx: &mut App| {
            let this = this.clone();
            let key = key.clone();
            let on_error = on_error.clone();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| {
                    this.profile_stories_last_loaded_at.remove(&key);
                    cx.notify();
                })
                .ok();
                if let Some(on_error) = on_error.as_ref() {
                    on_error(cx);
                }
            });
        });
        Self::actions(cx).update(cx, |actions, cx| actions.get_close_friends_action(None, Some(failure), cx));
    }

    pub fn save_close_friends(
        &mut self,
        friends: &[StoryAuthorDTO],
        on_success: Option<StoryActionDoneHandler>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let on_success: StoryActionDoneHandler = on_success.unwrap_or_else(|| Rc::new(|_: &mut App| {}));
        if (story_guide_sandbox_hooks().owns_close_friends)(cx) {
            GuideSignalBus::emit(RiseGuideSignal::CloseFriendsSaved);
            on_success(cx);
            return;
        }
        let on_error: StoryActionTextHandler = on_error.unwrap_or_else(|| Rc::new(|_: String, _: &mut App| {}));
        Self::actions(cx)
            .update(cx, |actions, cx| actions.set_close_friends_action(friends, on_success, on_error, cx));
        GuideSignalBus::emit(RiseGuideSignal::CloseFriendsSaved);
    }

    pub fn toggle_close_friend(
        &mut self,
        author: &StoryAuthorDTO,
        on_limit_reached: Option<StoryActionDoneHandler>,
        on_error: Option<StoryActionTextHandler>,
        cx: &mut Context<Self>,
    ) {
        let is_close = Self::service(cx).update(cx, |service, cx| service.is_close_friend(Some(&author.id), cx));
        let on_success: StoryActionDoneHandler = Rc::new(|_: &mut App| {});
        let on_limit_reached: StoryActionDoneHandler = on_limit_reached.unwrap_or_else(|| Rc::new(|_: &mut App| {}));
        let on_error: StoryActionTextHandler = on_error.unwrap_or_else(|| Rc::new(|_: String, _: &mut App| {}));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.set_close_friend_action(author, !is_close, on_success, on_limit_reached, on_error, cx);
        });
    }

    pub fn is_close_friend(&self, user_id: Option<&str>, cx: &mut App) -> bool {
        Self::service(cx).update(cx, |service, cx| service.is_close_friend(user_id, cx))
    }
}
