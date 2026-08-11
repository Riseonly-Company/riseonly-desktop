use crate::pane_layout::PaneSlot;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RootTab {
    Feed,
    Search,
    Shorts,
    Chats,
    Vacancies,
    Music,
    Profile,
}

impl RootTab {
    /// Presentation order. The left rail renders this array, so it is also the
    /// order of Cmd+1..n.
    pub const ALL: [RootTab; 7] = [
        RootTab::Feed,
        RootTab::Search,
        RootTab::Chats,
        RootTab::Shorts,
        RootTab::Vacancies,
        RootTab::Music,
        RootTab::Profile,
    ];

    /// The sections the rail stacks at the TOP, in order: everything but
    /// [`RootTab::FOOTER`].
    pub fn primary() -> &'static [RootTab] {
        &Self::ALL[..Self::ALL.len() - 1]
    }

    /// The one section the rail pins to the bottom.
    pub const FOOTER: RootTab = RootTab::Profile;

    pub fn screen_id(self) -> &'static str {
        match self {
            Self::Feed => "Feed",
            Self::Search => "Search",
            Self::Shorts => "Shorts",
            Self::Chats => "Chats",
            Self::Vacancies => "Vacancies",
            Self::Music => "Music",
            Self::Profile => "Profile",
        }
    }

    /// Whether this section opens with a LIST beside its content. True only for
    /// sections you browse then open; a feed or player IS the content.
    pub fn has_sidebar_list(self) -> bool {
        matches!(self, Self::Chats | Self::Search)
    }

    /// Position in `ALL`, which is what the per-tab stacks and Cmd+1..n index by.
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|tab| *tab == self)
            .expect("RootTab::ALL is missing a variant")
    }
}

/// Every screen the shell can show.
///
/// Variants carry ids and parameters only, never module types, so this enum
/// stays free of any feature crate.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RootRoute {
    Onboarding,
    SignIn,
    SignUp,
    Tab(RootTab),
    Chat { chat_id: String },
    ChatTopic { chat_id: String, topic_id: String },
    UserProfile { user_id: String },
    Post { post_id: String },
    Settings,
}

impl RootRoute {
    pub fn screen_id(&self) -> &'static str {
        match self {
            Self::Onboarding => "Onboarding",
            Self::SignIn => "SignIn",
            Self::SignUp => "SignUp",
            Self::Tab(tab) => tab.screen_id(),
            Self::Chat { .. } => "Chat",
            Self::ChatTopic { .. } => "ChatTopics",
            Self::UserProfile { .. } => "UserProfile",
            Self::Post { .. } => "Post",
            Self::Settings => "Settings",
        }
    }

    /// The key the screen's data is cached and revalidated under. Independent of
    /// which pane shows it, so two panes on one conversation are one resource.
    pub fn resource_key(&self) -> String {
        match self {
            Self::Onboarding | Self::SignIn | Self::SignUp | Self::Settings => {
                self.screen_id().to_owned()
            }
            Self::Tab(tab) => tab.screen_id().to_owned(),
            Self::Chat { chat_id } => format!("Chat:{chat_id}"),
            Self::ChatTopic { chat_id, topic_id } => format!("ChatTopics:{chat_id}:{topic_id}"),
            Self::UserProfile { user_id } => format!("UserProfile:{user_id}"),
            Self::Post { post_id } => format!("Post:{post_id}"),
        }
    }

    pub fn slot(&self) -> PaneSlot {
        match self {
            Self::Onboarding | Self::SignIn | Self::SignUp => PaneSlot::Primary,
            Self::Tab(_) => PaneSlot::Primary,
            Self::Chat { .. } | Self::ChatTopic { .. } | Self::Post { .. } => PaneSlot::Secondary,
            Self::UserProfile { .. } | Self::Settings => PaneSlot::Tertiary,
        }
    }

    pub fn requires_authentication(&self) -> bool {
        !matches!(self, Self::Onboarding | Self::SignIn | Self::SignUp)
    }

    /// Whether dropping files onto this screen means anything.
    pub fn accepts_files(&self) -> bool {
        matches!(self, Self::Chat { .. } | Self::ChatTopic { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn every_route() -> Vec<RootRoute> {
        let mut routes = vec![
            RootRoute::Onboarding,
            RootRoute::SignIn,
            RootRoute::SignUp,
            RootRoute::Chat {
                chat_id: "c1".into(),
            },
            RootRoute::ChatTopic {
                chat_id: "c1".into(),
                topic_id: "t1".into(),
            },
            RootRoute::UserProfile {
                user_id: "u1".into(),
            },
            RootRoute::Post {
                post_id: "p1".into(),
            },
            RootRoute::Settings,
        ];
        routes.extend(RootTab::ALL.map(RootRoute::Tab));
        routes
    }

    #[test]
    fn every_route_has_a_screen_id() {
        for route in every_route() {
            assert!(!route.screen_id().is_empty(), "{route:?} has no screen id");
        }
    }

    #[test]
    fn screen_ids_are_unique_per_destination() {
        let ids: HashSet<&str> = every_route().iter().map(|r| r.screen_id()).collect();
        assert_eq!(
            ids.len(),
            every_route().len(),
            "two routes share a screen id, which would merge their analytics and refresh keys"
        );
    }

    #[test]
    fn only_the_pre_auth_routes_are_reachable_while_signed_out() {
        let open: Vec<_> = every_route()
            .into_iter()
            .filter(|r| !r.requires_authentication())
            .collect();
        assert_eq!(
            open,
            vec![RootRoute::Onboarding, RootRoute::SignIn, RootRoute::SignUp]
        );
    }

    #[test]
    fn the_same_screen_with_different_parameters_is_a_different_resource() {
        let one = RootRoute::Chat {
            chat_id: "c1".into(),
        };
        let two = RootRoute::Chat {
            chat_id: "c2".into(),
        };
        assert_ne!(one.resource_key(), two.resource_key());
        assert_eq!(one.resource_key(), one.clone().resource_key());
    }

    #[test]
    fn every_resource_key_starts_with_its_screen_id() {
        for route in every_route() {
            assert!(
                route.resource_key().starts_with(route.screen_id()),
                "{route:?} keys its cache under a name its screen id cannot be read from"
            );
        }
    }

    #[test]
    fn resource_keys_are_unique_across_distinct_screens() {
        let keys: HashSet<String> = every_route().iter().map(RootRoute::resource_key).collect();
        assert_eq!(
            keys.len(),
            every_route().len(),
            "two screens sharing a key would make one of them revalidate the other's data"
        );
    }

    #[test]
    fn a_tab_index_addresses_the_section_the_shortcut_names() {
        for (index, tab) in RootTab::ALL.iter().enumerate() {
            assert_eq!(tab.index(), index);
        }
    }

    #[test]
    fn tabs_and_auth_screens_own_the_primary_column() {
        assert_eq!(RootRoute::SignIn.slot(), PaneSlot::Primary);
        for tab in RootTab::ALL {
            assert_eq!(RootRoute::Tab(tab).slot(), PaneSlot::Primary);
        }
    }

    #[test]
    fn content_screens_target_the_secondary_column() {
        assert_eq!(
            RootRoute::Chat {
                chat_id: "c".into()
            }
            .slot(),
            PaneSlot::Secondary
        );
    }
}
