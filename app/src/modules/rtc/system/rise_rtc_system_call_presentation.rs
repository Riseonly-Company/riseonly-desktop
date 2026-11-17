use std::collections::{HashSet, VecDeque};

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::rtc::system::rise_rtc_system_models::{
    RiseRTCSystemCallDirection, RiseRTCSystemCallPhase, RiseRTCSystemCallTerminalOutcome,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCSystemCallPresentation {
    pub local_uuid: Uuid,
    pub server_call_id: Option<String>,
    pub direction: RiseRTCSystemCallDirection,
    pub phase: RiseRTCSystemCallPhase,
    pub remote_user_id: Option<String>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub has_video: bool,
    pub requested_at: Date,
    pub connected_at: Option<Date>,
    pub terminal_at: Option<Date>,
    pub terminal_outcome: Option<RiseRTCSystemCallTerminalOutcome>,
    pub requires_in_app_answer_ui: bool,
    pub awaiting_connection: bool,
}

impl RiseRTCSystemCallPresentation {
    pub fn id(&self) -> Uuid {
        self.local_uuid
    }

    pub fn is_terminal(&self) -> bool {
        self.phase == RiseRTCSystemCallPhase::Ended
    }
}

pub struct RiseRTCSystemCallPresentationStore {
    calls: Vec<RiseRTCSystemCallPresentation>,
    dismissed_local_uuids: HashSet<Uuid>,
    dismissed_server_call_ids: HashSet<String>,
    dismissed_local_order: VecDeque<Uuid>,
    dismissed_server_order: VecDeque<String>,
}

impl RiseRTCSystemCallPresentationStore {
    const CALL_LIMIT: usize = 4;
    const DISMISSAL_LIMIT: usize = 32;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            calls: Vec::new(),
            dismissed_local_uuids: HashSet::new(),
            dismissed_server_call_ids: HashSet::new(),
            dismissed_local_order: VecDeque::new(),
            dismissed_server_order: VecDeque::new(),
        }
    }

    pub fn calls(&self) -> &[RiseRTCSystemCallPresentation] {
        &self.calls
    }

    pub fn dismissed_local_uuids(&self) -> &HashSet<Uuid> {
        &self.dismissed_local_uuids
    }

    pub fn dismissed_server_call_ids(&self) -> &HashSet<String> {
        &self.dismissed_server_call_ids
    }

    pub fn publish(&mut self, calls: Vec<RiseRTCSystemCallPresentation>, cx: &mut Context<Self>) {
        let dismissed: Vec<(Uuid, Option<String>)> = calls
            .iter()
            .filter(|call| self.dismissed_local_uuids.contains(&call.local_uuid))
            .map(|call| (call.local_uuid, call.server_call_id.clone()))
            .collect();
        for (local_uuid, server_call_id) in dismissed {
            self.dismiss_presentation(local_uuid, server_call_id.as_deref(), cx);
        }
        let mut next = calls;
        next.sort_by(|lhs, rhs| {
            if lhs.is_terminal() != rhs.is_terminal() {
                return lhs.is_terminal().cmp(&rhs.is_terminal());
            }
            rhs.requested_at.ms().cmp(&lhs.requested_at.ms())
        });
        next.truncate(Self::CALL_LIMIT);
        if next == self.calls {
            return;
        }
        self.calls = next;
        cx.notify();
    }

    pub fn dismiss_presentation(&mut self, local_uuid: Uuid, server_call_id: Option<&str>, cx: &mut Context<Self>) {
        if self.dismissed_local_uuids.insert(local_uuid) {
            self.dismissed_local_order.push_back(local_uuid);
            while self.dismissed_local_order.len() > Self::DISMISSAL_LIMIT {
                if let Some(dropped) = self.dismissed_local_order.pop_front() {
                    self.dismissed_local_uuids.remove(&dropped);
                }
            }
            cx.notify();
        }
        let Some(server_call_id) = server_call_id.map(trimming_whitespaces_and_newlines) else {
            return;
        };
        if server_call_id.is_empty() || !self.dismissed_server_call_ids.insert(server_call_id.to_owned()) {
            return;
        }
        self.dismissed_server_order.push_back(server_call_id.to_owned());
        while self.dismissed_server_order.len() > Self::DISMISSAL_LIMIT {
            if let Some(dropped) = self.dismissed_server_order.pop_front() {
                self.dismissed_server_call_ids.remove(&dropped);
            }
        }
        cx.notify();
    }

    pub fn reset_dismissed_presentations(&mut self, cx: &mut Context<Self>) {
        self.dismissed_local_uuids = HashSet::new();
        self.dismissed_server_call_ids = HashSet::new();
        self.dismissed_local_order = VecDeque::new();
        self.dismissed_server_order = VecDeque::new();
        cx.notify();
    }
}
