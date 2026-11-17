use rise_core::foundation::foundation_date::TimeInterval;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresenceState {
    pub is_online: bool,
    pub last_seen: Option<i64>,
    pub in_chat_id: Option<String>,
    pub updated_at: TimeInterval,
    pub status_known: bool,
}
