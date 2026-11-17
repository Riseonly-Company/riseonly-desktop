use std::borrow::Cow;

use gpui::{App, Context, Entity};
use rise_i18n::{FormatArg, tr, tr_args};

use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::stores::shorts::shorts_actions::shorts_actions::ShortsActionsStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::UserListRelationship;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub struct ShortsInteractionsStore {}

impl ShortsInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn actions(cx: &mut App) -> Entity<ShortsActionsStore> {
        ShortsActionsStore::shared(cx)
    }

    fn service(cx: &mut App) -> Entity<ShortsServicesStore> {
        ShortsServicesStore::shared(cx)
    }

    pub fn load_initial(&mut self, cx: &mut Context<Self>) {
        if Self::service(cx).read(cx).did_load() {
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).get_shorts_feed_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_shorts_feed_action(None, true, true, cx));
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_shorts_feed_action(None, false, true, cx));
    }

    pub fn tab_reselected(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.request_feed_top(cx));
        self.refresh(cx);
    }

    pub fn feed_chrome_hidden_changed(&mut self, hidden: bool, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.update_feed_chrome_hidden(hidden, cx));
    }

    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if !service.update(cx, |service, cx| service.has_more(cx)) {
            return;
        }
        let Some(anchor) = service.update(cx, |service, cx| service.relative_id(cx)).filter(|anchor| !anchor.is_empty())
        else {
            return;
        };
        let actions = Self::actions(cx);
        if actions.read(cx).get_shorts_feed_sai.is_pending() {
            return;
        }
        let pagination = SaiWs::pagination_state(&ShortsServicesStore::feed_cache_key(), cx);
        if !pagination.read(cx).trailing().can_request_automatically() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_shorts_feed_action(Some(&anchor), false, true, cx));
    }

    pub fn resolved_author_id(&self, post: &GetPostFeedResponse) -> Option<String> {
        RiseIdentity::author(Some(&post.author_id), None, post.author.id.as_deref()).wire_id().map(str::to_owned)
    }

    pub fn author_relationship(&self, post: &GetPostFeedResponse, cx: &mut App) -> UserListRelationship {
        let wire = Self::service(cx).read(cx).author_relationship(post);
        let Some(author_id) = self.resolved_author_id(post) else {
            return wire;
        };

        UserServicesStore::shared(cx).read(cx).optimistic_follow_relationship(&author_id, wire)
    }

    pub fn toggle_follow_author(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        let Some(author_id) = self.resolved_author_id(post) else {
            return;
        };

        let relationship = self.author_relationship(post, cx);
        UserInteractionsStore::shared(cx).update(cx, |user, cx| {
            user.toggle_subscribe_relationship(&author_id, &post.author.tag, relationship, cx);
        });

        if relationship.is_subbed {
            return;
        }

        let post_id = post.rise_view_target_post_id();
        EngagementInteractionsStore::shared(cx).update(cx, |engagement, cx| {
            engagement.follow(&author_id, Some(&post_id), cx);
        });
    }

    pub fn not_interested(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.not_interested_action(&post.id, cx));
        Notifier::send(
            tr_args("shorts_not_interested_notice", &[FormatArg::Text(Cow::Owned(tr("riffs")))]),
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }
}
