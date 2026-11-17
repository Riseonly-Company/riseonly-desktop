use crate::app::router::navigation_types::{MainTab, RootRoute};

/// desktop sections whose right panel has a default route
/// a section sets its default from its own file
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AsideSection {
    Posts,
    Profile,
    Chats,
    Search,
    Music,
    Riffs,
    Maps,
    Settings,
}

impl AsideSection {
    pub const ALL: [Self; 8] =
        [Self::Posts, Self::Profile, Self::Chats, Self::Search, Self::Music, Self::Riffs, Self::Maps, Self::Settings];

    pub fn from_context_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.context_key() == key)
    }

    /// the section the person sees, a pushed profile or chat or settings page wins over the selected tab
    pub fn of(tab: MainTab, top_route: Option<&RootRoute>) -> Option<Self> {
        match top_route {
            Some(RootRoute::UserPage { .. } | RootRoute::MyProfile) => return Some(Self::Profile),
            Some(RootRoute::Settings) => return Some(Self::Settings),
            Some(RootRoute::MusicTab) => return Some(Self::Music),
            Some(RootRoute::WorldMap) => return Some(Self::Maps),
            Some(RootRoute::ShortViewer { .. }) => return Some(Self::Riffs),
            Some(RootRoute::Chat { .. } | RootRoute::ChatTopic { .. } | RootRoute::ForumAll { .. }) => {
                return Some(Self::Chats);
            }
            _ => {}
        }
        match tab {
            MainTab::Posts | MainTab::Notifications => Some(Self::Posts),
            MainTab::GlobalSearch => Some(Self::Search),
            MainTab::Chats => Some(Self::Chats),
            MainTab::Shorts => Some(Self::Riffs),
            MainTab::Profile => Some(Self::Profile),
            MainTab::Music => Some(Self::Music),
            MainTab::Maps => Some(Self::Maps),
            MainTab::Vacancies => None,
        }
    }

    /// key so the panel remembers open or closed per context
    pub fn context_key(self) -> &'static str {
        match self {
            Self::Posts => "aside_posts",
            Self::Profile => "aside_profile",
            Self::Chats => "aside_chats",
            Self::Search => "aside_search",
            Self::Music => "aside_music",
            Self::Riffs => "aside_riffs",
            Self::Maps => "aside_maps",
            Self::Settings => "aside_settings",
        }
    }
}

/// true for detail screens that open in the panel and not the centre stack
/// person and post and chat info and activity and music request and people lists
pub fn opens_beside_centre(route: &RootRoute) -> bool {
    is_chat_info(route)
        || is_music_request_detail(route)
        || is_profile_people_list(route)
        || matches!(route, RootRoute::UserPage { .. } | RootRoute::PostDetail { .. } | RootRoute::Notifications)
}

/// true for a profile subscribers, subscriptions or friends list
pub fn is_profile_people_list(route: &RootRoute) -> bool {
    matches!(route, RootRoute::UserSubscribers { .. } | RootRoute::UserSubs { .. } | RootRoute::UserFriends { .. })
}

/// true for a music moderation request or an artist studio track or album
pub fn is_music_request_detail(route: &RootRoute) -> bool {
    matches!(
        route,
        RootRoute::MusicTrackRequestDetail { .. }
            | RootRoute::MusicArtistRequestDetail { .. }
            | RootRoute::MusicAlbumRequestDetail { .. }
            | RootRoute::StudioTrackDetail { .. }
            | RootRoute::StudioAlbumDetail { .. }
    )
}

/// true for a music requests list or artist studio, picks open in the panel even with nothing playing
pub fn is_music_request_list(route: &RootRoute) -> bool {
    matches!(
        route,
        RootRoute::MusicTrackRequestsAdmin
            | RootRoute::MusicArtistRequestsAdmin
            | RootRoute::MusicAlbumRequestsAdmin
            | RootRoute::ArtistStudio
    )
}

/// true for a chat info page, meaning the chat or a forum topic or the general topic
pub fn is_chat_info(route: &RootRoute) -> bool {
    matches!(route, RootRoute::ChatProfile { .. } | RootRoute::GeneralProfile { .. } | RootRoute::TopicProfile { .. })
}

/// true for a tool page opened from the chat header (bot settings), shown beside the chat
pub fn is_chat_tool(route: &RootRoute) -> bool {
    matches!(route, RootRoute::BotSettings { .. } | RootRoute::ChatSettings { .. })
}

/// true for pages that belong in the panel by nature, so the panel row has no open in centre for them
pub fn lives_in_panel(route: &RootRoute) -> bool {
    is_chat_info(route)
        || is_chat_tool(route)
        || is_profile_people_list(route)
        || is_music_request_detail(route)
        || matches!(route, RootRoute::UserPage { .. } | RootRoute::MyProfile | RootRoute::Notifications)
}

