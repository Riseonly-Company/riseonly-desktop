use rise_core::foundation::foundation_url::Url;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::modules::bot::shared::bot_start_payload::BotStartPayload;

fn parse(string: &str) -> Option<PushDeepLink> {
    let url = Url::from_string(string).expect("valid url");
    UniversalLinkParser::parse(&url, AppEnvironment::Prod)
}

fn bot_chat(start_param: Option<&str>) -> Option<PushDeepLink> {
    Some(PushDeepLink::BotChat { tag: "anonbot".to_owned(), start_param: start_param.map(str::to_owned) })
}

#[test]
fn preserves_payload_across_web_and_app_links() {
    for prefix in ["c", "b", "chat"] {
        for base in ["https://riseonly.net/", "riseonly://"] {
            assert_eq!(parse(&format!("{base}{prefix}/anonbot?start=4jzK82pQxsM")), bot_chat(Some("4jzK82pQxsM")));
            assert_eq!(parse(&format!("{base}{prefix}/anonbot")), bot_chat(None));
        }
    }
}

#[test]
fn rejects_malformed_payloads_and_keeps_mini_apps_separate() {
    let oversized = "a".repeat(65);
    let payloads = [
        ("?start=".to_owned(), String::new()),
        ("?start=a%20b".to_owned(), "a b".to_owned()),
        ("?start=a%0Astop".to_owned(), "a\nstop".to_owned()),
        (format!("?start={oversized}"), oversized.clone()),
    ];
    for prefix in ["c", "b", "chat"] {
        for suffix in ["?start=a&start=b", "?other=a", "?start=a#b"] {
            assert_eq!(parse(&format!("https://riseonly.net/{prefix}/anonbot{suffix}")), None);
        }
        for (suffix, payload) in &payloads {
            assert_eq!(parse(&format!("https://riseonly.net/{prefix}/anonbot{suffix}")), bot_chat(Some(payload)));
            assert!(!BotStartPayload::is_valid(payload));
        }
    }
    assert_eq!(
        parse("https://riseonly.net/b/anonbot/app?startapp=Ab_9-z"),
        Some(PushDeepLink::BotMiniApp {
            tag: "anonbot".to_owned(),
            slug: "app".to_owned(),
            start_param: Some("Ab_9-z".to_owned()),
        })
    );
    assert!(BotStartPayload::is_valid(&"a".repeat(64)));
    assert!(!BotStartPayload::is_valid("\u{0442}\u{0435}\u{0441}\u{0442}"));
}

#[test]
fn preserves_ordinary_chat_and_topic_links() {
    let chat_id = "af6ccda2-1bc9-425e-8439-e60a82de0aa1";
    for prefix in ["c", "chat"] {
        assert_eq!(
            parse(&format!("https://riseonly.net/{prefix}/{chat_id}?utm_source=share")),
            Some(PushDeepLink::Chat { chat_id: chat_id.to_owned() })
        );
        assert_eq!(
            parse(&format!("https://riseonly.net/{prefix}/{chat_id}/topic/5")),
            Some(PushDeepLink::ChatTopic { chat_id: chat_id.to_owned(), topic_id: "5".to_owned() })
        );
    }
}
