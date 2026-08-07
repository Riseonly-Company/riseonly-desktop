use rise_navigation::{RootRoute, RootTab};
use rise_platform::DeepLink;

/// Turns a link the OS handed us into a screen.
///
/// `rise-platform` parses; this decides. Keeping the two apart is what lets the
/// platform crate stay ignorant of what a chat is, and what makes every case
/// here testable without an OS event.
///
/// An unknown target is `None` rather than a guess: a link the running version
/// does not understand must leave the user where they were, not somewhere
/// arbitrary.
pub fn route_for(link: &DeepLink) -> Option<RootRoute> {
    match link.target.as_str() {
        "chat" => {
            let chat_id = link.segment(1)?.to_owned();
            match link.segment(2) {
                Some("topic") => link.segment(3).map(|topic_id| RootRoute::ChatTopic {
                    chat_id,
                    topic_id: topic_id.to_owned(),
                }),
                _ => Some(RootRoute::Chat { chat_id }),
            }
        }
        "user" => Some(RootRoute::UserProfile {
            user_id: link.segment(1)?.to_owned(),
        }),
        "post" => Some(RootRoute::Post {
            post_id: link.segment(1)?.to_owned(),
        }),
        "settings" => Some(RootRoute::Settings),
        "posts" | "feed" => Some(RootRoute::Tab(RootTab::Feed)),
        "search" => Some(RootRoute::Tab(RootTab::Search)),
        "chats" => Some(RootRoute::Tab(RootTab::Chats)),
        "shorts" => Some(RootRoute::Tab(RootTab::Shorts)),
        "more" | "profile" => Some(RootRoute::Tab(RootTab::Profile)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEME: &str = "riseonly";
    const HOST: &str = "riseonly.net";

    fn route(raw: &str) -> Option<RootRoute> {
        let link = DeepLink::parse(raw, SCHEME, HOST).expect("well-formed link");
        route_for(&link)
    }

    #[test]
    fn a_conversation_link_opens_that_conversation() {
        assert_eq!(
            route("riseonly://chat/42"),
            Some(RootRoute::Chat {
                chat_id: "42".into()
            })
        );
    }

    #[test]
    fn a_topic_link_opens_the_topic_rather_than_its_chat() {
        assert_eq!(
            route("riseonly://chat/42/topic/7"),
            Some(RootRoute::ChatTopic {
                chat_id: "42".into(),
                topic_id: "7".into(),
            })
        );
    }

    #[test]
    fn a_web_link_reaches_the_same_screen_as_the_scheme() {
        assert_eq!(
            route("https://riseonly.net/post/9"),
            route("riseonly://post/9")
        );
    }

    #[test]
    fn every_section_is_reachable_by_name() {
        assert_eq!(
            route("riseonly://posts"),
            Some(RootRoute::Tab(RootTab::Feed))
        );
        assert_eq!(
            route("riseonly://chats"),
            Some(RootRoute::Tab(RootTab::Chats))
        );
        assert_eq!(
            route("riseonly://shorts"),
            Some(RootRoute::Tab(RootTab::Shorts))
        );
        assert_eq!(
            route("riseonly://search"),
            Some(RootRoute::Tab(RootTab::Search))
        );
        assert_eq!(
            route("riseonly://more"),
            Some(RootRoute::Tab(RootTab::Profile))
        );
    }

    #[test]
    fn a_link_missing_its_identifier_opens_nothing_rather_than_an_empty_screen() {
        assert_eq!(route("riseonly://chat"), None);
        assert_eq!(route("riseonly://user"), None);
        assert_eq!(route("riseonly://chat/42/topic"), None);
    }

    #[test]
    fn a_target_this_version_has_never_heard_of_is_ignored() {
        assert_eq!(route("riseonly://vacancies/12"), None);
    }

    #[test]
    fn a_percent_encoded_identifier_survives_the_trip() {
        assert_eq!(
            route("riseonly://user/%D0%B0%D0%BD%D1%8F"),
            Some(RootRoute::UserProfile {
                user_id: "аня".into()
            })
        );
    }
}
