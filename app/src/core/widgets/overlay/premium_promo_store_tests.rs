use rise_core::foundation::foundation_url::Url;
use rise_i18n::{FormatArg, tr, tr_plural_with};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::core::widgets::overlay::premium_promo_store::PremiumPromoStore;

fn url(string: &str) -> Url {
    Url::from_string(string).unwrap_or_else(|| panic!("test URL must parse: {string}"))
}

fn response_error(message: &str, code: &str) -> anyhow::Error {
    anyhow::Error::new(SaiWsResponseError {
        message: message.to_owned(),
        code: Some(code.to_owned()),
        ..SaiWsResponseError::default()
    })
}

#[test]
fn short_token_and_aliases_open_the_same_promo_sheet() {
    let code = "launch2026";
    let short = url(&["https://riseonly.net/g/", code].concat());
    let gift = url(&["https://riseonly.net/gift/", code].concat());
    let promo = url(&["https://riseonly.net/promo/", code, "?utm_source=tg"].concat());
    let custom = url(&["riseonly://g/", code].concat());

    for link in [short, gift, promo, custom] {
        assert_eq!(
            UniversalLinkParser::parse(&link, AppEnvironment::Prod),
            Some(PushDeepLink::PremiumPromo { code: code.to_owned() })
        );
    }
}

#[test]
fn the_code_is_lower_cased_like_the_server_compares() {
    let link = url("https://riseonly.net/g/Launch-2026");
    assert_eq!(
        UniversalLinkParser::parse(&link, AppEnvironment::Prod),
        Some(PushDeepLink::PremiumPromo { code: "launch-2026".to_owned() })
    );
}

#[test]
fn malformed_promo_links_are_rejected() {
    assert_eq!(
        UniversalLinkParser::parse(&url("https://riseonly.net/g"), AppEnvironment::Prod),
        Some(PushDeepLink::PublicTag { tag: "g".to_owned() })
    );

    let links = [
        "https://riseonly.net/g/",
        "https://riseonly.net/g/code/extra",
        "https://riseonly.net/g/bad+code",
        "https://riseonly.net/g/code%2Fslash",
        "https://example.com/g/launch2026",
    ]
    .map(url);

    for link in links {
        assert_eq!(UniversalLinkParser::parse(&link, AppEnvironment::Prod), None);
    }
}

#[test]
fn the_printed_link_uses_the_short_token() {
    let link = RiseonlyPublicLink::PremiumPromo { code: "launch2026".to_owned() };
    assert_eq!(
        link.web_url(AppEnvironment::Prod).map(|url| url.absolute_string().to_owned()),
        Some("https://riseonly.net/g/launch2026".to_owned())
    );
    assert_eq!(
        link.app_url(AppEnvironment::Prod).map(|url| url.absolute_string().to_owned()),
        Some("riseonly://g/launch2026".to_owned())
    );
}

#[test]
fn a_wire_refusal_code_maps_back_to_the_service_vocabulary() {
    let error = response_error("taken", "PREMIUM_PROMO_ALREADY_REDEEMED");
    assert_eq!(PremiumPromoStore::refusal(&error), Some("already_redeemed".to_owned()));
    assert_eq!(PremiumPromoStore::refusal(&response_error("x", "GRPC_ERROR")), None);
}

#[test]
fn the_grant_period_prefers_months_for_round_grants() {
    assert_eq!(PremiumPromoStore::period_text_for(0, true), tr("premium_promo_period_lifetime"));
    assert_eq!(
        PremiumPromoStore::period_text_for(90, false),
        tr_plural_with("premium_gift_months", 3, FormatArg::from(3_i64))
    );
    assert_eq!(
        PremiumPromoStore::period_text_for(365, false),
        tr_plural_with("premium_gift_months", 12, FormatArg::from(12_i64))
    );
    assert_eq!(
        PremiumPromoStore::period_text_for(14, false),
        tr_plural_with("premium_promo_period_days", 14, FormatArg::from(14_i64))
    );
}
