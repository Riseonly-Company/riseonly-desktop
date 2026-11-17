use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::{
    RiseChatSearchRecentDestination, RiseChatSearchRecentItem, RiseChatSearchResultsPolicy,
};

fn recent(id: &str) -> RiseChatSearchRecentItem {
    RiseChatSearchRecentItem {
        id: id.to_owned(),
        destination: RiseChatSearchRecentDestination::Chat,
        title: id.to_owned(),
        subtitle: String::new(),
        logo_url: String::new(),
        chat_id: Some(id.to_owned()),
        user_id: None,
        user_tag: None,
        is_online: false,
    }
}

#[test]
fn test_global_results_expand_only_when_local_results_exist() {
    assert_eq!(RiseChatSearchResultsPolicy::global_limit(0, false), 10);
    assert_eq!(RiseChatSearchResultsPolicy::global_limit(2, false), 3);
    assert_eq!(RiseChatSearchResultsPolicy::global_limit(2, true), 10);
}

#[test]
fn test_recent_insertion_deduplicates_moves_to_front_and_bounds_history() {
    let current: Vec<RiseChatSearchRecentItem> = (0..12).map(|index| recent(&format!("item-{index}"))).collect();
    let existing = current[5].clone();

    let moved = RiseChatSearchResultsPolicy::inserting_recent(existing.clone(), &current);
    assert_eq!(moved.len(), 12);
    assert_eq!(moved.first().map(|item| item.id.as_str()), Some(existing.id.as_str()));
    assert_eq!(moved.iter().filter(|item| item.id == existing.id).count(), 1);

    let inserted = RiseChatSearchResultsPolicy::inserting_recent(recent("new-item"), &current);
    assert_eq!(inserted.len(), 12);
    assert_eq!(inserted.first().map(|item| item.id.as_str()), Some("new-item"));
    assert!(!inserted.iter().any(|item| item.id == "item-11"));
}
