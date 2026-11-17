use crate::modules::stickers::engine::rise_pack_collaboration_repository::RisePackInviteLink;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackInviteLinkDraft {
    pub name: Option<String>,
    pub expires_at_ms: Option<i64>,
    pub usage_limit: Option<i64>,
    pub creates_join_request: bool,
}

impl PackInviteLinkDraft {
    pub fn new(
        name: Option<String>,
        expires_at_ms: Option<i64>,
        usage_limit: Option<i64>,
        creates_join_request: bool,
    ) -> Self {
        Self { name, expires_at_ms, usage_limit, creates_join_request }
    }

    pub fn from_link(link: &RisePackInviteLink) -> Self {
        Self::new(link.name.clone(), link.expires_at_ms, link.usage_limit, link.creates_join_request)
    }
}
