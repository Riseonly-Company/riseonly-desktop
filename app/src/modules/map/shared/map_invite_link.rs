use rise_core::foundation::foundation_url::Url;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;

/// public form of a map invite
/// path comes from the canon route table and never a literal here
pub enum MapInviteLink {}

impl MapInviteLink {
    pub fn url(token: &str) -> Option<Url> {
        RiseonlyPublicLink::MapInvite { token: token.to_owned() }.web_url(AppEnvironment::current())
    }

    /// link text with the scheme stripped
    pub fn display_text(token: &str) -> String {
        let Some(url) = Self::url(token) else {
            return token.to_owned();
        };
        let text = url.absolute_string();
        match text.find("://") {
            Some(index) => text[index + "://".len()..].to_owned(),
            None => text.to_owned(),
        }
    }
}
