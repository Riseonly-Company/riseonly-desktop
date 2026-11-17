use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, Entity};
use rise_canon::{RiseGuideSignal, RiseTimingCanon};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_errors::ErrorCode;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_ui::native_confirm_alert::{NativeConfirmAlert, NativeConfirmAlertAction};

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::push_notifications::push_notification_manager_hooks::PushNotificationManagerHooks;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportDropWatch;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType, NotifierUndoOptions,
};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{ChatWallpaperLook, ChatWallpaperStore};
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationPresentation,
};
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};
use crate::modules::chat::shared::chat_notifications_debounce::ChatNotificationsDebounce;
use crate::modules::chat::stores::chat_guide_sandbox_hooks::chat_guide_sandbox_hooks;
use crate::modules::chat::stores::chat_stores_hooks::{chat_stores_components_hooks, chat_stores_user_hooks};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::{
    ChatActionCallback, ChatActionProgressCallback, ChatActionResultCallback, ChatActionsStore,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatInfo, ChatMuteScopes, ChatType, ChatWallpaperCatalogEntry, ChatWallpaperPlacement,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::{
    ChatServicesStore, ChatServicesStoreChatNotificationCommand,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::{RiseChatWallpaperDraft, RiseChatWallpaperUpload};
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_interactions::theme_wallpapers_interactions::ThemeWallpapersInteractionsStore;

pub type ChatInteractionsDone = Box<dyn FnOnce(&mut App)>;
pub type ChatInteractionsBoolResult = Box<dyn FnOnce(bool, &mut App)>;
pub type ChatInteractionsMessageResult = Box<dyn FnOnce(bool, Option<String>, &mut App)>;
pub type ChatInteractionsProgress = Rc<dyn Fn(f64, &mut App)>;

const NOTIFICATIONS_DEBOUNCE_MAX_CACHE_DATA: i64 = 50;

pub struct ChatInteractionsStore {
    pub topic_verify_reconnect_watches: HashMap<String, RiseTransportDropWatch>,
}

impl ChatInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { topic_verify_reconnect_watches: HashMap::new() }
    }

    pub fn actions(cx: &mut App) -> Entity<ChatActionsStore> {
        ChatActionsStore::shared(cx)
    }

    pub fn service(cx: &mut App) -> Entity<ChatServicesStore> {
        ChatServicesStore::shared(cx)
    }

    fn is_presentation_active(cx: &mut App) -> bool {
        RiseConversationPresentation::shared(cx).read(cx).is_active()
    }

    fn spawn_on_active_domain<Work, Fut>(work: Work, cx: &mut App)
    where
        Work: FnOnce(Arc<RiseConversationDomain>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() {
            drop(RiseAccountRuntime::current().spawn(work(domain)));
        }
    }

    fn can_request_trailing_page(cache_key: &str, cx: &mut App) -> bool {
        SaiWs::pagination_state(cache_key, cx).read(cx).trailing().can_request_automatically()
    }

    fn finish(completion: Option<ChatInteractionsDone>, cx: &mut App) {
        if let Some(completion) = completion {
            cx.defer(completion);
        }
    }

    fn answer(on_result: Option<ChatInteractionsMessageResult>, ok: bool, message: Option<String>, cx: &mut App) {
        if let Some(on_result) = on_result {
            cx.defer(move |cx| on_result(ok, message, cx));
        }
    }

    pub fn load_initial_chats(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.restore_undeliverable_draft_direct_chats(cx));
        if Self::is_presentation_active(cx) {
            Self::spawn_on_active_domain(
                |domain| async move {
                    domain.load_chats(RiseChatListKind::Inbox).await;
                },
                cx,
            );
            return;
        }
        let actions = Self::actions(cx);
        if actions.read(cx).get_chats_sai.is_pending() {
            return;
        }

        Self::service(cx).update(cx, |service, cx| service.preload_chats_slice_from_disk_if_needed(cx));
        let reflect_pending_in_ui = actions.read(cx).get_chats_sai.data().is_none();
        actions.update(cx, |actions, cx| actions.get_chats_action(None, false, false, Some(reflect_pending_in_ui), cx));
    }

    pub fn refresh_chats(&mut self, cx: &mut Context<Self>) {
        if Self::is_presentation_active(cx) {
            Self::spawn_on_active_domain(
                |domain| async move {
                    domain.refresh_chats(RiseChatListKind::Inbox).await;
                },
                cx,
            );
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_chats_action(None, false, true, Some(false), cx));
    }

    pub fn refresh_peer_chat(&mut self, peer_user_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let chat_id = service.read(cx).known_direct_chat_id(peer_user_id, cx);
        if chat_id.is_empty() {
            return;
        }
        if Self::is_presentation_active(cx) {
            Self::spawn_on_active_domain(
                move |domain| async move {
                    domain.refresh_direct_peer_chat(&chat_id).await;
                },
                cx,
            );
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_chats_action(None, false, true, Some(false), cx));
    }

    pub fn apply_peer_logo(&mut self, peer_user_id: &str, logo: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let chat_id = service.read(cx).known_direct_chat_id(peer_user_id, cx);
        if chat_id.is_empty() {
            return;
        }
        let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() else {
            return;
        };
        let peer_user_id = peer_user_id.to_owned();
        let logo = logo.to_owned();
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.apply_direct_peer_logo(&chat_id, &peer_user_id, &logo).await;
        }));
    }

    pub fn load_more_chats(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if Self::is_presentation_active(cx) {
            if !service.read(cx).chats_has_more(cx) {
                return;
            }
            Self::spawn_on_active_domain(
                |domain| async move {
                    let _ = domain.load_more_chats(RiseChatListKind::Inbox).await;
                },
                cx,
            );
            return;
        }
        let Some(cache_key) = service.read(cx).chats_cache_key(cx) else {
            return;
        };
        let actions = Self::actions(cx);
        let Some(page) = actions.read(cx).get_chats_sai.data() else {
            return;
        };
        if page.is_have_more != Some(true) {
            return;
        }

        let Some(anchor) = page.relative_id.filter(|anchor| !anchor.is_empty()) else {
            return;
        };

        if !Self::can_request_trailing_page(&cache_key, cx) {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_chats_action(Some(anchor), false, true, Some(false), cx));
    }

    pub fn load_more_folder_chat_search(&mut self, query: &str, cx: &mut Context<Self>) {
        let Some(user_id) = AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
        else {
            return;
        };
        let trimmed = trimming_whitespaces_and_newlines(query).to_owned();
        if trimmed.is_empty() {
            return;
        }
        let cache_key = ChatServicesStore::folder_chat_search_cache_key(&user_id, &trimmed);
        let service = Self::service(cx);
        let Some(page) = service.update(cx, |service, cx| service.folder_chat_search_page(&trimmed, &user_id, cx)) else {
            return;
        };
        if page.is_have_more != Some(true) {
            return;
        }
        let Some(cursor) = page.relative_id.clone().filter(|cursor| !cursor.is_empty()) else {
            return;
        };
        if !Self::can_request_trailing_page(&cache_key, cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.search_folder_chats_action(&trimmed, Some(&cursor), cx));
    }

    pub fn mute_chat(&mut self, chat_id: &str, duration_seconds: Option<f64>, cx: &mut Context<Self>) {
        let until = duration_seconds.map(|seconds| RiseClock::now().time_interval_since_1970() + seconds);

        Self::actions(cx).update(cx, |actions, cx| actions.mute_chat_action(chat_id, until, None, cx));
    }

    pub fn unmute_chat(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.unmute_chat_action(chat_id, cx));
    }

    pub fn toggle_chat_muted(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let Some(chat) = service.read(cx).chat_info(chat_id, cx) else {
            return;
        };
        if !service.read(cx).effective_is_muted(&chat, cx) {
            self.mute_chat(chat_id, None, cx);
            return;
        }

        if let Some(hooks) = chat_stores_user_hooks()
            && (hooks.mute_all_chats_enabled)(cx)
            && !(hooks.chat_mute_override_ids)(cx).contains(chat_id)
        {
            (hooks.add_chat_mute_exception)(chat_id, cx);
        }
        if chat.is_muted || chat.notification_scopes().is_fully_muted() {
            self.unmute_chat(chat_id, cx);
        }
    }

    pub fn pin_chat(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let is_pinned = service.read(cx).current_chat_infos(cx).iter().any(|chat| chat.id == chat_id && chat.is_pinned);
        if is_pinned {
            return;
        }
        let is_premium =
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.more.is_premium).unwrap_or(false);
        let limit = ChatServicesStore::max_pinned_chats(is_premium);
        if service.read(cx).pinned_count(cx) >= limit {
            Self::notify_pin_limit(limit, is_premium, cx);
            return;
        }
        let snapshot = service.read(cx).current_chat_infos(cx);
        let order = service.read(cx).pinned_reorder_inserting(chat_id, cx);
        service.update(cx, |service, cx| service.apply_pinned_order_local(&order, cx));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.reorder_pinned_chats_action(
                order,
                None,
                Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                    Self::service(cx).update(cx, |service, cx| service.restore_chats_snapshot(snapshot.clone(), cx));
                    Self::report_pin_failure(error, limit, is_premium, cx);
                }),
                cx,
            )
        });
    }

    pub fn pin_chat_in_folder(&mut self, chat_id: &str, folder_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let Some(folder) = service
            .read(cx)
            .folder(folder_id, cx)
            .filter(|folder| !folder.pinned_chat_ids.iter().any(|id| id == chat_id))
        else {
            return;
        };
        let is_premium =
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.more.is_premium).unwrap_or(false);
        let limit = ChatServicesStore::max_pinned_chats(is_premium);
        if i64::try_from(folder.pinned_chat_ids.len()).unwrap_or(i64::MAX) >= limit {
            Self::notify_pin_limit(limit, is_premium, cx);
            return;
        }
        let snapshot = service.read(cx).folders_snapshot();
        let order: Vec<String> =
            std::iter::once(chat_id.to_owned()).chain(folder.pinned_chat_ids.iter().cloned()).collect();
        service.update(cx, |service, cx| service.set_folder_pinned_local(folder_id, &order, cx));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.reorder_pinned_chats_action(
                order,
                Some(folder_id),
                Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                    Self::service(cx).update(cx, |service, cx| service.restore_folders_snapshot(snapshot.clone(), cx));
                    Self::report_pin_failure(error, limit, is_premium, cx);
                }),
                cx,
            )
        });
    }

    pub fn unpin_chat_in_folder(&mut self, chat_id: &str, folder_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let Some(folder) = service
            .read(cx)
            .folder(folder_id, cx)
            .filter(|folder| folder.pinned_chat_ids.iter().any(|id| id == chat_id))
        else {
            return;
        };
        let snapshot = service.read(cx).folders_snapshot();
        let remaining: Vec<String> = folder.pinned_chat_ids.into_iter().filter(|id| id != chat_id).collect();
        service.update(cx, |service, cx| service.set_folder_pinned_local(folder_id, &remaining, cx));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.unpin_chat_action(
                chat_id,
                Some(folder_id),
                Rc::new(move |cx: &mut App| {
                    Self::service(cx).update(cx, |service, cx| service.restore_folders_snapshot(snapshot.clone(), cx));
                }),
                cx,
            )
        });
    }

    fn report_pin_failure(error: &anyhow::Error, limit: i64, is_premium: bool, cx: &mut App) {
        let code = error
            .downcast_ref::<SaiWsResponseError>()
            .and_then(|response| response.code.clone())
            .unwrap_or_else(|| RiseMessageRepository::error_code(error));
        if code == ErrorCode::ChatPinLimit.wire() {
            Self::notify_pin_limit(limit, is_premium, cx);
            return;
        }
        Notifier::send(
            CanonErrors::refusal_text(error),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("pin.slash.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn notify_pin_limit(limit: i64, is_premium: bool, cx: &mut App) {
        let title = tr_args("chat_pin_limit_title", &[FormatArg::from(usize::try_from(limit).unwrap_or(0))]);
        Notifier::send(
            title.clone(),
            NotifierSendOptions {
                content: Some(tr("chat_pin_limit_subtitle")),
                r#type: NotifierType::Warning,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("pin.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
        let pinned_chats_limit = ChatServicesStore::pinned_chats_limit();
        PremiumUpsellStore::shared(cx).update(cx, |store, cx| {
            store.offer_client_limit(&pinned_chats_limit, title, is_premium, cx);
        });
    }

    pub fn unpin_chat(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let snapshot = service.read(cx).current_chat_infos(cx);
        service.update(cx, |service, cx| service.set_chat_pinned_local(chat_id, false, cx));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.unpin_chat_action(
                chat_id,
                None,
                Rc::new(move |cx: &mut App| {
                    Self::service(cx).update(cx, |service, cx| service.restore_chats_snapshot(snapshot.clone(), cx));
                }),
                cx,
            )
        });
    }

    pub fn toggle_pin_chat(&mut self, chat_id: &str, folder_id: Option<&str>, cx: &mut Context<Self>) {
        if let Some(folder_id) = folder_id {
            let pinned = Self::service(cx)
                .read(cx)
                .folder(folder_id, cx)
                .is_some_and(|folder| folder.pinned_chat_ids.iter().any(|id| id == chat_id));
            if pinned {
                self.unpin_chat_in_folder(chat_id, folder_id, cx);
            } else {
                self.pin_chat_in_folder(chat_id, folder_id, cx);
            }
            return;
        }
        let is_pinned = Self::service(cx)
            .read(cx)
            .current_chat_infos(cx)
            .iter()
            .find(|chat| chat.id == chat_id)
            .is_some_and(|chat| chat.is_pinned);
        if is_pinned {
            self.unpin_chat(chat_id, cx);
        } else {
            self.pin_chat(chat_id, cx);
        }
    }

    pub fn reorder_pinned_chats(&mut self, ordered_ids: Vec<String>, folder_id: Option<&str>, cx: &mut Context<Self>) {
        if ordered_ids.is_empty() {
            return;
        }
        let service = Self::service(cx);
        if let Some(folder_id) = folder_id {
            let snapshot = service.read(cx).folders_snapshot();
            service.update(cx, |service, cx| service.set_folder_pinned_local(folder_id, &ordered_ids, cx));
            Self::actions(cx).update(cx, |actions, cx| {
                actions.reorder_pinned_chats_action(
                    ordered_ids,
                    Some(folder_id),
                    Rc::new(move |_: &anyhow::Error, cx: &mut App| {
                        Self::service(cx)
                            .update(cx, |service, cx| service.restore_folders_snapshot(snapshot.clone(), cx));
                    }),
                    cx,
                )
            });
            return;
        }
        let snapshot = service.read(cx).current_chat_infos(cx);
        service.update(cx, |service, cx| service.apply_pinned_order_local(&ordered_ids, cx));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.reorder_pinned_chats_action(
                ordered_ids,
                None,
                Rc::new(move |_: &anyhow::Error, cx: &mut App| {
                    Self::service(cx).update(cx, |service, cx| service.restore_chats_snapshot(snapshot.clone(), cx));
                }),
                cx,
            )
        });
    }

    pub fn update_notifications(&mut self, chat_id: &str, scopes: ChatMuteScopes, cx: &mut Context<Self>) {
        let command = ChatServicesStoreChatNotificationCommand::new(chat_id.to_owned(), scopes);
        let service = Self::service(cx);
        let current = service
            .read(cx)
            .chat_info(chat_id, cx)
            .map_or(ChatMuteScopes::ALL_ENABLED, |chat| chat.notification_scopes());
        service.update(cx, |service, cx| service.apply_notif_scopes_local(chat_id, scopes, cx));
        if !service.update(cx, |service, _| service.enqueue_chat_notification_command(command.clone(), current)) {
            return;
        }
        Self::dispatch_chat_notification_command(command, cx);
    }

    fn dispatch_chat_notification_command(command: ChatServicesStoreChatNotificationCommand, cx: &mut App) {
        let succeeded = command.clone();
        let failed = command.clone();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_chat_notifications_action(
                &command.chat_id,
                command.scopes,
                Rc::new(move |cx: &mut App| Self::finish_chat_notification_command(&succeeded, true, cx)),
                Rc::new(move |cx: &mut App| Self::finish_chat_notification_command(&failed, false, cx)),
                cx,
            )
        });
    }

    fn finish_chat_notification_command(command: &ChatServicesStoreChatNotificationCommand, success: bool, cx: &mut App) {
        let service = Self::service(cx);
        let (accepted, next, rollback) =
            service.update(cx, |service, _| service.complete_chat_notification_command(command, success));
        if !accepted {
            return;
        }
        if let Some(next) = next {
            Self::dispatch_chat_notification_command(next, cx);
        } else if let Some(rollback) = rollback {
            service.update(cx, |service, cx| service.apply_notif_scopes_local(&command.chat_id, rollback, cx));
        }
    }

    pub fn update_notifications_debounced(&mut self, chat_id: &str, scopes: ChatMuteScopes, cx: &mut Context<Self>) {
        let owned_chat_id = chat_id.to_owned();
        SaiDebouncer::debounced_action_delay_milliseconds(
            chat_id,
            Box::new(move |cx: &mut App| {
                if let Some(hooks) = chat_stores_user_hooks()
                    && (hooks.mute_all_chats_enabled)(cx)
                {
                    let is_exception = (hooks.chat_mute_override_ids)(cx).contains(&owned_chat_id);
                    let should_be_exception = !scopes.is_fully_muted();
                    if should_be_exception && !is_exception {
                        (hooks.add_chat_mute_exception)(&owned_chat_id, cx);
                    } else if !should_be_exception && is_exception {
                        (hooks.remove_chat_mute_exception)(&owned_chat_id, cx);
                    }
                }
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.update_notifications(&owned_chat_id, scopes, cx);
                });
            }),
            RiseTimingCanon::debounce("chat_notifications").client_debounce_ms,
            ChatNotificationsDebounce::GROUP_KEY,
            NOTIFICATIONS_DEBOUNCE_MAX_CACHE_DATA,
            None,
            cx,
        );
    }

    pub fn transfer_ownership(
        &mut self,
        chat_id: &str,
        new_owner_id: &str,
        on_result: Option<ChatInteractionsBoolResult>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx)
            .update(cx, |actions, cx| actions.transfer_ownership_action(chat_id, new_owner_id, on_result.map(Self::once_result), cx));
    }

    pub fn set_auto_delete(&mut self, chat_id: &str, seconds: i64, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.set_auto_delete_action(chat_id, seconds, cx));
    }

    pub fn leave_chat(&mut self, chat_id: &str, completion: Option<ChatInteractionsDone>, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.leave_chat_action(chat_id, cx));
        Self::finish(completion, cx);
    }

    pub fn confirm_leave_chat(
        &mut self,
        chat: &ChatInfo,
        on_confirm: Option<ChatInteractionsDone>,
        completion: Option<ChatInteractionsDone>,
        cx: &mut Context<Self>,
    ) {
        let is_channel = chat.r#type == ChatType::Channel;
        let title = if is_channel { tr("chat_leave_channel_confirm_title") } else { tr("chat_leave_group_confirm_title") };
        let confirm_title = if is_channel { tr("chat_leave_channel") } else { tr("chat_leave_group") };
        let chat_id = chat.id.clone();
        NativeConfirmAlert::present_delete(
            &title,
            trimming_whitespaces_and_newlines(&chat.title),
            Some(confirm_title),
            None,
            true,
            None,
            None,
            move |cx: &mut App| {
                if let Some(on_confirm) = on_confirm {
                    on_confirm(cx);
                }
                Self::shared(cx).update(cx, |store, cx| store.leave_chat(&chat_id, completion, cx));
            },
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn delete_chat(
        &mut self,
        chat_id: &str,
        delete_for_both: bool,
        clear_history: bool,
        known_chat: Option<ChatInfo>,
        on_error: Option<ChatInteractionsDone>,
        completion: Option<ChatInteractionsDone>,
        cx: &mut Context<Self>,
    ) {
        if ChatServicesStore::is_draft_direct_chat_id(chat_id) {
            let on_error = on_error.unwrap_or_else(|| Box::new(|_: &mut App| {}));
            let draft_chat_id = chat_id.to_owned();
            let on_cleared: ChatInteractionsDone = Box::new(move |cx: &mut App| {
                Self::service(cx).update(cx, |service, cx| service.remove_draft_direct_chat(&draft_chat_id, cx));
                ChatDraftStore::shared(cx).update(cx, |drafts, cx| drafts.clear_chat(&draft_chat_id, cx));
                if let Some(completion) = completion {
                    completion(cx);
                }
            });
            MessageInteractionsStore::shared(cx).update(cx, |messages, cx| {
                messages.discard_draft_chat_history(chat_id, on_error, on_cleared, cx);
            });
            return;
        }

        let actions = Self::actions(cx);
        let pending =
            actions.update(cx, |actions, cx| actions.begin_chat_delete_action(chat_id, known_chat, clear_history, cx));

        if pending.keeps_chat() {
            actions.update(cx, |actions, cx| {
                actions.delete_chat_action(
                    chat_id,
                    delete_for_both,
                    clear_history,
                    Some(pending),
                    on_error.map(Self::once_callback),
                    cx,
                );
            });
            Self::finish(completion, cx);
            return;
        }

        let title = Self::chat_delete_undo_title(pending.chat.as_ref(), delete_for_both);
        let commit_chat_id = chat_id.to_owned();
        let commit_pending = pending.clone();
        Notifier::send_undo(
            title,
            tr("common_cancel"),
            NotifierUndoOptions { position: NotifierPosition::Bottom, ..NotifierUndoOptions::default() },
            move |cx: &mut App| {
                ChatActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.delete_chat_action(
                        &commit_chat_id,
                        delete_for_both,
                        false,
                        Some(commit_pending),
                        on_error.map(Self::once_callback),
                        cx,
                    );
                });
            },
            move |cx: &mut App| {
                ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.cancel_chat_delete_action(&pending, cx));
            },
            cx,
        );

        Self::finish(completion, cx);
    }

    fn send_delete_failure(title: String, cx: &mut App) {
        Notifier::send(
            title,
            NotifierSendOptions {
                content: Some(tr("chat_delete_failed_text")),
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn confirm_clear_history(&mut self, chat: &ChatInfo, cx: &mut Context<Self>) {
        if chat.r#type != ChatType::Direct || chat.is_official_bot_chat() {
            self.confirm_delete_chat(chat, None, None, cx);
            return;
        }
        let known_chat = chat.clone();
        NativeConfirmAlert::present_delete(
            &tr("chat_clear_history_dialog_title"),
            &tr("chat_clear_history_private_dialog_body").replace("{name}", &Self::chat_delete_peer_name(chat)),
            Some(tr("chat_clear_history_confirm")),
            None,
            true,
            None,
            None,
            move |cx: &mut App| {
                let chat_id = known_chat.id.clone();
                Self::shared(cx).update(cx, |store, cx| {
                    store.delete_chat(
                        &chat_id,
                        false,
                        true,
                        Some(known_chat),
                        Some(Box::new(|cx: &mut App| {
                            Self::send_delete_failure(tr("chat_clear_history_failed_title"), cx);
                        })),
                        None,
                        cx,
                    );
                });
            },
            cx,
        );
    }

    pub fn confirm_delete_chat(
        &mut self,
        chat: &ChatInfo,
        on_transfer: Option<NativeConfirmAlertAction>,
        completion: Option<ChatInteractionsDone>,
        cx: &mut Context<Self>,
    ) {
        let known_chat = chat.clone();
        let completion = Rc::new(RefCell::new(completion));
        let delete: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |for_both: bool, cx: &mut App| {
            let chat_id = known_chat.id.clone();
            let known_chat = known_chat.clone();
            let completion = completion.borrow_mut().take();
            Self::shared(cx).update(cx, |store, cx| {
                store.delete_chat(
                    &chat_id,
                    for_both,
                    false,
                    Some(known_chat),
                    Some(Box::new(|cx: &mut App| {
                        Self::send_delete_failure(tr("chat_delete_failed_title"), cx);
                    })),
                    completion,
                    cx,
                );
            });
        });
        if chat.is_official_bot_chat() {
            NativeConfirmAlert::present_delete(
                &tr("chat_clear_history_dialog_title"),
                &tr("chat_clear_history_dialog_body"),
                Some(tr("chat_clear_history_confirm")),
                None,
                true,
                None,
                None,
                move |cx: &mut App| delete(false, cx),
                cx,
            );
            return;
        }
        let title = Self::chat_delete_dialog_title(chat);
        let message = Self::chat_delete_dialog_body(chat);
        match chat.r#type {
            ChatType::Group | ChatType::Channel => {
                let secondary_title = on_transfer.as_ref().map(|_| tr("chat_transfer_title"));
                NativeConfirmAlert::present_delete(
                    &title,
                    &message,
                    Some(tr("chat_delete_for_everyone")),
                    secondary_title,
                    false,
                    on_transfer,
                    None,
                    move |cx: &mut App| delete(true, cx),
                    cx,
                );
            }
            ChatType::Direct => {
                let delete_for_both = delete.clone();
                NativeConfirmAlert::present_delete(
                    &title,
                    &message,
                    Some(tr("chat_delete_for_me")),
                    Some(tr("chat_delete_for_both").replace("{name}", &Self::chat_delete_peer_name(chat))),
                    true,
                    Some(Box::new(move |cx: &mut App| delete_for_both(true, cx))),
                    None,
                    move |cx: &mut App| delete(false, cx),
                    cx,
                );
            }
            _ => {
                NativeConfirmAlert::present_delete(
                    &title,
                    &message,
                    Some(tr("chat_delete_for_me")),
                    None,
                    true,
                    None,
                    None,
                    move |cx: &mut App| delete(false, cx),
                    cx,
                );
            }
        }
    }

    pub fn chat_delete_peer_name(chat: &ChatInfo) -> String {
        if chat.r#type == ChatType::Direct {
            let name = chat.direct_peer_profile_name();
            return if name.is_empty() { tr("chat_delete_fallback_name") } else { name };
        }
        let title = trimming_whitespaces_and_newlines(&chat.title);
        if title.is_empty() { tr("chat_delete_fallback_name") } else { title.to_owned() }
    }

    pub fn chat_delete_dialog_title(chat: &ChatInfo) -> String {
        match chat.r#type {
            ChatType::Group => tr("chat_delete_group"),
            ChatType::Channel => tr("chat_delete_channel"),
            _ => tr("chat_delete_dialog_title"),
        }
    }

    pub fn chat_delete_dialog_body(chat: &ChatInfo) -> String {
        let body = match chat.r#type {
            ChatType::Group => tr("chat_delete_group_dialog_body"),
            ChatType::Channel => tr("chat_delete_channel_dialog_body"),
            _ => tr("chat_delete_dialog_body"),
        };
        body.replace("{name}", &Self::chat_delete_peer_name(chat))
    }

    fn chat_delete_undo_title(chat: Option<&ChatInfo>, delete_for_both: bool) -> String {
        let chat_type = chat.map(|chat| chat.r#type);
        if chat_type == Some(ChatType::Channel) {
            return tr("chat_delete_undo_channel");
        }
        if chat_type == Some(ChatType::Group) {
            return tr("chat_delete_undo_group");
        }
        if !delete_for_both {
            return tr("chat_delete_undo_for_me");
        }

        if chat_type == Some(ChatType::Direct) {
            return tr("chat_delete_undo_for_both");
        }
        tr("chat_delete_undo_for_everyone")
    }

    #[allow(clippy::too_many_arguments)]
    pub fn note_community_opening(
        &mut self,
        chat_id: &str,
        is_channel: bool,
        title: &str,
        logo_url: &str,
        tag: Option<&str>,
        is_verified: bool,
        is_public: bool,
        member_count: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let service = Self::service(cx);
        if service.read(cx).chat_info(chat_id, cx).is_some() {
            service.update(cx, |service, cx| service.set_opening_hint(None, cx));
            return;
        }
        let mut hint = ChatInfo::new(chat_id.to_owned());
        hint.r#type = if is_channel { ChatType::Channel } else { ChatType::Group };
        hint.title = title.to_owned();
        hint.logo_url = if logo_url.is_empty() { None } else { Some(logo_url.to_owned()) };
        hint.tag = tag.map(str::to_owned);
        hint.is_verified = is_verified;
        hint.is_public = is_public;
        hint.member_count = member_count.unwrap_or(0).max(0);
        service.update(cx, |service, cx| service.set_opening_hint(Some(hint), cx));
    }

    pub fn note_community_opening_row(&mut self, row: &SearchEntityRow, chat_id: &str, cx: &mut Context<Self>) {
        if !row.is_channel() && !row.is_group() {
            return;
        }
        self.note_community_opening(
            chat_id,
            row.is_channel(),
            &row.display_name,
            &row.logo_url,
            row.user_tag.as_deref(),
            row.is_premium,
            row.is_public.unwrap_or(true),
            row.member_count,
            cx,
        );
    }

    pub fn update_chat(
        &mut self,
        chat_id: &str,
        title: Option<&str>,
        description: Option<&str>,
        logo_url: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !SaiWebSocketManager::shared(cx).read(cx).is_connected() {
            Notifier::send(
                tr("sync_error_unavailable"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            return false;
        }
        let title = title.map(str::to_owned);
        let description = description.map(str::to_owned);
        let logo_url = logo_url.map(str::to_owned);
        Self::actions(cx)
            .update(cx, |actions, cx| actions.update_chat_action(chat_id, title, description, logo_url, cx));
        true
    }

    fn can_set_wallpaper(chat_id: &str, cx: &mut App) -> bool {
        if chat_id.is_empty() {
            return false;
        }
        let chat = Self::service(cx).read(cx).chat_info(chat_id, cx);
        chat.is_some_and(|chat| chat_stores_components_hooks().is_some_and(|hooks| (hooks.can_set)(&chat, cx)))
    }

    fn sandbox_owns_wallpaper(chat_id: &str, topic_id: Option<&str>, cx: &App) -> bool {
        topic_id.is_none() && (chat_guide_sandbox_hooks().owns_wallpaper)(chat_id, cx)
    }

    pub fn set_chat_wallpaper(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        entry: ChatWallpaperCatalogEntry,
        on_result: Option<ChatInteractionsMessageResult>,
        cx: &mut Context<Self>,
    ) {
        if entry.id.is_empty() || !Self::can_set_wallpaper(chat_id, cx) {
            tracing::warn!(target: "riseonly::X1056", chat = chat_id, entry_empty = entry.id.is_empty(), "set wallpaper: refused locally");
            Self::answer(on_result, false, None, cx);
            return;
        }
        if Self::sandbox_owns_wallpaper(chat_id, topic_id, cx) {
            (chat_guide_sandbox_hooks().pin_wallpaper)(&entry, cx);
            Self::answer(on_result, true, None, cx);
            return;
        }
        let placement = ChatWallpaperPlacement {
            id: entry.id.clone(),
            image_url: entry.image_url.clone(),
            preview_url: entry.preview_url.clone(),
            look: entry.look,
            set_by_user_id: AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default(),
            updated_at_ms: (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64,
        };
        let owned_chat_id = chat_id.to_owned();
        let on_set: ChatInteractionsMessageResult = Box::new(move |ok: bool, message: Option<String>, cx: &mut App| {
            if ok {
                let wallpapers = ChatWallpaperStore::shared(cx);
                let wallpapers = wallpapers.read(cx);
                if wallpapers.own_wallpapers_only() && !wallpapers.shows_chat_wallpaper(&owned_chat_id, cx) {
                    ThemeWallpapersInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.set_shows_chat_wallpaper(true, &owned_chat_id, cx);
                    });
                }
                GuideSignalBus::emit(RiseGuideSignal::ChatWallpaperSet);
            }
            if let Some(on_result) = on_result {
                on_result(ok, message, cx);
            }
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.set_chat_wallpaper_action(
                chat_id,
                topic_id,
                Some(placement),
                Some(&entry.id),
                Some(entry.look.dim_percent),
                Some(entry.look.blur_percent),
                Some(on_set),
                cx,
            );
        });
    }

    pub fn clear_chat_wallpaper(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        on_result: Option<ChatInteractionsMessageResult>,
        cx: &mut Context<Self>,
    ) {
        if !Self::can_set_wallpaper(chat_id, cx) {
            Self::answer(on_result, false, None, cx);
            return;
        }
        if Self::sandbox_owns_wallpaper(chat_id, topic_id, cx) {
            (chat_guide_sandbox_hooks().unpin_wallpaper)(cx);
            Self::answer(on_result, true, None, cx);
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.set_chat_wallpaper_action(chat_id, topic_id, None, None, None, None, on_result, cx);
        });
    }

    pub fn load_chat_wallpaper_catalog(&mut self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }
        let invented = (chat_guide_sandbox_hooks().wallpaper_catalog)(chat_id, cx).is_some();
        if invented {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_chat_wallpaper_catalog_action(chat_id, topic_id, cx));
    }

    pub fn add_chat_wallpaper(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        draft: RiseChatWallpaperDraft,
        look: ChatWallpaperLook,
        on_result: Option<ChatInteractionsMessageResult>,
        cx: &mut Context<Self>,
    ) {
        let is_uploading = Self::service(cx).read(cx).wallpaper_catalog(chat_id, topic_id, cx).is_uploading;
        if !Self::can_set_wallpaper(chat_id, cx) || is_uploading {
            RiseChatWallpaperUpload::discard_staged_file(&draft.local_url);
            Self::answer(on_result, false, None, cx);
            return;
        }
        if Self::sandbox_owns_wallpaper(chat_id, topic_id, cx) {
            (chat_guide_sandbox_hooks().adopt_wallpaper)(draft, look.clamped(), cx);
            Self::answer(on_result, true, None, cx);
            return;
        }
        let owned_chat_id = chat_id.to_owned();
        let owned_topic_id = topic_id.map(str::to_owned);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.add_chat_wallpaper_action(
                chat_id,
                topic_id,
                draft,
                look.clamped(),
                Some(Box::new(
                    move |ok: bool, added: Option<ChatWallpaperCatalogEntry>, message: Option<String>, cx: &mut App| {
                        let Some(added) = added.filter(|_| ok) else {
                            Self::answer(on_result, ok, message, cx);
                            return;
                        };
                        cx.defer(move |cx| {
                            Self::shared(cx).update(cx, |store, cx| {
                                store.set_chat_wallpaper(
                                    &owned_chat_id,
                                    owned_topic_id.as_deref(),
                                    added,
                                    on_result,
                                    cx,
                                );
                            });
                        });
                    },
                )),
                cx,
            );
        });
    }

    pub fn delete_chat_wallpaper_entry(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        entry_id: &str,
        on_result: Option<ChatInteractionsMessageResult>,
        cx: &mut Context<Self>,
    ) {
        let is_mutating =
            Self::service(cx).read(cx).wallpaper_catalog(chat_id, topic_id, cx).mutating_ids.contains(entry_id);
        if entry_id.is_empty() || !Self::can_set_wallpaper(chat_id, cx) || is_mutating {
            Self::answer(on_result, false, None, cx);
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.delete_chat_wallpaper_entry_action(chat_id, topic_id, entry_id, on_result, cx);
        });
    }

    pub fn update_chat_visibility(
        &mut self,
        chat_id: &str,
        is_public: Option<bool>,
        tag: &str,
        forward_in_chat_enabled: Option<bool>,
        on_result: ChatInteractionsMessageResult,
        cx: &mut Context<Self>,
    ) {
        let on_result = RefCell::new(Some(on_result));
        let on_result: ChatActionResultCallback = Rc::new(move |ok: bool, message: Option<String>, cx: &mut App| {
            let on_result = on_result.borrow_mut().take();
            if let Some(on_result) = on_result {
                on_result(ok, message, cx);
            }
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_chat_visibility_action(chat_id, is_public, tag, forward_in_chat_enabled, on_result, cx);
        });
    }

    pub fn update_chat_logo(
        &mut self,
        chat_id: &str,
        local_file: SaiWsOptimisticLocalFile,
        on_progress: Option<ChatInteractionsProgress>,
        on_error: Option<ChatInteractionsDone>,
        cx: &mut Context<Self>,
    ) {
        let on_progress: ChatActionProgressCallback = match on_progress {
            Some(on_progress) => on_progress,
            None => Rc::new(|_: f64, _: &mut App| {}),
        };
        let on_error: ChatActionCallback = match on_error {
            Some(on_error) => Self::once_callback(on_error),
            None => Rc::new(|_: &mut App| {}),
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_chat_logo_action(chat_id, local_file, on_progress, on_error, cx);
        });
    }

    pub(crate) fn once_callback(done: ChatInteractionsDone) -> ChatActionCallback {
        let done = RefCell::new(Some(done));
        Rc::new(move |cx: &mut App| {
            let done = done.borrow_mut().take();
            if let Some(done) = done {
                done(cx);
            }
        })
    }

    pub fn clear_delivered_notifications(&mut self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        if let Some(hooks) = PushNotificationManagerHooks::installed() {
            (hooks.remove_delivered_notifications)(chat_id, topic_id, cx);
        }
    }
}