/// true for a conversation page in the centre stack
/// chat, forum topic or thread
pub fn is_conversation(route: &RootRoute) -> bool {
    matches!(route, RootRoute::Chat { .. } | RootRoute::ChatTopic { .. } | RootRoute::ChatThread { .. })
}

/// what the panel is asked for, the section and the screen on top of it
#[derive(Clone, PartialEq, Debug)]
pub struct AsideScope {
    pub section: AsideSection,
    pub tab: MainTab,
    pub top_route: Option<RootRoute>,
}

impl AsideScope {
    pub fn current(tab: MainTab, top_route: Option<&RootRoute>) -> Option<Self> {
        AsideSection::of(tab, top_route).map(|section| Self { section, tab, top_route: top_route.cloned() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_group_settings_open_beside_the_conversation() {
        assert!(is_chat_tool(&RootRoute::ChatSettings { chat_id: Some("chat".to_owned()) }));
        assert!(!is_chat_tool(&RootRoute::ChatSettingsMembers { chat_id: Some("chat".to_owned()) }));
    }

    #[test]
    fn a_pushed_profile_is_the_profile_section_on_any_tab() {
        let route = RootRoute::MyProfile;
        assert_eq!(AsideSection::of(MainTab::Chats, Some(&route)), Some(AsideSection::Profile));
        assert_eq!(AsideSection::of(MainTab::Chats, None), Some(AsideSection::Chats));
    }

    #[test]
    fn a_pushed_conversation_is_the_chats_section_on_any_tab() {
        let chat = RootRoute::Chat { chat_id: Some("c".to_owned()) };
        assert_eq!(AsideSection::of(MainTab::Posts, Some(&chat)), Some(AsideSection::Chats));
        assert_eq!(AsideSection::of(MainTab::Music, Some(&RootRoute::ForumAll { chat_id: None })), Some(AsideSection::Chats));
        assert_eq!(AsideSection::of(MainTab::Posts, None), Some(AsideSection::Posts));
    }

    #[test]
    fn a_pushed_riff_is_the_riffs_section_on_any_tab() {
        let riff = RootRoute::ShortViewer { post: Default::default(), origin: None, sound_track_id: None };
        assert_eq!(AsideSection::of(MainTab::GlobalSearch, Some(&riff)), Some(AsideSection::Riffs));
        assert_eq!(AsideSection::of(MainTab::Posts, Some(&riff)), Some(AsideSection::Riffs));
        assert_eq!(AsideSection::of(MainTab::Profile, Some(&riff)), Some(AsideSection::Riffs));
    }

    #[test]
    fn a_profile_people_list_opens_beside_the_centre() {
        assert!(opens_beside_centre(&RootRoute::UserSubscribers { user_id: Some("u".to_owned()) }));
        assert!(opens_beside_centre(&RootRoute::UserSubs { user_id: None }));
        assert!(opens_beside_centre(&RootRoute::UserFriends { user_id: None }));
        assert!(!is_profile_people_list(&RootRoute::MyBots));
    }

    #[test]
    fn bot_settings_are_a_chat_tool_and_a_chat_is_a_conversation() {
        assert!(is_chat_tool(&RootRoute::BotSettings { bot_id: "b".to_owned() }));
        assert!(!is_chat_tool(&RootRoute::MyBots));
        assert!(is_conversation(&RootRoute::Chat { chat_id: Some("c".to_owned()) }));
        assert!(!is_conversation(&RootRoute::MyBots));
    }

    #[test]
    fn people_posts_and_chat_info_open_beside_the_centre() {
        assert!(opens_beside_centre(&RootRoute::UserPage { user_id: None, tag: None }));
        assert!(opens_beside_centre(&RootRoute::ChatProfile { chat_id: None }));
        assert!(opens_beside_centre(&RootRoute::TopicProfile { chat_id: None, topic_id: None }));
        assert!(opens_beside_centre(&RootRoute::GeneralProfile { chat_id: None }));
        assert!(!opens_beside_centre(&RootRoute::Chat { chat_id: None }));
        assert!(!opens_beside_centre(&RootRoute::MyProfile));
        assert!(opens_beside_centre(&RootRoute::MusicTrackRequestDetail { track_id: String::new() }));
        assert!(!opens_beside_centre(&RootRoute::MusicTrackRequestsAdmin));
    }

    #[test]
    fn panel_pages_drop_open_in_centre_and_a_post_keeps_it() {
        assert!(lives_in_panel(&RootRoute::Notifications));
        assert!(lives_in_panel(&RootRoute::UserFriends { user_id: Some("u1".to_owned()) }));
        assert!(!lives_in_panel(&RootRoute::PostDetail { post_id: Some("p1".to_owned()), origin: None }));
    }

    #[test]
    fn every_section_has_its_own_remembered_context() {
        let sections = AsideSection::ALL;
        for (index, section) in sections.iter().enumerate() {
            assert_eq!(AsideSection::from_context_key(section.context_key()), Some(*section));
            for other in &sections[index + 1..] {
                assert_ne!(section.context_key(), other.context_key());
            }
        }
    }
}
