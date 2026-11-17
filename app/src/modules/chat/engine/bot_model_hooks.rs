use rise_canon::rise_proto;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotModel {
    pub id: String,
    pub owner_user_id: String,
    pub bot_user_id: String,
    pub private_chat_id: String,
    pub tag: String,
    pub name: String,
    pub description: String,
    pub avatar_url: String,
    pub banner_url: String,
    pub is_active: bool,
    pub can_join_groups: bool,
    pub privacy_mode: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub bot_user_chat_id: String,
    pub is_owned_by_requester: bool,
    pub moderation_status: String,
    pub is_system: bool,
}

impl BotModel {
    pub const DEFAULT_MODERATION_STATUS: &'static str = "active";
    const DISABLED_MODERATION_STATUS: &'static str = "disabled";

    pub fn is_moderation_disabled(&self) -> bool {
        self.moderation_status == Self::DISABLED_MODERATION_STATUS
    }
}

pub trait BotWireDTOExt {
    fn to_bot_model(&self) -> BotModel;
}

impl BotWireDTOExt for rise_proto::bot::Bot {
    fn to_bot_model(&self) -> BotModel {
        BotModel {
            id: self.id.clone(),
            owner_user_id: self.owner_user_id.clone(),
            bot_user_id: self.bot_user_id.clone(),
            private_chat_id: self.private_chat_id.clone(),
            tag: self.tag.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            avatar_url: self.avatar_url.clone(),
            banner_url: self.banner_url.clone(),
            is_active: self.is_active,
            can_join_groups: self.can_join_groups,
            privacy_mode: self.privacy_mode,
            created_at: self.created_at,
            updated_at: self.updated_at,
            bot_user_chat_id: self.bot_user_chat_id.clone(),
            is_owned_by_requester: self.is_owned_by_requester,
            moderation_status: self.moderation_status.clone(),
            is_system: self.is_system,
        }
    }
}

pub type BotGatewayDTO = rise_proto::bot::GetBotResponse;
