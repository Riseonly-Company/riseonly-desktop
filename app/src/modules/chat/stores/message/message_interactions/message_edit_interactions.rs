use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context};
use rise_canon::{RiseGuideSignal, RiseValidation};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::CancellationError;
use rise_errors::ErrorCode;
use rise_i18n::tr;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::core::widgets::sheets::share_sheet::ShareForwardIdentity;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::messages::rise_message_contracts::RiseMessageRepositoryError;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_upload_retry::RiseMessageUploadResend;
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageDeleteBatchPlanner, MessageDeliveryStatus, MessageForwardError, MessageMediaItem,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    ForwardMessageDestinationBody, ForwardMessagesGatewayDTO,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;

pub type MessageInteractionsErrorHandler = Rc<dyn Fn(&anyhow::Error, &mut App)>;
pub type MessageForwardCompletion = Box<dyn FnOnce(anyhow::Result<ForwardMessagesGatewayDTO>, &mut App)>;

fn bottom_error_options() -> NotifierSendOptions {
    NotifierSendOptions {
        r#type: NotifierType::Error,
        position: NotifierPosition::Bottom,
        ..NotifierSendOptions::default()
    }
}

fn forward_pair(message_id: Option<&str>, chat_id: Option<&str>, topic_id: Option<&str>) -> String {
    [
        ShareForwardIdentity::canonical(message_id).unwrap_or_default(),
        ShareForwardIdentity::canonical(chat_id).unwrap_or_default(),
        ShareForwardIdentity::canonical(topic_id).unwrap_or_default(),
    ]
    .join("\u{0}")
}

