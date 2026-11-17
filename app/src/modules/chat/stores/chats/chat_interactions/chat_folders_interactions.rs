use std::rc::Rc;

use gpui::{App, Context};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::native_confirm_alert::NativeConfirmAlert;

use super::chat_interactions::ChatInteractionsStore;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

pub type ChatFolderOnResult = Rc<dyn Fn(bool, &mut App)>;

fn with_services<Output>(
    change: impl FnOnce(&mut ChatServicesStore, &mut Context<ChatServicesStore>) -> Output,
    cx: &mut App,
) -> Output {
    ChatServicesStore::shared(cx).update(cx, change)
}

impl ChatInteractionsStore {
    pub fn load_folders(&self, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_chat_folders_action(cx);
            actions.load_folder_contacts_action(None, cx);
        });
    }

    pub fn retry_rejected_chat_list_reads(&mut self, cx: &mut Context<Self>) {
        let actions = ChatActionsStore::shared(cx);
        if matches!(actions.read(cx).get_chat_folders_sai.status(), SaiWsSliceStatus::Rejected) {
            self.load_folders(cx);
        }
        if RiseConversationPresentation::shared(cx).read(cx).is_active() {
            return;
        }
        if matches!(actions.read(cx).get_message_requests_sai.status(), SaiWsSliceStatus::Rejected) {
            self.load_message_requests(cx);
        }
        if matches!(actions.read(cx).get_archived_chats_sai.status(), SaiWsSliceStatus::Rejected) {
            self.load_initial_archived_chats(cx);
        }
    }

    pub fn create_folder(&self, folder: ChatFolder, on_result: ChatFolderOnResult, cx: &mut Context<Self>) {
        if GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_folders()) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.create_folder(folder, cx));
            cx.defer(move |cx| on_result(true, cx));
            return;
        }
        if !with_services(|services, cx| services.begin_folder_mutation(cx), cx) {
            cx.defer(move |cx| on_result(false, cx));
            return;
        }
        let snapshot = ChatServicesStore::shared(cx).read(cx).folders_snapshot();
        let temp_id = format!("temp_folder_{}", Uuid::new_v4().uuid_string());
        let mut optimistic = folder.clone();
        optimistic.id = temp_id.clone();
        with_services(
            |services, cx| {
                optimistic.position = services.folders.len() as i64;
                services.upsert_folder_local(optimistic, cx);
            },
            cx,
        );
        let handler = Rc::new(move |folder_id: Option<String>, cx: &mut App| {
            let on_result = on_result.clone();
            let Some(folder_id) = folder_id.filter(|folder_id| !folder_id.is_empty()) else {
                let snapshot = snapshot.clone();
                with_services(
                    |services, cx| {
                        services.restore_folders_snapshot(snapshot, cx);
                        services.finish_folder_mutation(cx);
                    },
                    cx,
                );
                cx.defer(move |cx| on_result(false, cx));
                return;
            };
            with_services(
                |services, cx| {
                    services.replace_folder_id_local(&temp_id, &folder_id, cx);
                    services.finish_folder_mutation(cx);
                },
                cx,
            );
            cx.defer(move |cx| on_result(true, cx));
        });
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.create_chat_folder_action(&folder, handler, cx));
    }

    pub fn update_folder(&self, folder: ChatFolder, on_result: ChatFolderOnResult, cx: &mut Context<Self>) {
        let folder_id = folder.id.clone();
        if GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_folder(&folder_id)) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.update_folder(folder, cx));
            cx.defer(move |cx| on_result(true, cx));
            return;
        }
        if !with_services(|services, cx| services.begin_folder_mutation(cx), cx) {
            cx.defer(move |cx| on_result(false, cx));
            return;
        }
        let snapshot = ChatServicesStore::shared(cx).read(cx).folders_snapshot();
        let optimistic = folder.clone();
        with_services(|services, cx| services.upsert_folder_local(optimistic, cx), cx);
        let handler = Rc::new(move |ok: bool, authoritative_folder: Option<ChatFolder>, cx: &mut App| {
            let snapshot = snapshot.clone();
            with_services(
                |services, cx| {
                    if !ok {
                        services.restore_folders_snapshot(snapshot, cx);
                    } else if let Some(authoritative_folder) = authoritative_folder {
                        services.upsert_folder_local(authoritative_folder, cx);
                    }
                    services.finish_folder_mutation(cx);
                },
                cx,
            );
            let on_result = on_result.clone();
            cx.defer(move |cx| on_result(ok, cx));
        });
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.edit_chat_folder_action(&folder, handler, cx));
    }

    pub fn confirm_delete_folder(&self, folder_id: &str, cx: &mut Context<Self>) {
        let folder_id = folder_id.to_owned();
        NativeConfirmAlert::present_delete(
            &tr("folder_delete_title"),
            &tr("folder_delete_text"),
            None,
            None,
            false,
            None,
            None,
            move |cx| {
                Self::shared(cx).update(cx, |interactions, cx| interactions.delete_folder(&folder_id, cx));
            },
            cx,
        );
    }

    pub fn delete_folder(&self, folder_id: &str, cx: &mut Context<Self>) {
        if GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_folder(folder_id)) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_folder(folder_id, cx));
            return;
        }
        if !with_services(|services, cx| services.begin_folder_mutation(cx), cx) {
            return;
        }
        let snapshot = ChatServicesStore::shared(cx).read(cx).folders_snapshot();
        with_services(|services, cx| services.remove_folder_local(folder_id, cx), cx);
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.delete_chat_folder_action(
                folder_id,
                Rc::new(|cx: &mut App| with_services(|services, cx| services.finish_folder_mutation(cx), cx)),
                Rc::new(move |cx: &mut App| {
                    let snapshot = snapshot.clone();
                    with_services(
                        |services, cx| {
                            services.restore_folders_snapshot(snapshot, cx);
                            services.finish_folder_mutation(cx);
                        },
                        cx,
                    );
                }),
                cx,
            );
        });
    }

    pub fn reorder_folders(&self, ordered: Vec<ChatFolder>, cx: &mut Context<Self>) {
        if ordered.is_empty() {
            return;
        }
        let folder_ids: Vec<String> = ordered.iter().map(|folder| folder.id.clone()).collect();
        if GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_folders()) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.reorder_folders(folder_ids, cx));
            return;
        }
        if !with_services(|services, cx| services.begin_folder_mutation(cx), cx) {
            return;
        }
        let snapshot = ChatServicesStore::shared(cx).read(cx).folders_snapshot();
        with_services(|services, cx| services.set_folders_order_local(ordered, cx), cx);
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.reorder_chat_folders_action(
                folder_ids,
                Rc::new(|cx: &mut App| with_services(|services, cx| services.finish_folder_mutation(cx), cx)),
                Rc::new(move |cx: &mut App| {
                    let snapshot = snapshot.clone();
                    with_services(
                        |services, cx| {
                            services.restore_folders_snapshot(snapshot, cx);
                            services.finish_folder_mutation(cx);
                        },
                        cx,
                    );
                }),
                cx,
            );
        });
    }
}
