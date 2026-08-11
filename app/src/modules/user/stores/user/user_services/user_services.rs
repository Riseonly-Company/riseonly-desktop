use std::sync::Arc;

use gpui::{App, Context, Entity, Global};
use rise_widgets::PageStatus;

use crate::modules::user::engine::rise_user_domain::RiseUserDomain;
use crate::modules::user::engine::rise_user_engine_models::{ProfileKey, ProfileModel};
use crate::modules::user::engine::rise_user_presentation::{ProfileEntry, UserSnapshot};

use super::super::user_actions::UserActionsStore;
use super::super::user_interactions::UserInteractionsStore;

pub struct UserServicesStore {
    snapshot: Arc<UserSnapshot>,
}

impl UserServicesStore {
    pub fn new(domain: Arc<RiseUserDomain>, cx: &mut Context<Self>) -> Self {
        let mut updates = domain.subscribe();

        cx.spawn(async move |this, cx| {
            while updates.changed().await.is_ok() {
                let next = Arc::clone(&updates.borrow_and_update());
                if this
                    .update(cx, |store, cx| {
                        store.snapshot = next;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        Self {
            snapshot: domain.snapshot(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.snapshot.revision
    }

    pub fn viewer_id(&self) -> Option<&str> {
        self.snapshot.viewer_id.as_deref()
    }

    pub fn entry(&self, key: &ProfileKey) -> Option<&ProfileEntry> {
        self.snapshot.entry(key)
    }

    pub fn profile(&self, key: &ProfileKey) -> Option<&ProfileModel> {
        self.snapshot.profile(key)
    }

    pub fn status(&self, key: &ProfileKey) -> PageStatus {
        self.snapshot
            .entry(key)
            .map(|entry| entry.status)
            .unwrap_or_default()
    }

    pub fn has_content(&self, key: &ProfileKey) -> bool {
        self.snapshot.profile(key).is_some()
    }

    pub fn is_viewer(&self, key: &ProfileKey) -> bool {
        self.snapshot.is_viewer(key)
    }

    pub fn is_follow_in_flight(&self, key: &ProfileKey) -> bool {
        self.snapshot
            .entry(key)
            .is_some_and(|entry| entry.follow_in_flight)
    }

    /// The tag a profile's own post feeds are addressed by. Absent until the
    /// payload lands, because `post.get_user_publications` keys on tag, not id.
    pub fn feed_tag(&self, key: &ProfileKey) -> Option<String> {
        self.snapshot
            .profile(key)
            .map(|profile| profile.tag.trim().to_owned())
            .filter(|tag| !tag.is_empty())
    }
}

pub struct UserStores {
    pub interactions: UserInteractionsStore,
    pub actions: UserActionsStore,
    pub services: Entity<UserServicesStore>,
}

impl Global for UserStores {}

pub fn user_stores(cx: &App) -> &UserStores {
    cx.global::<UserStores>()
}

pub fn try_user_stores(cx: &App) -> Option<&UserStores> {
    cx.try_global::<UserStores>()
}
