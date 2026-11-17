use gpui::App;
use rise_core::foundation::foundation_url::{Url, UrlComponents, UrlQueryItem};

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::core::config::app_environment::AppEnvironment;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseLegalLinksDocument {
    TermsOfService,
    PrivacyPolicy,
    ChildSafety,
}

impl RiseLegalLinksDocument {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::TermsOfService => "terms-of-service",
            Self::PrivacyPolicy => "privacy-policy",
            Self::ChildSafety => "child-safety",
        }
    }

    fn tags_source(self) -> bool {
        self != Self::ChildSafety
    }
}

pub enum RiseLegalLinks {}

impl RiseLegalLinks {
    pub const SUPPORT_EMAIL: &'static str = "support@riseonly.net";

    const SOURCE_QUERY_NAME: &'static str = "source";
    const SOURCE_QUERY_VALUE: &'static str = "ios";

    pub fn url(document: RiseLegalLinksDocument) -> Option<Url> {
        let mut components = UrlComponents::from_url(&AppEnvironment::Prod.public_site_url(), false)?;
        components.set_path(&format!("/{}", document.raw_value()));
        if document.tags_source() {
            components.set_query_items(Some(vec![UrlQueryItem::new(
                Self::SOURCE_QUERY_NAME,
                Some(Self::SOURCE_QUERY_VALUE.to_owned()),
            )]));
        }
        components.url()
    }

    pub fn support_email_url() -> Option<Url> {
        Url::from_string(&format!("mailto:{}", Self::SUPPORT_EMAIL))
    }

    /// opens in the apps browser and mail in the mail app
    pub fn open(document: RiseLegalLinksDocument, cx: &mut App) {
        Self::open_url(Self::url(document), cx);
    }

    pub fn open_support_email(cx: &mut App) {
        Self::open_url(Self::support_email_url(), cx);
    }

    pub fn open_url(url: Option<Url>, cx: &mut App) {
        let Some(url) = url else { return };
        if InAppLinkBrowser::open(&url, cx) {
            return;
        }
        rise_platform::open_url::open_url(&url, cx);
    }
}
