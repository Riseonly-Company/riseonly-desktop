use gpui::App;
use rise_canon::rise_key;
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use serde_json::{Map, Value};

use super::bot_inline_wire_types::{
    BotInlineIdentityWireDTO, BotInlinePreviewMediaWireDTO, BotInlinePreviewMessageWireDTO, BotInlineResultWireDTO,
    BotMentionCandidateWireDTO,
};
use super::bot_types::BotCommandModel;
use crate::modules::chat::shared::utils::message_preview_hooks::MessagePreviewHooks;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BotMentionCandidateKind {
    User,
    Bot,
}

impl BotMentionCandidateKind {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Bot => "bot",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        match raw_value {
            "user" => Some(Self::User),
            "bot" => Some(Self::Bot),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotMentionCandidateModel {
    pub id: String,
    pub user_id: String,
    pub bot_id: String,
    pub kind: BotMentionCandidateKind,
    pub tag: String,
    pub name: String,
    pub avatar_url: String,
    pub is_premium: bool,
    pub is_online: bool,
    pub last_seen: i64,
    pub inline_enabled: bool,
    pub inline_placeholder: String,
}

impl BotMentionCandidateModel {
    pub fn is_inline_bot(&self) -> bool {
        self.kind == BotMentionCandidateKind::Bot && self.inline_enabled
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotInlineIdentityModel {
    pub id: String,
    pub bot_user_id: String,
    pub tag: String,
    pub name: String,
    pub avatar_url: String,
    pub inline_placeholder: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotInlineCommandsModel {
    pub bot: Option<BotInlineIdentityModel>,
    pub commands: Vec<BotCommandModel>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BotInlinePreviewMediaModel {
    pub url: String,
    pub media_type: String,
    pub thumbnail_url: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<f64>,
    pub mime_type: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BotInlinePreviewMessageModel {
    pub content: String,
    pub original_content: String,
    pub content_type: String,
    pub caption: String,
    pub media_items: Vec<BotInlinePreviewMediaModel>,
    pub entities: String,
    pub reply_markup: String,
    pub inline_bot_id: String,
    pub inline_query_id: String,
    pub inline_result_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BotInlineResultModel {
    pub id: String,
    pub query_id: String,
    pub r#type: String,
    pub title: String,
    pub description: String,
    pub thumbnail_url: String,
    pub media_url: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<f64>,
    pub mime_type: String,
    pub content: String,
    pub caption: String,
    pub send_message: Option<BotInlinePreviewMessageModel>,
}

impl BotInlineResultModel {
    const VISUAL_TYPES: &'static [&'static str] = &["photo", "image", "gif", "video", "animation"];
    const FILE_TYPES: &'static [&'static str] = &["audio", "voice", "document", "file"];
    const TEXT_CONTENT_TYPE: &'static str = "text";
    const PHOTO_TYPE: &'static str = "photo";
    const IMAGE_MEDIA_TYPE: &'static str = "image";

    pub fn normalized_type(&self) -> String {
        trimming_whitespaces_and_newlines(&self.r#type).to_lowercase()
    }

    pub fn is_visual(&self) -> bool {
        Self::VISUAL_TYPES.contains(&self.normalized_type().as_str())
    }

    pub fn is_file(&self) -> bool {
        Self::FILE_TYPES.contains(&self.normalized_type().as_str())
    }

    pub fn preview_url(&self) -> String {
        if !self.thumbnail_url.is_empty() {
            return self.thumbnail_url.clone();
        }
        if !self.media_url.is_empty() {
            return self.media_url.clone();
        }
        let first_media = self.send_message.as_ref().and_then(|message| message.media_items.first());
        first_media
            .and_then(|item| non_empty(&item.thumbnail_url))
            .or_else(|| first_media.map(|item| item.url.as_str()))
            .unwrap_or("")
            .to_owned()
    }

    pub fn effective_query_id(&self, fallback: &str) -> String {
        non_empty(&self.query_id)
            .or_else(|| self.send_message.as_ref().and_then(|message| non_empty(&message.inline_query_id)))
            .unwrap_or(fallback)
            .to_owned()
    }

    pub fn optimistic_message_body(&self, topic_id: Option<&str>) -> Map<String, Value> {
        let preview = self.send_message.as_ref();
        let normalized_type = self.normalized_type();
        let mut body = Map::new();
        body.insert(
            rise_key::bot::CONTENT.to_owned(),
            Value::from(preview.map_or(self.content.as_str(), |preview| preview.content.as_str())),
        );
        body.insert(
            rise_key::bot::ORIGINAL_CONTENT.to_owned(),
            Value::from(preview.map_or(self.content.as_str(), |preview| preview.original_content.as_str())),
        );
        body.insert(
            rise_key::bot::CONTENT_TYPE.to_owned(),
            Value::from(
                preview
                    .and_then(|preview| non_empty(&preview.content_type))
                    .or_else(|| non_empty(&normalized_type))
                    .unwrap_or(Self::TEXT_CONTENT_TYPE),
            ),
        );
        body.insert(
            rise_key::bot::CAPTION.to_owned(),
            Value::from(preview.map_or(self.caption.as_str(), |preview| preview.caption.as_str())),
        );
        if let Some(topic_id) = topic_id
            && !topic_id.is_empty()
        {
            body.insert(rise_key::bot::TOPIC_ID.to_owned(), Value::from(topic_id));
        }
        if let Some(entities) = preview.map(|preview| preview.entities.as_str())
            && !entities.is_empty()
        {
            body.insert(rise_key::bot::ENTITIES.to_owned(), Value::from(entities));
        }
        if let Some(reply_markup) = preview.map(|preview| preview.reply_markup.as_str())
            && !reply_markup.is_empty()
        {
            body.insert(rise_key::bot::REPLY_MARKUP.to_owned(), Value::from(reply_markup));
        }
        if let Some(inline_bot_id) = preview.map(|preview| preview.inline_bot_id.as_str())
            && !inline_bot_id.is_empty()
        {
            body.insert(rise_key::bot::VIA_BOT.to_owned(), Value::from(inline_bot_id));
            body.insert(rise_key::bot::IS_BOT_MESSAGE.to_owned(), Value::from(true));
        }
        let media: &[BotInlinePreviewMediaModel] =
            preview.map(|preview| preview.media_items.as_slice()).unwrap_or_default();
        if !media.is_empty() {
            let items: Vec<Value> = media
                .iter()
                .map(|item| {
                    let mut value = Map::new();
                    value.insert(
                        rise_key::bot::MEDIA_TYPE.to_owned(),
                        Value::from(non_empty(&item.media_type).unwrap_or(normalized_type.as_str())),
                    );
                    value.insert(rise_key::bot::FILE_URL.to_owned(), Value::from(item.url.as_str()));
                    value.insert(rise_key::bot::THUMBNAIL_URL.to_owned(), Value::from(item.thumbnail_url.as_str()));
                    value.insert(rise_key::bot::MIME_TYPE.to_owned(), Value::from(item.mime_type.as_str()));
                    if let Some(width) = item.width {
                        value.insert(rise_key::bot::WIDTH.to_owned(), Value::from(width));
                    }
                    if let Some(height) = item.height {
                        value.insert(rise_key::bot::HEIGHT.to_owned(), Value::from(height));
                    }
                    if let Some(duration) = item.duration {
                        value.insert(rise_key::bot::DURATION.to_owned(), Value::from(duration));
                    }
                    Value::Object(value)
                })
                .collect();
            body.insert(rise_key::bot::MEDIA_ITEMS.to_owned(), Value::Array(items));
        } else if self.is_visual() && !self.media_url.is_empty() {
            let mut value = Map::new();
            value.insert(
                rise_key::bot::MEDIA_TYPE.to_owned(),
                Value::from(if normalized_type == Self::PHOTO_TYPE {
                    Self::IMAGE_MEDIA_TYPE
                } else {
                    normalized_type.as_str()
                }),
            );
            value.insert(rise_key::bot::FILE_URL.to_owned(), Value::from(self.media_url.as_str()));
            value.insert(rise_key::bot::THUMBNAIL_URL.to_owned(), Value::from(self.thumbnail_url.as_str()));
            value.insert(rise_key::bot::MIME_TYPE.to_owned(), Value::from(self.mime_type.as_str()));
            body.insert(rise_key::bot::MEDIA_ITEMS.to_owned(), Value::Array(vec![Value::Object(value)]));
        }
        body
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BotInlineResultsPage {
    pub query_id: String,
    pub results: Vec<BotInlineResultModel>,
    pub next_offset: Option<String>,
    pub has_more: bool,
    pub cache_time: i64,
}

pub trait BotMentionCandidateWireDTOExt {
    fn to_model(&self, cx: &mut App) -> BotMentionCandidateModel;
}

impl BotMentionCandidateWireDTOExt for BotMentionCandidateWireDTO {
    fn to_model(&self, cx: &mut App) -> BotMentionCandidateModel {
        let resolved_kind = BotMentionCandidateKind::from_raw_value(&self.kind.to_lowercase()).unwrap_or(if self.is_bot {
            BotMentionCandidateKind::Bot
        } else {
            BotMentionCandidateKind::User
        });
        let name = if resolved_kind == BotMentionCandidateKind::User {
            MessagePreviewHooks::contact_name(Some(self.user_id.as_str()), &self.name, cx)
        } else {
            self.name.clone()
        };
        BotMentionCandidateModel {
            id: self.id.clone(),
            user_id: self.user_id.clone(),
            bot_id: self.bot_id.clone().unwrap_or_default(),
            kind: resolved_kind,
            tag: trimming_characters(&self.tag, |character| character == '@').to_owned(),
            name,
            avatar_url: self.avatar_url.clone(),
            is_premium: self.is_premium,
            is_online: self.is_online,
            last_seen: self.last_seen,
            inline_enabled: self.inline_enabled,
            inline_placeholder: self.inline_placeholder.clone(),
        }
    }
}

pub trait BotInlineIdentityWireDTOExt {
    fn to_model(&self) -> BotInlineIdentityModel;
}

impl BotInlineIdentityWireDTOExt for BotInlineIdentityWireDTO {
    fn to_model(&self) -> BotInlineIdentityModel {
        BotInlineIdentityModel {
            id: self.id.clone(),
            bot_user_id: self.bot_user_id.clone(),
            tag: trimming_characters(&self.tag, |character| character == '@').to_owned(),
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
            inline_placeholder: self.inline_placeholder.clone(),
        }
    }
}

pub trait BotInlinePreviewMediaWireDTOExt {
    fn to_model(&self) -> BotInlinePreviewMediaModel;
}

impl BotInlinePreviewMediaWireDTOExt for BotInlinePreviewMediaWireDTO {
    fn to_model(&self) -> BotInlinePreviewMediaModel {
        BotInlinePreviewMediaModel {
            url: self.url.clone(),
            media_type: self.media_type.clone(),
            thumbnail_url: self.thumbnail_url.clone().unwrap_or_default(),
            width: self.width,
            height: self.height,
            duration: self.duration.map(|duration| duration as f64),
            mime_type: self.mime_type.clone().unwrap_or_default(),
        }
    }
}

pub trait BotInlinePreviewMessageWireDTOExt {
    fn to_model(&self) -> BotInlinePreviewMessageModel;
}

impl BotInlinePreviewMessageWireDTOExt for BotInlinePreviewMessageWireDTO {
    fn to_model(&self) -> BotInlinePreviewMessageModel {
        BotInlinePreviewMessageModel {
            content: self.content.clone(),
            original_content: self.original_content.clone(),
            content_type: self.content_type.clone(),
            caption: self.caption.clone().unwrap_or_default(),
            media_items: self.media_items.iter().map(BotInlinePreviewMediaWireDTOExt::to_model).collect(),
            entities: self.entities.clone().unwrap_or_default(),
            reply_markup: self.reply_markup.clone().unwrap_or_default(),
            inline_bot_id: self.inline_bot_id.clone(),
            inline_query_id: self.inline_query_id.clone(),
            inline_result_id: self.inline_result_id.clone(),
        }
    }
}

pub trait BotInlineResultWireDTOExt {
    fn to_model(&self, query_id: &str) -> BotInlineResultModel;
}

impl BotInlineResultWireDTOExt for BotInlineResultWireDTO {
    fn to_model(&self, query_id: &str) -> BotInlineResultModel {
        BotInlineResultModel {
            id: self.id.clone(),
            query_id: query_id.to_owned(),
            r#type: self.r#type.clone(),
            title: self.title.clone().unwrap_or_default(),
            description: self.description.clone().unwrap_or_default(),
            thumbnail_url: self.thumbnail_url.clone().unwrap_or_default(),
            media_url: self.media_url.clone().unwrap_or_default(),
            width: self.width,
            height: self.height,
            duration: self.duration.map(|duration| duration as f64),
            mime_type: self.mime_type.clone().unwrap_or_default(),
            content: self.content.clone().unwrap_or_default(),
            caption: self.caption.clone().unwrap_or_default(),
            send_message: self.send_message.as_ref().map(BotInlinePreviewMessageWireDTOExt::to_model),
        }
    }
}

fn non_empty(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}
