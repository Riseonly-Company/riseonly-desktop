use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_platform::haptics::{NotificationKind, notification, prepare};

use crate::modules::user::engine::rise_user_achievements_realtime::RiseAchievementGrant;
use crate::modules::user::stores::user_achievements::user_achievements_interactions::user_achievements_interactions::UserAchievementsInteractionsStore;

/// shows earned achievements one at a time in arrival order
/// a grant can land while another celebration is up so this holds a payload and queue
pub struct AchievementEarnedStore {
    current: Option<RiseAchievementGrant>,
    queue: VecDeque<RiseAchievementGrant>,
    celebrated_grants: HashSet<String>,
    settle_task: Option<Task<()>>,
    is_settling: bool,
}

impl AchievementEarnedStore {
    const MAXIMUM_QUEUED: usize = 8;
    const MAXIMUM_REMEMBERED: usize = 256;
    const SETTLE_MILLISECONDS: u64 = 420;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            current: None,
            queue: VecDeque::new(),
            celebrated_grants: HashSet::new(),
            settle_task: None,
            is_settling: false,
        }
    }

    pub fn current(&self) -> Option<&RiseAchievementGrant> {
        self.current.as_ref()
    }

    pub fn is_visible(&self) -> bool {
        self.current.is_some()
    }

    fn grant_key(grant: &RiseAchievementGrant) -> String {
        [grant.achievement.id.to_string(), grant.achievement.granted_at_ms.to_string()].join(":")
    }

    /// called from the realtime reducer only, older grants stay a quiet mark
    /// total and idempotent per grant, never fails or suspends
    /// regranted award celebrates again
    pub fn enqueue(&mut self, grant: RiseAchievementGrant, cx: &mut Context<Self>) {
        if grant.achievement.id <= 0 {
            return;
        }

        let key = Self::grant_key(&grant);
        if self.celebrated_grants.contains(&key) {
            return;
        }

        if self.celebrated_grants.len() >= Self::MAXIMUM_REMEMBERED {
            self.celebrated_grants.clear();
        }
        self.celebrated_grants.insert(key);

        if self.queue.len() >= Self::MAXIMUM_QUEUED {
            return;
        }
        self.queue.push_back(grant);

        self.present_next_if_idle(cx);
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        if self.current.is_none() {
            return;
        }
        self.current = None;
        cx.notify();
        if self.queue.is_empty() {
            return;
        }

        self.is_settling = true;
        self.settle_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(Self::SETTLE_MILLISECONDS))
                .await;
            let _ = this.update(cx, |store, cx| {
                store.is_settling = false;
                store.settle_task = None;
                store.present_next_if_idle(cx);
            });
        }));
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.settle_task = None;
        self.is_settling = false;
        self.current = None;
        self.queue.clear();
        self.celebrated_grants.clear();
        cx.notify();
    }

    fn present_next_if_idle(&mut self, cx: &mut Context<Self>) {
        if self.current.is_some() || self.is_settling {
            return;
        }
        let Some(next) = self.queue.pop_front() else {
            return;
        };
        let next_id = next.id();
        self.current = Some(next);
        cx.notify();

        prepare();
        notification(NotificationKind::Success);

        UserAchievementsInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.mark_seen(&[next_id], cx));
    }
}
