use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;

use gpui::{App, AsyncApp};
use rise_core::foundation::foundation_url::Url;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use super::chat_actions::ChatActionsStore;
use super::chat_types::{
    ChatWallpaperCatalogEntry, ChatWallpaperCatalogState, ChatWallpaperPlacement,
};
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::{RiseChatWallpaperDraft, RiseChatWallpaperUpload};

pub type ChatWallpaperResult = Box<dyn FnOnce(bool, Option<String>, &mut App)>;
pub type ChatWallpaperAddResult = Box<dyn FnOnce(bool, Option<ChatWallpaperCatalogEntry>, Option<String>, &mut App)>;

struct StagedWallpaperFile(Url);

impl Drop for StagedWallpaperFile {
    fn drop(&mut self) {
        RiseChatWallpaperUpload::discard_staged_file(&self.0);
    }
}

async fn on_account_runtime<Output: Send + 'static>(
    work: impl Future<Output = anyhow::Result<Output>> + Send + 'static,
) -> anyhow::Result<Output> {
    RiseAccountRuntime::current().spawn(work).await.map_err(|_| CancellationError)?
}

fn active_domain(cx: &mut AsyncApp) -> Option<Arc<RiseConversationDomain>> {
    cx.update(|cx| RiseConversationDomainRegistry::shared(cx).read(cx).active())
}

fn mutate_catalog(
    chat_id: &str,
    topic_id: Option<&str>,
    change: impl FnOnce(&mut ChatWallpaperCatalogState),
    cx: &mut App,
) {
    ChatServicesStore::shared(cx).update(cx, |services, cx| {
        services.mutate_wallpaper_catalog(chat_id, topic_id, change, cx);
    });
}

fn log_network(line: String, cx: &mut App) {
    Sai::log("Chat", move || line, SaiLogKind::Network, cx);
}

fn non_empty(message: String) -> Option<String> {
    (!message.is_empty()).then_some(message)
}

fn apply_wallpaper(chat_id: &str, topic: Option<&str>, wallpaper: Option<ChatWallpaperPlacement>, cx: &mut App) {
    ChatServicesStore::shared(cx).update(cx, |services, cx| match topic {
        Some(topic) => services.update_topic_wallpaper_local(chat_id, topic, wallpaper, cx),
        None => services.update_chat_wallpaper_local(chat_id, wallpaper, cx),
    });
}

