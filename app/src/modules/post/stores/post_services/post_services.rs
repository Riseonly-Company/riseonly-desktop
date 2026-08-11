use std::sync::Arc;

use gpui::{App, Context, Entity, EventEmitter, Global};

use crate::modules::post::engine::rise_post_domain::RisePostDomain;
use crate::modules::post::engine::rise_post_engine_models::PostEntry;
use crate::modules::post::engine::rise_post_repository::{FeedPage, PostSnapshot};
use crate::modules::post::engine::rise_post_rpc::PostScope;

use super::super::post_actions::PostActionsStore;
use super::super::post_interactions::PostInteractionsStore;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PostUiEvent {
    OpenComments { post_id: String, title: String },
    OpenProfile { user_id: String },
}

pub struct PostServicesStore {
    snapshot: Arc<PostSnapshot>,
}

impl EventEmitter<PostUiEvent> for PostServicesStore {}

impl PostServicesStore {
    pub fn new(domain: Arc<RisePostDomain>, cx: &mut Context<Self>) -> Self {
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

    pub fn page(&self, scope: &PostScope) -> Option<&FeedPage> {
        self.snapshot.page(scope)
    }

    pub fn items(&self, scope: &PostScope) -> &[PostEntry] {
        self.snapshot
            .page(scope)
            .map(|page| page.items.as_slice())
            .unwrap_or_default()
    }

    pub fn item(&self, scope: &PostScope, index: usize) -> Option<&PostEntry> {
        self.items(scope).get(index)
    }

    pub fn has_more(&self, scope: &PostScope) -> bool {
        self.snapshot.page(scope).is_some_and(|page| page.has_more)
    }

    pub fn is_loading_more(&self, scope: &PostScope) -> bool {
        self.snapshot
            .page(scope)
            .is_some_and(|page| page.is_loading_more)
    }

    pub fn open_comments(&self, post: &PostEntry, cx: &mut Context<Self>) {
        cx.emit(PostUiEvent::OpenComments {
            post_id: post.id.clone(),
            title: post.author.name.clone(),
        });
    }

    pub fn open_profile(&self, user_id: String, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        cx.emit(PostUiEvent::OpenProfile { user_id });
    }
}

pub struct PostStores {
    pub interactions: PostInteractionsStore,
    pub actions: PostActionsStore,
    pub services: Entity<PostServicesStore>,
}

impl Global for PostStores {}

pub fn post_stores(cx: &App) -> &PostStores {
    cx.global::<PostStores>()
}

pub fn try_post_stores(cx: &App) -> Option<&PostStores> {
    cx.try_global::<PostStores>()
}
