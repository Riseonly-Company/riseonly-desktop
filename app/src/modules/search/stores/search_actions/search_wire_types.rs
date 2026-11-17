use rise_canon::rise_proto_enum::tag::EntityType;
use rise_canon::{rise_proto, rise_reply, rise_view};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, AuthorInfoMore};
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;

pub type SearchTagUserMoreDTO = rise_proto::tag::SearchUserMore;
pub type SearchTagUserDTO = rise_proto::tag::SearchUserResult;
pub type SearchResultItemDTO = rise_proto::tag::SearchResultItem;
pub type GlobalSearchByTagGatewayDTO = rise_view::tag::SearchPage;
pub type SearchUserBasicDTO = rise_proto::user::UserBasicInfoDto;
pub type SearchUsersListDTO = rise_proto::user::SearchUserResponse;

pub type GlobalSearchGatewayDTO = rise_reply::search::MixedResults;

/// entity type goes over the wire as a number, user 0 and channel 1
fn entity_kind(number: i64) -> String {
    if EntityType::from_raw_value(number) == Some(EntityType::Channel) {
        EntityType::Channel.proto_name().to_owned()
    } else {
        EntityType::User.proto_name().to_owned()
    }
}

pub trait SearchResultItemDTOExt {
    fn as_entity_row(&self) -> Option<SearchEntityRow>;
}

impl SearchResultItemDTOExt for SearchResultItemDTO {
    fn as_entity_row(&self) -> Option<SearchEntityRow> {
        let eid = trimming_whitespaces_and_newlines(&self.entity_id);
        if eid.is_empty() {
            return None;
        }
        if let Some(user) = &self.user {
            let tag = &user.tag;
            return Some(SearchEntityRow {
                id: eid.to_owned(),
                entity_type: entity_kind(self.entity_type),
                display_name: user.name.clone(),
                tag_line: if tag.is_empty() { String::new() } else { format!("@{tag}") },
                logo_url: if user.logo.is_empty() {
                    user.more.as_ref().map(|more| more.logo.clone()).unwrap_or_default()
                } else {
                    user.logo.clone()
                },
                user_id: Some(user.id.clone()),
                user_tag: if tag.is_empty() { None } else { Some(tag.clone()) },
                chat_id: user.chat_id.clone().or_else(|| user.user_chat_id.clone()),
                is_premium: user.is_premium,
                is_official: user.is_official,
                premium_badge_emoji: user.premium_badge_emoji.clone(),
                name_color: user.name_color.clone(),
                is_bot: user.is_bot,
                bot_is_active: user.bot_is_active,
                member_count: None,
                is_public: None,
            });
        }
        if let Some(chat) = &self.chat {
            let tag = chat.tag.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default();
            return Some(SearchEntityRow {
                id: eid.to_owned(),
                entity_type: if chat.chat_type.is_empty() {
                    entity_kind(self.entity_type)
                } else {
                    chat.chat_type.clone()
                },
                display_name: chat.title.clone(),
                tag_line: if tag.is_empty() { String::new() } else { format!("@{tag}") },
                logo_url: chat.logo_url.clone(),
                user_id: None,
                user_tag: if tag.is_empty() { None } else { Some(tag.to_owned()) },
                chat_id: Some(chat.id.clone()),
                is_premium: chat.is_verified,
                is_official: false,
                premium_badge_emoji: None,
                name_color: None,
                is_bot: false,
                bot_is_active: false,
                member_count: Some(chat.member_count),
                is_public: Some(chat.is_public),
            });
        }
        None
    }
}

pub trait GlobalSearchByTagGatewayDTOExt {
    fn entity_rows(&self) -> Vec<SearchEntityRow>;
}

impl GlobalSearchByTagGatewayDTOExt for GlobalSearchByTagGatewayDTO {
    fn entity_rows(&self) -> Vec<SearchEntityRow> {
        self.results.iter().filter_map(SearchResultItemDTOExt::as_entity_row).collect()
    }
}

pub trait GlobalSearchGatewayDTOExt {
    fn chat_infos(&self) -> Vec<ChatInfo>;
    fn global_entity_rows(&self) -> Vec<SearchEntityRow>;
}

impl GlobalSearchGatewayDTOExt for GlobalSearchGatewayDTO {
    fn chat_infos(&self) -> Vec<ChatInfo> {
        self.chats
            .iter()
            .map(|chat| RiseStoredChat::from_canon(chat).to_chat_info())
            .collect()
    }

    fn global_entity_rows(&self) -> Vec<SearchEntityRow> {
        self.global_results
            .iter()
            .filter_map(SearchResultItemDTOExt::as_entity_row)
            .collect()
    }
}

pub trait SearchUserBasicDTOExt {
    fn as_author_info(&self) -> AuthorInfo;
}

impl SearchUserBasicDTOExt for SearchUserBasicDTO {
    fn as_author_info(&self) -> AuthorInfo {
        AuthorInfo {
            id: Some(self.id.clone()),
            name: self.name.clone(),
            tag: self.tag.clone(),
            more: AuthorInfoMore {
                p_lang: Vec::new(),
                who: String::new(),
                role: self.role.clone(),
                logo: self.more.as_ref().map(|more| more.logo.clone()).unwrap_or_default(),
                is_premium: self.is_premium,
                is_official: self.is_official,
                premium_badge_emoji: self.premium_badge_emoji.clone(),
                name_color: self.name_color.clone(),
                ..AuthorInfoMore::default()
            },
            ..AuthorInfo::default()
        }
    }
}

pub trait SearchUsersListDTOExt {
    fn authors(&self) -> Vec<AuthorInfo>;
}

impl SearchUsersListDTOExt for SearchUsersListDTO {
    fn authors(&self) -> Vec<AuthorInfo> {
        self.list
            .iter()
            .filter(|dto| !dto.id.is_empty())
            .map(SearchUserBasicDTOExt::as_author_info)
            .collect()
    }
}