impl ChatActionsStore {
    #[allow(clippy::too_many_arguments)]
    pub fn set_chat_wallpaper_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        wallpaper: Option<ChatWallpaperPlacement>,
        entry_id: Option<&str>,
        dim_percent: Option<i64>,
        blur_percent: Option<i64>,
        on_result: Option<ChatWallpaperResult>,
        cx: &mut App,
    ) {
        if chat_id.is_empty() {
            return;
        }
        let topic = topic_id.filter(|topic| !topic.is_empty()).map(str::to_owned);
        let services = ChatServicesStore::shared(cx);
        let previous = services.update(cx, |services, cx| match topic.as_deref() {
            None => services.chat_info(chat_id, cx).and_then(|chat| chat.wallpaper),
            Some(topic) => services.topic_from_any_cache(chat_id, topic, cx).and_then(|topic| topic.wallpaper),
        });
        match topic.as_deref() {
            None => apply_wallpaper(chat_id, None, wallpaper.clone(), cx),
            Some(topic) => {
                if wallpaper.is_some() {
                    apply_wallpaper(chat_id, Some(topic), wallpaper.clone(), cx);
                }
            }
        }

        let chat_id = chat_id.to_owned();
        let entry_id = entry_id.map(str::to_owned);
        cx.spawn(async move |cx| {
            let Some(domain) = active_domain(cx) else {
                tracing::warn!(target: "riseonly::X1056", chat = %chat_id, "set wallpaper: no active domain");
                cx.update(|cx| {
                    apply_wallpaper(&chat_id, topic.as_deref(), previous, cx);
                    if let Some(on_result) = on_result {
                        on_result(false, None, cx);
                    }
                });
                return;
            };
            let outcome = {
                let chats = domain.chats.clone();
                let chat_id = chat_id.clone();
                let topic = topic.clone();
                on_account_runtime(async move {
                    chats
                        .set_wallpaper(
                            &chat_id,
                            topic.as_deref(),
                            wallpaper.as_ref(),
                            entry_id.as_deref(),
                            dim_percent,
                            blur_percent,
                        )
                        .await
                })
                .await
            };
            cx.update(|cx| match outcome {
                Ok(confirmed) => {
                    tracing::warn!(target: "riseonly::X1056", chat = %chat_id, confirmed = confirmed.is_some(), "set wallpaper: done");
                    apply_wallpaper(&chat_id, topic.as_deref(), confirmed, cx);
                    if let Some(on_result) = on_result {
                        on_result(true, None, cx);
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target: "riseonly::X1056",
                        chat = %chat_id,
                        connection_loss = CanonErrors::is_connection_loss(&error),
                        error = ?error,
                        "set wallpaper: failed"
                    );
                    log_network(format!("set_chat_wallpaper({chat_id}) error: {error}"), cx);
                    apply_wallpaper(&chat_id, topic.as_deref(), previous, cx);
                    if let Some(on_result) = on_result {
                        on_result(false, non_empty(CanonErrors::refusal_text(&error)), cx);
                    }
                }
            });
        })
        .detach();
    }

    pub fn load_chat_wallpaper_catalog_action(&self, chat_id: &str, topic_id: Option<&str>, cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        let is_loading = ChatServicesStore::shared(cx)
            .update(cx, |services, cx| services.wallpaper_catalog(chat_id, topic_id, cx).is_loading);
        if is_loading {
            return;
        }
        mutate_catalog(
            chat_id,
            topic_id,
            |state| {
                state.is_loading = true;
                state.error = None;
            },
            cx,
        );
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        cx.spawn(async move |cx| {
            let Some(domain) = active_domain(cx) else {
                cx.update(|cx| mutate_catalog(&chat_id, topic_id.as_deref(), |state| state.is_loading = false, cx));
                return;
            };
            let outcome = {
                let chats = domain.chats.clone();
                let chat_id = chat_id.clone();
                let topic_id = topic_id.clone();
                on_account_runtime(async move { chats.wallpaper_catalog(&chat_id, topic_id.as_deref()).await }).await
            };
            cx.update(|cx| match outcome {
                Ok(entries) => mutate_catalog(
                    &chat_id,
                    topic_id.as_deref(),
                    |state| {
                        state.entries = entries;
                        state.is_loading = false;
                        state.is_loaded = true;
                    },
                    cx,
                ),
                Err(error) => mutate_catalog(
                    &chat_id,
                    topic_id.as_deref(),
                    |state| {
                        state.is_loading = false;
                        state.error = Some(CanonErrors::load_failure_text(&error));
                    },
                    cx,
                ),
            });
        })
        .detach();
    }

    pub fn add_chat_wallpaper_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        draft: RiseChatWallpaperDraft,
        look: ChatWallpaperLook,
        on_result: Option<ChatWallpaperAddResult>,
        cx: &mut App,
    ) {
        if chat_id.is_empty() {
            RiseChatWallpaperUpload::discard_staged_file(&draft.local_url);
            return;
        }
        let known_ids: HashSet<String> = ChatServicesStore::shared(cx).update(cx, |services, cx| {
            services.wallpaper_catalog(chat_id, topic_id, cx).entries.iter().map(|entry| entry.id.clone()).collect()
        });
        mutate_catalog(
            chat_id,
            topic_id,
            |state| {
                state.is_uploading = true;
                state.error = None;
            },
            cx,
        );
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        cx.spawn(async move |cx| {
            let _staged = StagedWallpaperFile(draft.local_url.clone());
            let outcome = Self::upload_catalog_wallpaper(&chat_id, topic_id.as_deref(), &draft, look, cx).await;
            cx.update(|cx| match outcome {
                Ok(None) => {
                    mutate_catalog(&chat_id, topic_id.as_deref(), |state| state.is_uploading = false, cx);
                    if let Some(on_result) = on_result {
                        on_result(false, None, None, cx);
                    }
                }
                Ok(Some((remote_url, entries))) => {
                    let added = entries
                        .iter()
                        .find(|entry| entry.image_url == remote_url)
                        .or_else(|| entries.iter().find(|entry| !known_ids.contains(&entry.id)))
                        .cloned();
                    mutate_catalog(
                        &chat_id,
                        topic_id.as_deref(),
                        |state| {
                            state.entries = entries;
                            state.is_loaded = true;
                            state.is_uploading = false;
                        },
                        cx,
                    );
                    if let Some(on_result) = on_result {
                        on_result(true, added, None, cx);
                    }
                }
                Err(error) => {
                    log_network(format!("add_chat_wallpaper({chat_id}) error: {error}"), cx);
                    let message = non_empty(CanonErrors::refusal_text(&error));
                    let shown = message.clone();
                    mutate_catalog(
                        &chat_id,
                        topic_id.as_deref(),
                        |state| {
                            state.is_uploading = false;
                            state.error = shown;
                        },
                        cx,
                    );
                    if let Some(on_result) = on_result {
                        on_result(false, None, message, cx);
                    }
                }
            });
        })
        .detach();
    }

    async fn upload_catalog_wallpaper(
        chat_id: &str,
        topic_id: Option<&str>,
        draft: &RiseChatWallpaperDraft,
        look: ChatWallpaperLook,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Option<(String, Vec<ChatWallpaperCatalogEntry>)>> {
        let remote_url = RiseChatWallpaperUpload::upload(draft, cx).await?;
        let Some(domain) = active_domain(cx) else { return Ok(None) };
        let chats = domain.chats.clone();
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        let image_url = remote_url.clone();
        let entries = on_account_runtime(async move {
            chats.add_wallpaper_catalog_entry(&chat_id, topic_id.as_deref(), &image_url, look).await
        })
        .await?;
        Ok(Some((remote_url, entries)))
    }

    pub fn delete_chat_wallpaper_entry_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        entry_id: &str,
        on_result: Option<ChatWallpaperResult>,
        cx: &mut App,
    ) {
        if chat_id.is_empty() || entry_id.is_empty() {
            return;
        }
        let previous = ChatServicesStore::shared(cx)
            .update(cx, |services, cx| services.wallpaper_catalog(chat_id, topic_id, cx).entries.clone());
        mutate_catalog(
            chat_id,
            topic_id,
            |state| {
                state.entries.retain(|entry| entry.id != entry_id);
                state.mutating_ids.insert(entry_id.to_owned());
                state.error = None;
            },
            cx,
        );
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        let entry_id = entry_id.to_owned();
        cx.spawn(async move |cx| {
            let Some(domain) = active_domain(cx) else {
                cx.update(|cx| {
                    mutate_catalog(
                        &chat_id,
                        topic_id.as_deref(),
                        |state| {
                            state.entries = previous;
                            state.mutating_ids.remove(&entry_id);
                        },
                        cx,
                    );
                    if let Some(on_result) = on_result {
                        on_result(false, None, cx);
                    }
                });
                return;
            };
            let outcome = {
                let chats = domain.chats.clone();
                let chat_id = chat_id.clone();
                let topic_id = topic_id.clone();
                let entry_id = entry_id.clone();
                on_account_runtime(async move {
                    chats.delete_wallpaper_catalog_entry(&chat_id, topic_id.as_deref(), &entry_id).await
                })
                .await
            };
            cx.update(|cx| match outcome {
                Ok(entries) => {
                    mutate_catalog(
                        &chat_id,
                        topic_id.as_deref(),
                        |state| {
                            state.entries = entries;
                            state.mutating_ids.remove(&entry_id);
                        },
                        cx,
                    );
                    if let Some(on_result) = on_result {
                        on_result(true, None, cx);
                    }
                }
                Err(error) => {
                    log_network(format!("delete_chat_wallpaper({chat_id}) error: {error}"), cx);
                    let message = non_empty(CanonErrors::refusal_text(&error));
                    let shown = message.clone();
                    mutate_catalog(
                        &chat_id,
                        topic_id.as_deref(),
                        |state| {
                            state.entries = previous;
                            state.mutating_ids.remove(&entry_id);
                            state.error = shown;
                        },
                        cx,
                    );
                    if let Some(on_result) = on_result {
                        on_result(false, message, cx);
                    }
                }
            });
        })
        .detach();
    }
}
