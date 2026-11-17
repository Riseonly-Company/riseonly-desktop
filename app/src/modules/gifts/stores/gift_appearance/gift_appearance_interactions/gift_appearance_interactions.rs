use std::collections::HashSet;
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};

use crate::modules::gifts::engine::core::rise_gifts_rpc::RiseGiftsRPC;
use crate::modules::gifts::stores::gift_appearance::gift_appearance_actions::gift_appearance_actions::GiftAppearanceActionsStore;
use crate::modules::gifts::stores::gift_appearance::gift_appearance_services::gift_appearance_services::GiftAppearanceServicesStore;

pub struct GiftAppearanceInteractionsStore {
    wanted: HashSet<String>,
    forced: HashSet<String>,
    flush_task: Option<Task<()>>,
}

impl GiftAppearanceInteractionsStore {
    const BATCH_WINDOW: Duration = Duration::from_millis(80);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            wanted: HashSet::new(),
            forced: HashSet::new(),
            flush_task: None,
        }
    }

    fn actions(cx: &mut App) -> Entity<GiftAppearanceActionsStore> {
        GiftAppearanceActionsStore::shared(cx)
    }

    pub fn want(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(id) = GiftAppearanceServicesStore::normalized(user_id) else {
            return;
        };

        self.wanted.insert(id);
        self.schedule_flush(cx);
    }

    pub fn profile_opened(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(id) = GiftAppearanceServicesStore::normalized(user_id) else {
            return;
        };

        self.forced.insert(id);
        self.schedule_flush(cx);
    }

    pub fn reset(&mut self, _cx: &mut Context<Self>) {
        self.flush_task = None;
        self.wanted.clear();
        self.forced.clear();
    }

    fn schedule_flush(&mut self, cx: &mut Context<Self>) {
        if self.wanted.len() + self.forced.len() >= RiseGiftsRPC::appearance_batch_max() {
            self.flush(cx);
            return;
        }
        if self.flush_task.is_some() {
            return;
        }

        self.flush_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Self::BATCH_WINDOW).await;
            let _ = this.update(cx, |this, cx| this.flush(cx));
        }));
    }

    fn flush(&mut self, cx: &mut Context<Self>) {
        self.flush_task = None;
        let forced = std::mem::take(&mut self.forced);
        let wanted: Vec<String> = std::mem::take(&mut self.wanted)
            .into_iter()
            .filter(|id| !forced.contains(id))
            .collect();

        let actions = Self::actions(cx);
        if !wanted.is_empty() {
            actions.update(cx, |actions, cx| actions.load_appearances_action(wanted, false, cx));
        }
        if !forced.is_empty() {
            let forced: Vec<String> = forced.into_iter().collect();
            actions.update(cx, |actions, cx| actions.load_appearances_action(forced, true, cx));
        }
    }
}