impl MessageInteractionsStore {
    pub fn edit_message_media(
        &mut self,
        message_id: &str,
        item: MediaPickerItem,
        caption: &str,
        entities: Option<&str>,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_message_id(message_id)) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.replace_photo(item, cx));
            return;
        }
        let Some(resolved) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };

        let report_failure = Self::edit_failure_reporter();
        let persisted_message_id = message_id.to_owned();
        let persisted_item = item.clone();
        let persisted_chat_id = resolved.clone();
        let persisted_topic_id = topic_id.map(str::to_owned);
        let persisted_thread_root_id = thread_root_id.map(str::to_owned);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.edit_message_media_action(
                message_id,
                &resolved,
                caption,
                entities.map(str::to_owned),
                item,
                topic_id,
                thread_root_id,
                Some(Box::new(move |success: bool, cx: &mut App| {
                    if !success {
                        return;
                    }
                    GuideSignalBus::emit(RiseGuideSignal::MessageMediaReplaced);
                    Self::persist_reedit_doc(
                        &persisted_message_id,
                        &persisted_item,
                        &persisted_chat_id,
                        persisted_topic_id.as_deref(),
                        persisted_thread_root_id.as_deref(),
                        cx,
                    );
                })),
                Some(Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                    report_failure(error, cx);
                })),
                cx,
            );
        });
    }

    fn edit_failure_reporter() -> impl Fn(&anyhow::Error, &mut App) -> bool + 'static {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        move |error: &anyhow::Error, cx: &mut App| {
            if error.downcast_ref::<CancellationError>().is_some()
                || !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
            {
                return false;
            }
            if PremiumUpsellStore::shared(cx).update(cx, |store, cx| store.offer_covers(error, cx)) {
                return true;
            }
            let text = Self::message_refusal_text(error).unwrap_or_else(|| tr("save_failed"));
            Notifier::send(text, bottom_error_options(), cx);
            true
        }
    }

    pub fn message_refusal_text(error: &anyhow::Error) -> Option<String> {
        CanonErrors::failure(error)
            .and_then(|failure| CanonErrors::text_for(&failure))
            .or_else(|| CanonErrors::text(Some(&RiseMessageRepository::error_code(error)), &HashMap::new()))
    }

    pub fn cancel_outgoing_message(&mut self, message: &GetMessageMessage, chat: &ChatInfo, cx: &mut Context<Self>) {
        if !message.is_pending_outgoing() && message.delivery_status != MessageDeliveryStatus::Failed {
            return;
        }
        self.drop_parked_draft_send(&message.id, &chat.id);
        let scope = RiseMessageScope::new(chat.id.clone(), message.topic_id.clone(), None);
        let local_id = message.id.clone();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                return;
            };
            domain.cancel_outgoing_message(&local_id, &scope).await;
        })
        .detach();
        cx.notify();
    }

    pub fn retry_outgoing_message(&mut self, message: &GetMessageMessage, chat: &ChatInfo, cx: &mut Context<Self>) {
        if message.delivery_status != MessageDeliveryStatus::Failed {
            return;
        }
        if self.is_parked_draft_send(&message.id, &chat.id) {
            self.retry_parked_draft_sends(&chat.id, cx);
            return;
        }
        if self.can_repark_draft_send(message, &chat.id) {
            self.repark_draft_send(message, &chat.id, cx);
            self.retry_parked_draft_sends(&chat.id, cx);
            return;
        }
        if ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.slow_mode_blocks_send(&chat.id, cx)) {
            return;
        }
        let scope = RiseMessageScope::new(chat.id.clone(), message.topic_id.clone(), None);
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() else {
            return;
        };
        if account_epoch.is_none() || Some(domain.account_id.as_str()) != account_user_id.as_deref() {
            return;
        }
        let local_id = message.id.clone();
        let chat_id = chat.id.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let is_current = |cx: &AsyncApp, domain: &Arc<RiseConversationDomain>| {
                SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
                    && cx
                        .update(|cx| RiseConversationDomainRegistry::shared(cx).read(cx).active())
                        .is_some_and(|active| Arc::ptr_eq(&active, domain))
            };
            if !is_current(cx, &domain) {
                return;
            }
            let resend = domain.outgoing_upload_resend(&local_id).await;
            if !is_current(cx, &domain) {
                return;
            }
            if let Some(resend) = resend {
                let _ = this.update(cx, |store, cx| store.resend_upload(resend, &local_id, domain.clone(), cx));
                return;
            }
            let watches_slow_mode = cx.update(|cx| {
                ChatServicesStore::shared(cx).update(cx, |services, cx| {
                    services.start_slow_mode(&chat_id, RiseClock::now(), cx);
                    services.slow_mode_may_apply(&chat_id, cx)
                })
            });
            domain.retry_outgoing_message(&local_id, &scope).await;
            if !watches_slow_mode {
                return;
            }
            let Err(error) = domain.wait_for_message_submission(&local_id).await else {
                return;
            };
            let Some(RiseMessageRepositoryError::RemoteFailure(code)) = error.downcast_ref::<RiseMessageRepositoryError>()
            else {
                return;
            };
            if code != ErrorCode::ChatSlowModeActive.wire()
                || !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
            {
                return;
            }
            cx.update(|cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.slow_mode_refused_send(&chat_id, cx));
            });
        })
        .detach();
    }

    fn resend_upload(
        &mut self,
        resend: RiseMessageUploadResend,
        local_id: &str,
        domain: Arc<RiseConversationDomain>,
        cx: &mut Context<Self>,
    ) {
        let mut body = resend.body;
        if body.media_group_id.is_some() {
            body.media_group_id = Some(format!("mg_{}", RiseClock::now().ms()));
        }
        let chat_id = resend.scope.chat_id.clone();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let has_local_files = !resend.local_files.is_empty();
        let weak_store = cx.weak_entity();
        let bound_domain = domain.clone();

        Self::actions(cx).update(cx, |actions, cx| {
            actions.create_message_action(
                body,
                &chat_id,
                resend.scope.topic_id.as_deref(),
                resend.scope.thread_root_id.as_deref(),
                resend.local_files,
                resend.picker_items,
                resend.voice_metadata_slots,
                local_id,
                true,
                None,
                Some(bound_domain),
                Some(Box::new(move |result: anyhow::Result<()>, cx: &mut App| {
                    let Err(error) = result else {
                        return;
                    };
                    let is_active = RiseConversationDomainRegistry::shared(cx)
                        .read(cx)
                        .active()
                        .is_some_and(|active| Arc::ptr_eq(&active, &domain));
                    if !has_local_files
                        || !SaiAccountCacheScope::is_current_account_epoch(
                            account_user_id.as_deref(),
                            account_epoch.as_deref(),
                        )
                        || !is_active
                        || error.downcast_ref::<CancellationError>().is_some()
                    {
                        return;
                    }
                    cx.defer(move |cx| {
                        if PremiumUpsellStore::shared(cx).update(cx, |store, cx| store.offer_covers(&error, cx)) {
                            tracing::warn!(target: "riseonly::X1801", %error, "resend failure covered by premium offer");
                            return;
                        }
                        let content = weak_store.upgrade().and_then(|store| store.read(cx).media_send_failure_text(&error));
                        Notifier::send(
                            tr("chat_media_send_failed"),
                            NotifierSendOptions { content, ..bottom_error_options() },
                            cx,
                        );
                    });
                })),
                cx,
            );
        });
        Self::service(cx).update(cx, |service, cx| service.bump_chat_list(&chat_id, cx));
    }

    pub fn cancel_outgoing_media(
        &mut self,
        message: &GetMessageMessage,
        chat: &ChatInfo,
        media_item: &MessageMediaItem,
        cx: &mut Context<Self>,
    ) {
        let active_item_count = message.media_items.iter().filter(|item| !item.is_cancelled).count();
        let is_create = message.is_temp || message.delivery_status == MessageDeliveryStatus::Sending;
        let local_id = message.id.clone();
        if is_create {
            self.drop_parked_draft_send(&message.id, &chat.id);
            cx.notify();
            if active_item_count <= 1 {
                let rise_scope = RiseMessageScope::new(chat.id.clone(), message.topic_id.clone(), None);
                cx.spawn(async move |_, cx: &mut AsyncApp| {
                    let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                        return;
                    };
                    domain.cancel_outgoing_message(&local_id, &rise_scope).await;
                })
                .detach();
            } else {
                let local_slot_index = media_item.local_slot_index.max(0);
                cx.spawn(async move |_, cx: &mut AsyncApp| {
                    let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                        return;
                    };
                    let _ = domain.cancel_outgoing_media_slot(&local_id, local_slot_index).await;
                })
                .detach();
            }
            return;
        }
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                return;
            };
            let _ = domain.cancel_outgoing_media_slot(&local_id, 0).await;
        })
        .detach();
    }

    fn persist_reedit_doc(
        message_id: &str,
        item: &MediaPickerItem,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        cx: &mut App,
    ) {
        let Some(img) = item.image_payload.as_ref() else {
            return;
        };
        let (Some(state_data), Some(original_url)) = (img.editor_state_data.as_ref(), img.original_image_url.as_ref())
        else {
            return;
        };
        let Some(original_path) = original_url.to_file_path() else {
            return;
        };
        let reedit = MediaReeditStore::shared(cx);
        let Some(edit_id) = reedit.update(cx, |store, _| {
            store.save_document(&original_path, state_data, MediaReeditStore::chat_ttl_seconds())
        }) else {
            return;
        };
        reedit.update(cx, |store, _| store.link(message_id, &edit_id));
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let server_id = Self::service(cx).update(cx, |service, cx| service.server_message_id(message_id, &scope, cx));
        if !server_id.is_empty() {
            reedit.update(cx, |store, _| store.link(&server_id, &edit_id));
        }
    }

    pub fn refuses_caption_edit(&self, caption: &str, cx: &mut App) -> bool {
        let Some(refusal) = RiseValidation::MESSAGE_CAPTION.error_text(caption) else {
            return false;
        };
        Notifier::send(refusal, bottom_error_options(), cx);
        true
    }

    pub fn edit_message(
        &mut self,
        message_id: &str,
        new_content: &str,
        entities: Option<&str>,
        link_preview_type: Option<&str>,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        on_failure: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let trimmed = trimming_whitespaces_and_newlines(new_content);
        if trimmed.is_empty() {
            return;
        }

        let report_failure = Self::edit_failure_reporter();
        let on_failure = Cell::new(on_failure);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.edit_message_action(
                message_id,
                trimmed,
                entities.map(str::to_owned),
                link_preview_type.map(str::to_owned),
                chat_id,
                topic_id,
                thread_root_id,
                Some(Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                    if report_failure(error, cx)
                        && let Some(on_failure) = on_failure.take()
                    {
                        on_failure(cx);
                    }
                })),
                cx,
            );
        });
    }

    pub fn delete_messages(
        &mut self,
        ids: &[String],
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        for_everyone: bool,
        cx: &mut Context<Self>,
    ) {
        if RiseTransportStatus::is_waiting_for_network(cx) {
            RiseTransportStatus::wake_if_offline(cx);
        }
        let report_failure = Self::delete_failure_reporter(chat_id);
        let actions = Self::actions(cx);
        let ids = ids.to_vec();
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        let thread_root_id = thread_root_id.map(str::to_owned);
        HoldContextMenuPresenter::after_dismiss(
            move |cx: &mut App| {
                for batch in MessageDeleteBatchPlanner::chunks(&ids) {
                    let report_failure = report_failure.clone();
                    actions.update(cx, |actions, cx| {
                        actions.delete_messages_action(
                            batch,
                            &chat_id,
                            topic_id.as_deref(),
                            thread_root_id.as_deref(),
                            for_everyone,
                            Some(report_failure),
                            cx,
                        );
                    });
                }
            },
            cx,
        );
    }

    pub fn delete_failure_reporter(chat_id: &str) -> MessageInteractionsErrorHandler {
        let chat_id = chat_id.to_owned();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let reported = Cell::new(false);
        Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if reported.get()
                || error.downcast_ref::<CancellationError>().is_some()
                || !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
            {
                return;
            }
            reported.set(true);
            let code = RiseMessageRepository::error_code(error);
            if code == ErrorCode::ChatPermissionDenied.wire() && !chat_id.is_empty() {
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_effective_permissions(&chat_id, None, true, cx));
            }
            let text = Self::message_refusal_text(error).unwrap_or_else(|| tr("retry_later_error"));
            Notifier::send(text, bottom_error_options(), cx);
        })
    }

    pub fn forward_messages(
        &mut self,
        message_ids: Vec<String>,
        destinations: Vec<ForwardMessageDestinationBody>,
        idempotency_key: String,
        completion: MessageForwardCompletion,
        cx: &mut Context<Self>,
    ) {
        let completion = Rc::new(Cell::new(Some(completion)));
        let error_completion = completion.clone();
        let expected_key = idempotency_key.clone();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.forward_messages_action(
                message_ids,
                destinations,
                idempotency_key,
                Box::new(
                    move |response: ForwardMessagesGatewayDTO,
                          sent_message_ids: Vec<String>,
                          sent_destinations: Vec<ForwardMessageDestinationBody>,
                          cx: &mut App| {
                        let Some(completion) = completion.take() else {
                            return;
                        };
                        let expected_message_ids: HashSet<String> = sent_message_ids
                            .iter()
                            .filter_map(|id| ShareForwardIdentity::canonical(Some(id)))
                            .collect();
                        let expected_destinations: HashSet<(Option<String>, Option<String>)> = sent_destinations
                            .iter()
                            .map(|destination| (destination.chat_id.clone(), destination.topic_id.clone()))
                            .collect();
                        let expected_pairs: HashSet<String> = expected_destinations
                            .iter()
                            .flat_map(|(chat_id, topic_id)| {
                                expected_message_ids.iter().map(move |message_id| {
                                    forward_pair(Some(message_id), chat_id.as_deref(), topic_id.as_deref())
                                })
                            })
                            .collect();
                        let is_valid = response.idempotency_key.as_deref().is_some_and(|key| {
                            key.to_lowercase() == expected_key.to_lowercase()
                        }) && response.results.as_ref().is_some_and(|results| {
                            let actual_pairs: HashSet<String> = results
                                .iter()
                                .map(|result| {
                                    forward_pair(
                                        Some(&result.source_message_id),
                                        result.destination.as_ref().map(|destination| destination.chat_id.as_str()),
                                        result.destination.as_ref().and_then(|destination| destination.topic_id.as_deref()),
                                    )
                                })
                                .collect();
                            results.len() == expected_message_ids.len() * expected_destinations.len()
                                && actual_pairs.len() == results.len()
                                && actual_pairs == expected_pairs
                        });
                        if !is_valid {
                            completion(Err(anyhow::Error::new(MessageForwardError::InvalidResponse)), cx);
                            return;
                        }
                        Self::service(cx).update(cx, |service, cx| service.apply_forward_messages_success(&response, cx));
                        completion(Ok(response), cx);
                    },
                ),
                Box::new(move |error: anyhow::Error, cx: &mut App| {
                    if let Some(completion) = error_completion.take() {
                        completion(Err(error), cx);
                    }
                }),
                cx,
            );
        });
    }
}
