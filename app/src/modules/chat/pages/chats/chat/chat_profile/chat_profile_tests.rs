use std::collections::HashSet;

use rise_chat::ChatType;

use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile::{
    ChatProfileOverviewLayoutPolicy, ChatProfileScrollCoordinatePolicy,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab::ChatProfileTab;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tabs::{
    ChatProfilePaneMountPolicy, ChatProfileViewportLayoutPolicy,
};
use crate::modules::user::components::profile_content::profile_scroll_coordinator::{
    ProfileMasterPinningPolicy, ProfileScrollMetrics,
};

fn tab_set(tabs: &[ChatProfileTab]) -> HashSet<ChatProfileTab> {
    tabs.iter().copied().collect()
}

fn scroll_opacity(frame_max_y: f32) -> f32 {
    ChatProfileOverviewLayoutPolicy::scroll_opacity(
        frame_max_y,
        ProfileScrollMetrics::CHAT_PINNED_TABS_TOP,
        ChatProfileViewportLayoutPolicy::OUTER_SPACING,
    )
}

#[test]
fn favourites_expose_only_meaningful_tabs_in_stable_order() {
    assert_eq!(
        ChatProfileTab::ordered(ChatType::Favourites, false, false, true),
        vec![
            ChatProfileTab::Media,
            ChatProfileTab::Fav,
            ChatProfileTab::File,
            ChatProfileTab::Link,
            ChatProfileTab::Music,
            ChatProfileTab::Voice,
            ChatProfileTab::Gif,
        ]
    );
}

#[test]
fn members_first_changes_presentation_order_without_changing_semantic_keys() {
    let default_tabs = ChatProfileTab::ordered(ChatType::Group, false, false, true);
    let members_first_tabs = ChatProfileTab::ordered(ChatType::Group, true, false, true);

    assert_eq!(default_tabs.first(), Some(&ChatProfileTab::Media));
    assert_eq!(members_first_tabs.first(), Some(&ChatProfileTab::Members));
    assert_eq!(tab_set(&default_tabs), tab_set(&members_first_tabs));
    assert_eq!(ChatProfileTab::Members.tab_index(), members_first_tabs[0].tab_index());
}

#[test]
fn pager_mount_window_never_exceeds_current_page_and_neighbors() {
    let tabs = ChatProfileTab::ALL;

    assert_eq!(
        ChatProfilePaneMountPolicy::window(&tabs, ChatProfileTab::Media),
        tab_set(&[ChatProfileTab::Media, ChatProfileTab::Fav])
    );
    assert_eq!(
        ChatProfilePaneMountPolicy::window(&tabs, ChatProfileTab::Music),
        tab_set(&[ChatProfileTab::Link, ChatProfileTab::Music, ChatProfileTab::Voice])
    );
    assert_eq!(
        ChatProfilePaneMountPolicy::window(&tabs, ChatProfileTab::Gif),
        tab_set(&[ChatProfileTab::Members, ChatProfileTab::Gif, ChatProfileTab::Stories])
    );
    assert!(ChatProfilePaneMountPolicy::window(&tabs, ChatProfileTab::Music).len() <= 3);
}

#[test]
fn pager_never_accepts_gesture_toward_an_unprepared_heavy_page() {
    let tabs = ChatProfileTab::ALL;

    assert_eq!(
        ChatProfilePaneMountPolicy::prepared_directions(
            &tabs,
            ChatProfileTab::Music,
            &tab_set(&[ChatProfileTab::Link, ChatProfileTab::Music]),
        ),
        HashSet::from([-1])
    );
    assert_eq!(
        ChatProfilePaneMountPolicy::prepared_directions(
            &tabs,
            ChatProfileTab::Music,
            &tab_set(&[ChatProfileTab::Link, ChatProfileTab::Music, ChatProfileTab::Voice]),
        ),
        HashSet::from([-1, 1])
    );
}

#[test]
fn scope_transition_drops_far_heavy_pages_immediately() {
    let tabs = ChatProfileTab::ALL;

    assert_eq!(
        ChatProfilePaneMountPolicy::retained_during_transition(
            &tabs,
            ChatProfileTab::Gif,
            &tab_set(&[ChatProfileTab::Media, ChatProfileTab::Fav, ChatProfileTab::File]),
        ),
        tab_set(&[ChatProfileTab::Gif])
    );
    assert_eq!(
        ChatProfilePaneMountPolicy::retained_during_transition(
            &tabs,
            ChatProfileTab::Music,
            &tab_set(&[
                ChatProfileTab::File,
                ChatProfileTab::Link,
                ChatProfileTab::Music,
                ChatProfileTab::Voice,
                ChatProfileTab::Groups,
            ]),
        ),
        tab_set(&[ChatProfileTab::Link, ChatProfileTab::Music, ChatProfileTab::Voice])
    );
}

#[test]
fn chat_pager_underlaps_floating_chrome_without_changing_feed_viewport() {
    let viewport_height = 844.0;
    let container_height = ChatProfileViewportLayoutPolicy::container_height(viewport_height);
    let pager_height = ChatProfileViewportLayoutPolicy::pager_height(container_height);

    assert_eq!(container_height, viewport_height - ProfileScrollMetrics::CHAT_PINNED_TABS_TOP);
    assert_eq!(pager_height, viewport_height);
    assert_eq!(
        ChatProfileViewportLayoutPolicy::page_content_top_inset() - ChatProfileViewportLayoutPolicy::HEADER_UNDERLAP,
        ProfileScrollMetrics::TABS_BAR_HEIGHT
    );
}

#[test]
fn overview_fades_completely_before_tabs_join_the_sticky_layer() {
    let hidden_max_y = ProfileScrollMetrics::CHAT_PINNED_TABS_TOP - ChatProfileViewportLayoutPolicy::OUTER_SPACING;
    let fade_distance = ChatProfileOverviewLayoutPolicy::FADE_DISTANCE;

    assert_eq!(scroll_opacity(hidden_max_y + fade_distance), 1.0);
    assert_eq!(scroll_opacity(hidden_max_y + fade_distance / 2.0), 0.5);
    assert_eq!(scroll_opacity(hidden_max_y), 0.0);
    assert_eq!(scroll_opacity(hidden_max_y - 80.0), 0.0);
}

#[test]
fn list_shell_inset_is_not_counted_twice_by_chat_profile_sticky_geometry() {
    let floating_top_inset = 115.0;
    let local_pinned_top = ChatProfileScrollCoordinatePolicy::local_pinned_tabs_top(floating_top_inset);

    assert_eq!(local_pinned_top, ProfileScrollMetrics::CHAT_PINNED_TABS_TOP - floating_top_inset);
    assert_eq!(ProfileMasterPinningPolicy::correction(local_pinned_top, 500.0, true, local_pinned_top), 0.0);
    assert_eq!(ProfileMasterPinningPolicy::correction(local_pinned_top - 36.0, 500.0, true, local_pinned_top), 36.0);
    assert_eq!(
        ChatProfileOverviewLayoutPolicy::scroll_opacity(
            local_pinned_top - ChatProfileViewportLayoutPolicy::OUTER_SPACING,
            local_pinned_top,
            ChatProfileViewportLayoutPolicy::OUTER_SPACING,
        ),
        0.0
    );
}

#[test]
fn short_pane_keeps_room_to_pin_tabs_under_floating_chrome() {
    let viewport_height = 844.0;
    let inset = 64.0;
    let pinned_top = ProfileScrollMetrics::CHAT_PINNED_TABS_TOP.min(inset);
    let container_height = ChatProfileViewportLayoutPolicy::pinned_container_height(viewport_height, inset);

    assert_eq!(container_height, viewport_height - pinned_top);
    assert_eq!(
        ChatProfileViewportLayoutPolicy::pinned_container_height(viewport_height, 500.0),
        ChatProfileViewportLayoutPolicy::container_height(viewport_height)
    );
}
