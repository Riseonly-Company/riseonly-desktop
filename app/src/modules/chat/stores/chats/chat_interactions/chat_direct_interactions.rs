use std::cell::Cell;
use std::rc::Rc;

use futures::channel::oneshot;
use gpui::{App, AsyncApp, Context};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use super::chat_interactions::ChatInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

impl ChatInteractionsStore {
    pub fn direct_conversation_id(&self, participant: &User, cx: &mut Context<Self>) -> String {
        let peer_id = trimming_whitespaces_and_newlines(&participant.id);
        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        if peer_id.is_empty() || viewer_id.as_deref() == Some(peer_id) {
            return String::new();
        }

        ChatServicesStore::shared(cx).update(cx, |services, cx| {
            let known = services.known_direct_chat_id(peer_id, cx);
            if !known.is_empty() {
                return known;
            }
            services.register_draft_direct_chat(participant, cx)
        })
    }

    pub fn materialize_draft_direct_chat(
        &self,
        draft_id: &str,
        completion: impl FnOnce(Option<String>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let services = ChatServicesStore::shared(cx);
        if let Some(real_id) = services.update(cx, |services, cx| services.real_chat_id(draft_id, cx)) {
            cx.defer(move |cx| completion(Some(real_id), cx));
            return;
        }

        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let peer_id = ChatServicesStore::draft_direct_chat_peer_id(draft_id);
        let draft = services.read(cx).draft_direct_chat(draft_id).filter(|draft| {
            !viewer_id.is_empty()
                && !peer_id.is_empty()
                && peer_id != viewer_id
                && trimming_whitespaces_and_newlines(&draft.participant.id) == peer_id
        });
        let Some(draft) = draft else {
            cx.defer(move |cx| completion(None, cx));
            return;
        };

        let completion = Rc::new(Cell::new(Some(completion)));
        let open_completion = completion.clone();
        let draft_id = draft_id.to_owned();
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.open_direct_chat_action(
                draft.participant,
                Box::new(move |real_id: String, cx: &mut App| {
                    ChatServicesStore::shared(cx)
                        .update(cx, |services, cx| services.finalize_created_chat(&draft_id, &real_id, cx));
                    if let Some(completion) = open_completion.take() {
                        completion(Some(real_id), cx);
                    }
                }),
                Some(Box::new(move |_: String, cx: &mut App| {
                    if let Some(completion) = completion.take() {
                        completion(None, cx);
                    }
                })),
                cx,
            );
        });
    }

    pub async fn server_chat_id(chat_id: String, cx: &mut AsyncApp) -> Option<String> {
        if !ChatServicesStore::is_draft_direct_chat_id(&chat_id) {
            return Some(chat_id);
        }
        let (answer, answered) = oneshot::channel();
        cx.update(|cx| {
            Self::shared(cx).update(cx, |interactions, cx| {
                interactions.materialize_draft_direct_chat(
                    &chat_id,
                    move |real_id, _| {
                        let _ = answer.send(real_id);
                    },
                    cx,
                );
            });
        });
        answered.await.ok().flatten()
    }
}
