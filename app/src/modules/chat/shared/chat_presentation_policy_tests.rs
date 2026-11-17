use std::collections::HashSet;

use gpui::TestAppContext;

use super::chat_presentation_policy::{ChatProfileNavigationPolicy, ChatProfileOpenChatDecision};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatType, User};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

fn id(value: &str) -> Option<String> {
    Some(value.to_owned())
}

fn chat(chat_id: &str) -> RootRoute {
    RootRoute::Chat { chat_id: id(chat_id) }
}

fn chat_profile(chat_id: &str) -> RootRoute {
    RootRoute::ChatProfile { chat_id: id(chat_id) }
}

fn chat_ids(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn decision(path: &[RootRoute], ids: &[&str]) -> ChatProfileOpenChatDecision {
    ChatProfileNavigationPolicy::open_chat_decision(path, &chat_ids(ids), None)
}

#[test]
fn test_draft_direct_chat_id_round_trips() {
    let draft_id = ChatServicesStore::draft_direct_chat_id(" peer-1 ");
    assert_eq!(draft_id, "temp_direct_peer-1");
    assert!(ChatServicesStore::is_draft_direct_chat_id(&draft_id));
    assert!(GetSaiServerId::is_temp_id(Some(draft_id.as_str())));
    assert_eq!(ChatServicesStore::draft_direct_chat_peer_id(&draft_id), "peer-1");
    assert_eq!(ChatServicesStore::draft_direct_chat_id("   "), "");
    assert!(!ChatServicesStore::is_draft_direct_chat_id("real-chat"));
    assert_eq!(ChatServicesStore::draft_direct_chat_peer_id("real-chat"), "");
}

#[gpui::test]
fn test_draft_is_registered_outside_every_chat_list_and_evicts_oldest(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let store = ChatServicesStore::shared(cx);
        let before = store.read(cx).current_chat_infos(cx).len();
        let mut registered = Vec::new();
        for index in 0..(ChatServicesStore::DRAFT_DIRECT_CHAT_LIMIT + 3) {
            let peer = User {
                id: format!("peer-{index}"),
                name: format!("Peer {index}"),
                ..User::default()
            };
            registered.push(store.update(cx, |store, cx| store.register_draft_direct_chat(&peer, cx)));
        }

        assert_eq!(store.read(cx).current_chat_infos(cx).len(), before);
        assert_eq!(store.read(cx).draft_direct_chat_by_id.len(), ChatServicesStore::DRAFT_DIRECT_CHAT_LIMIT);
        assert!(store.read(cx).draft_direct_chat(&registered[0]).is_none());

        let newest = registered[registered.len() - 1].clone();
        let draft = store.read(cx).draft_direct_chat(&newest);
        assert_eq!(draft.as_ref().map(|draft| draft.r#type), Some(ChatType::Direct));
        assert_eq!(
            draft.as_ref().map(|draft| draft.participant.id.clone()),
            Some(format!("peer-{}", registered.len() - 1))
        );
        assert_eq!(store.read(cx).chat_info(&newest, cx).map(|chat| chat.id), Some(newest.clone()));

        for draft_id in &registered {
            store.update(cx, |store, cx| store.remove_draft_direct_chat(draft_id, cx));
        }
        assert!(store.read(cx).draft_direct_chat_by_id.is_empty());
        assert!(store.read(cx).draft_direct_chat_order.is_empty());
    });
}

#[test]
fn test_open_chat_decision_pushes_when_the_conversation_is_not_behind_the_profile() {
    let members_path = [
        chat("group"),
        chat_profile("group"),
        RootRoute::ChatSettingsMembers { chat_id: id("group") },
        chat_profile("temp_direct_peer"),
    ];

    assert_eq!(decision(&members_path, &["temp_direct_peer"]), ChatProfileOpenChatDecision::Push);
}

#[test]
fn test_open_chat_decision_unwinds_to_the_conversation_behind_the_profile() {
    assert_eq!(
        decision(&[chat("c1"), chat_profile("c1")], &["c1"]),
        ChatProfileOpenChatDecision::Unwind { depth: 1 }
    );
    assert_eq!(
        decision(
            &[
                RootRoute::ForumHome { chat_id: id("forum") },
                RootRoute::ChatTopic { chat_id: id("forum"), topic_id: id("t1") },
                RootRoute::TopicProfile { chat_id: id("forum"), topic_id: id("t1") },
            ],
            &["forum"],
        ),
        ChatProfileOpenChatDecision::Unwind { depth: 2 }
    );
    assert_eq!(
        decision(&[chat("temp_direct_peer"), chat_profile("real")], &["real", "temp_direct_peer"]),
        ChatProfileOpenChatDecision::Unwind { depth: 1 }
    );
}

#[test]
fn test_open_chat_decision_pushes_for_an_empty_or_rootless_stack() {
    assert_eq!(decision(&[], &["c1"]), ChatProfileOpenChatDecision::Push);
    assert_eq!(decision(&[chat_profile("c1")], &["c1"]), ChatProfileOpenChatDecision::Push);
    assert_eq!(decision(&[chat("c1"), chat_profile("c1")], &[""]), ChatProfileOpenChatDecision::Push);
}
