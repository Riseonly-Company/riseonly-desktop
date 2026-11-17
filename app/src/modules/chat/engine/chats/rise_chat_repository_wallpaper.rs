use std::collections::BTreeMap;
use std::sync::Weak;

use rise_canon::rise_key;
use rise_engine::RiseJSONValue;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepository;
use crate::modules::chat::engine::chats::rise_chat_repository_models::{
    RiseChatOptimisticOperation, RiseChatOptimisticToken, RiseChatRepositoryError,
};
use crate::modules::chat::engine::core::rise_conversation_rpc::RiseConversationEndpoints;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatWallpaperCatalogEntry, ChatWallpaperPlacement,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    ChatWallpaperCatalogEntryWireDTOExt, ChatWallpaperCatalogGatewayDTO, ChatWallpaperWireDTO,
    ChatWallpaperWireDTOExt, SetChatWallpaperGatewayDTO,
};

struct RiseChatWallpaperRollbackGuard {
    repository: Weak<RiseChatRepository>,
    token: Option<RiseChatOptimisticToken>,
}

impl Drop for RiseChatWallpaperRollbackGuard {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        let Some(repository) = self.repository.upgrade() else {
            return;
        };
        RiseAccountRuntime::current().spawn(async move {
            let _ = repository.rollback_optimistic(&token).await;
        });
    }
}

impl RiseChatRepository {
    pub async fn set_wallpaper(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        wallpaper: Option<&ChatWallpaperPlacement>,
        entry_id: Option<&str>,
        dim_percent: Option<i64>,
        blur_percent: Option<i64>,
    ) -> anyhow::Result<Option<ChatWallpaperPlacement>> {
        let topic = topic_id.unwrap_or("");
        let wallpaper_id = if entry_id.is_none() { wallpaper.map_or("", |value| value.id.as_str()) } else { "" };
        let body = rise_canon::rise_request::chat::SetChatWallpaper {
            blur_percent,
            chat_id: Some(chat_id.to_owned()),
            dim_percent,
            entry_id: Some(entry_id.unwrap_or("").to_owned()),
            topic_id: Some(topic.to_owned()),
            wallpaper_id: Some(wallpaper_id.to_owned()),
        };
        if !topic.is_empty() {
            let response: SetChatWallpaperGatewayDTO =
                self.rpc.call(RiseConversationEndpoints::set_chat_wallpaper(), body).await?;
            return Ok(response.wallpaper.as_ref().and_then(ChatWallpaperWireDTOExt::to_placement));
        }

        let optimistic = Self::wallpaper_patch(wallpaper)?;
        let token = self.apply_optimistic(chat_id, RiseChatOptimisticOperation::Patch(optimistic)).await?;
        let mut guard =
            RiseChatWallpaperRollbackGuard { repository: self.this.clone(), token: Some(token.clone()) };
        let settled = async {
            let response: SetChatWallpaperGatewayDTO =
                self.rpc.call(RiseConversationEndpoints::set_chat_wallpaper(), body).await?;
            self.confirm_optimistic(&token).await?;
            let confirmed = response.wallpaper.as_ref().and_then(ChatWallpaperWireDTOExt::to_placement);
            if confirmed.as_ref() != wallpaper {
                let settle = self
                    .apply_optimistic(
                        chat_id,
                        RiseChatOptimisticOperation::Patch(Self::wallpaper_patch(confirmed.as_ref())?),
                    )
                    .await?;
                self.confirm_optimistic(&settle).await?;
            }
            Ok::<_, anyhow::Error>(confirmed)
        }
        .await;
        guard.token = None;
        match settled {
            Ok(confirmed) => Ok(confirmed),
            Err(error) => {
                let _ = self.rollback_optimistic(&token).await;
                Err(error)
            }
        }
    }

    /// wallpaper catalog of one chat or topic, newest first
    pub async fn wallpaper_catalog(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
    ) -> anyhow::Result<Vec<ChatWallpaperCatalogEntry>> {
        let response: ChatWallpaperCatalogGatewayDTO = self
            .rpc
            .call(
                RiseConversationEndpoints::get_chat_wallpapers(),
                rise_canon::rise_request::chat::GetChatWallpapers {
                    chat_id: Some(chat_id.to_owned()),
                    topic_id: Some(topic_id.unwrap_or("").to_owned()),
                },
            )
            .await?;
        Self::wallpaper_catalog_entries(response)
    }

    pub async fn add_wallpaper_catalog_entry(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        image_url: &str,
        look: ChatWallpaperLook,
    ) -> anyhow::Result<Vec<ChatWallpaperCatalogEntry>> {
        let response: ChatWallpaperCatalogGatewayDTO = self
            .rpc
            .call(
                RiseConversationEndpoints::add_chat_wallpaper(),
                rise_canon::rise_request::chat::AddChatWallpaper {
                    blur_percent: Some(look.blur_percent),
                    chat_id: Some(chat_id.to_owned()),
                    dim_percent: Some(look.dim_percent),
                    image_url: Some(image_url.to_owned()),
                    preview_url: Some(image_url.to_owned()),
                    topic_id: Some(topic_id.unwrap_or("").to_owned()),
                },
            )
            .await?;
        Self::wallpaper_catalog_entries(response)
    }

    pub async fn delete_wallpaper_catalog_entry(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        entry_id: &str,
    ) -> anyhow::Result<Vec<ChatWallpaperCatalogEntry>> {
        let response: ChatWallpaperCatalogGatewayDTO = self
            .rpc
            .call(
                RiseConversationEndpoints::delete_chat_wallpaper(),
                rise_canon::rise_request::chat::DeleteChatWallpaper {
                    chat_id: Some(chat_id.to_owned()),
                    entry_id: Some(entry_id.to_owned()),
                    topic_id: Some(topic_id.unwrap_or("").to_owned()),
                },
            )
            .await?;
        Self::wallpaper_catalog_entries(response)
    }

    fn wallpaper_catalog_entries(
        response: ChatWallpaperCatalogGatewayDTO,
    ) -> anyhow::Result<Vec<ChatWallpaperCatalogEntry>> {
        if !response.success {
            return Err(RiseChatRepositoryError::Rejected(response.error).into());
        }
        Ok(response.entries.iter().filter_map(ChatWallpaperCatalogEntryWireDTOExt::to_entry).collect())
    }

    /// wire shape of the wallpaper key
    /// removal is an empty object and never null, null reads as untouched
    fn wallpaper_patch(
        wallpaper: Option<&ChatWallpaperPlacement>,
    ) -> anyhow::Result<BTreeMap<String, RiseJSONValue>> {
        let Some(wallpaper) = wallpaper else {
            return Ok(BTreeMap::from([(
                rise_key::chat::WALLPAPER.to_owned(),
                RiseJSONValue::Object(BTreeMap::new()),
            )]));
        };
        match RiseJSONValue::encode(&ChatWallpaperWireDTO::from_placement(wallpaper))? {
            RiseJSONValue::Object(object) => {
                Ok(BTreeMap::from([(rise_key::chat::WALLPAPER.to_owned(), RiseJSONValue::Object(object))]))
            }
            _ => Err(RiseChatRepositoryError::CorruptedChat(wallpaper.id.clone()).into()),
        }
    }
}
