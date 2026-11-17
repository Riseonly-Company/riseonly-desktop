use std::collections::{HashMap, HashSet};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_character::int_from_string;

use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SAI_WS_DID_CONNECT;
use crate::modules::post::stores::post_actions::post_watch_actions::PostWatchActionsStore;

pub struct PostWatchServicesStore {
    desired_by_owner: HashMap<String, HashSet<i64>>,
    currently_watched: HashSet<i64>,
    reconcile_scheduled: bool,
}

impl PostWatchServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (did_connect, mut on_did_connect) = mpsc::unbounded::<()>();
        RiseAppEvents::default_center()
            .observe(SAI_WS_DID_CONNECT, move |_| {
                let _ = did_connect.unbounded_send(());
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while on_did_connect.next().await.is_some() {
                if this.update(cx, |store, cx| store.rewatch_all(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            desired_by_owner: HashMap::new(),
            currently_watched: HashSet::new(),
            reconcile_scheduled: false,
        }
    }

    pub fn set_desired<I, S>(&mut self, post_ids: I, owner: &str, cx: &mut Context<Self>)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let ids: HashSet<i64> = post_ids
            .into_iter()
            .filter_map(|id| {
                let id = id.as_ref();
                if GetSaiServerId::is_temp_id(Some(id)) {
                    return None;
                }
                int_from_string(id).filter(|value| *value > 0)
            })
            .collect();
        if ids.is_empty() {
            if self.desired_by_owner.remove(owner).is_none() {
                return;
            }
        } else {
            if self.desired_by_owner.get(owner) == Some(&ids) {
                return;
            }
            self.desired_by_owner.insert(owner.to_owned(), ids);
        }
        cx.notify();
        self.schedule_reconcile(cx);
    }

    pub fn rewatch_all(&self, cx: &mut App) {
        if self.currently_watched.is_empty() {
            return;
        }
        let post_ids: Vec<i64> = self.currently_watched.iter().copied().collect();
        PostWatchActionsStore::shared(cx).update(cx, |store, cx| store.watch_action(post_ids, cx));
    }

    fn schedule_reconcile(&mut self, cx: &mut Context<Self>) {
        if self.reconcile_scheduled {
            return;
        }
        self.reconcile_scheduled = true;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |store, cx| {
                store.reconcile_scheduled = false;
                store.reconcile(cx);
            });
        })
        .detach();
    }

    fn reconcile(&mut self, cx: &mut Context<Self>) {
        let union: HashSet<i64> = self.desired_by_owner.values().flat_map(|ids| ids.iter().copied()).collect();
        let to_add: Vec<i64> = union.difference(&self.currently_watched).copied().collect();
        let to_remove: Vec<i64> = self.currently_watched.difference(&union).copied().collect();
        self.currently_watched = union;
        cx.notify();
        if !to_add.is_empty() {
            PostWatchActionsStore::shared(cx).update(cx, |store, cx| store.watch_action(to_add, cx));
        }
        if !to_remove.is_empty() {
            PostWatchActionsStore::shared(cx).update(cx, |store, cx| store.unwatch_action(to_remove, cx));
        }
    }
}
