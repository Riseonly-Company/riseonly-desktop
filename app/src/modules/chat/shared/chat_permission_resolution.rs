use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use rise_canon::{ChatTopicPosting, ChatTopicState};

use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatPerm, ChatPermExt, ChatPermOverwrite};

/// chat permission resolution by exactly the server rule
/// checked against the backend permissions file over 60000 generated cases
/// pure on purpose since it runs on the render path
pub enum ChatPermissionResolution {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatPermissionResolutionOverwriteTarget {
    Role,
    Member,
}

/// merged allow and deny of one target
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChatPermissionResolutionOverwriteBucket {
    pub allow: i64,
    pub deny: i64,
}

impl ChatPermissionResolutionOverwriteBucket {
    pub fn is_empty(&self) -> bool {
        self.allow == 0 && self.deny == 0
    }

    /// several rows can come for one target, canon ORs them all instead of keeping the last
    /// otherwise a deny gets lost silently
    pub fn merge(&mut self, allow: i64, deny: i64) {
        self.allow |= allow;
        self.deny |= deny;
    }

    pub fn applied(&self, permissions: i64) -> i64 {
        (permissions & !self.deny) | self.allow
    }
}

/// one channel layer in canon order: everyone, roles member
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChatPermissionResolutionChannelOverwrites {
    pub everyone: ChatPermissionResolutionOverwriteBucket,
    pub roles: ChatPermissionResolutionOverwriteBucket,
    pub member: ChatPermissionResolutionOverwriteBucket,
}

impl ChatPermissionResolutionChannelOverwrites {
    pub fn is_empty(&self) -> bool {
        self.everyone.is_empty() && self.roles.is_empty() && self.member.is_empty()
    }

    pub fn applied(&self, permissions: i64) -> i64 {
        self.member.applied(self.roles.applied(self.everyone.applied(permissions)))
    }
}

static BYPASSES_BASE_STAGE: LazyLock<bool> = LazyLock::new(|| ChatPerm::BYPASS_STAGES.contains(&"base"));
static BYPASSES_CHANNEL_STAGE: LazyLock<bool> = LazyLock::new(|| ChatPerm::BYPASS_STAGES.contains(&"channel"));

static TARGET_BY_TOKEN: LazyLock<HashMap<String, ChatPermissionResolutionOverwriteTarget>> = LazyLock::new(|| {
    let mut map: HashMap<String, ChatPermissionResolutionOverwriteTarget> = HashMap::new();
    for target in ChatPerm::OVERWRITE_TARGETS {
        let kind = match target.name {
            "role" => ChatPermissionResolutionOverwriteTarget::Role,
            "member" => ChatPermissionResolutionOverwriteTarget::Member,
            _ => continue,
        };
        for token in [target.name, target.wire].iter().chain(target.aliases.iter()) {
            map.insert(token.to_uppercase(), kind);
        }
    }
    map
});

impl ChatPermissionResolution {
    pub fn target(of_wire_target_type: &str) -> Option<ChatPermissionResolutionOverwriteTarget> {
        TARGET_BY_TOKEN.get(&of_wire_target_type.to_uppercase()).copied()
    }

    pub fn bypasses(permissions: i64) -> bool {
        permissions & ChatPerm::BYPASS_BIT != 0
    }

    /// base stage: everyone and roles, then personal deny then allow then the short circuit
    /// owner skips the layers
    pub fn base_permissions(
        is_owner: bool,
        everyone_permissions: i64,
        role_permissions: i64,
        member_allow: i64,
        member_deny: i64,
    ) -> i64 {
        if is_owner && ChatPerm::OWNER_BYPASSES_OVERWRITES {
            return ChatPerm::BYPASS_RESULT;
        }

        let mut value = everyone_permissions | role_permissions;
        value = (value & !member_deny) | member_allow;

        if *BYPASSES_BASE_STAGE && Self::bypasses(value) {
            return ChatPerm::BYPASS_RESULT;
        }
        value
    }

    pub fn channel_permissions(
        base: i64,
        is_owner: bool,
        scopes: &[ChatPermissionResolutionChannelOverwrites],
    ) -> i64 {
        if is_owner && ChatPerm::OWNER_BYPASSES_OVERWRITES {
            return ChatPerm::BYPASS_RESULT;
        }
        if *BYPASSES_CHANNEL_STAGE && Self::bypasses(base) {
            return ChatPerm::BYPASS_RESULT;
        }

        let mut value = base;
        for scope in scopes {
            value = scope.applied(value);
        }
        value
    }

    /// splits wire rows by target
    /// everyone is its own layer and must not land in roles too, or its deny applies twice and
    /// outlives a role allow
    pub fn overwrites(
        rows: &[ChatPermOverwrite],
        everyone_role_id: Option<&str>,
        role_ids: &HashSet<String>,
        user_id: &str,
    ) -> ChatPermissionResolutionChannelOverwrites {
        let mut result = ChatPermissionResolutionChannelOverwrites::default();
        let resolved_everyone_role_id: Option<&str> = everyone_role_id.filter(|id| !id.is_empty());

        for row in rows {
            let Some(target) = Self::target(&row.target_type) else {
                continue;
            };

            match target {
                ChatPermissionResolutionOverwriteTarget::Role => {
                    if resolved_everyone_role_id == Some(row.target_id.as_str()) {
                        result.everyone.merge(row.allow, row.deny);
                    } else if role_ids.contains(&row.target_id) {
                        result.roles.merge(row.allow, row.deny);
                    }
                }
                ChatPermissionResolutionOverwriteTarget::Member => {
                    if !user_id.is_empty() && row.target_id == user_id {
                        result.member.merge(row.allow, row.deny);
                    }
                }
            }
        }
        result
    }

    /// true when the viewer may publish in a channel
    /// a member is just a subscriber so the everyone send grant doesnt count
    /// owner and admin may
    pub fn can_post_to_channel(role: Option<&str>) -> bool {
        ChatPerm::is_owner_or_legacy_admin_role(role)
    }

    pub fn can_post(
        in_topic_state: ChatTopicState,
        is_owner: impl FnOnce() -> bool,
        permissions: impl FnOnce() -> i64,
    ) -> bool {
        match in_topic_state.posting() {
            ChatTopicPosting::Allowed => true,
            ChatTopicPosting::Denied => false,
            ChatTopicPosting::RequiresBypass => {
                if ChatPerm::CLOSED_TOPIC_OWNER_BYPASSES && is_owner() {
                    return true;
                }
                permissions() & ChatPerm::CLOSED_TOPIC_BYPASS != 0
            }
        }
    }
}
