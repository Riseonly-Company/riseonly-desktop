use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};

/// web admin page named by an admin only notification
/// address comes in the payload so its checked before opening
/// https and admin host and no credentials and no port, else opens nothing
pub enum AdminConsoleLink {}

impl AdminConsoleLink {
    pub const PAYLOAD_KEY: &'static str = "admin_url";
    pub const ALERT_TYPE: &'static str = "admin_alert";
    pub const INCIDENT_TYPE: &'static str = "security_incident";

    pub fn url(from: &str) -> Option<Url> {
        let raw = trimming_whitespaces_and_newlines(from);
        if raw.is_empty() || !raw.chars().all(Self::is_plain_ascii) {
            return None;
        }
        let components = UrlComponents::from_string(raw)?;
        if components.scheme().map(str::to_lowercase).as_deref() != Some("https")
            || components.user().is_some()
            || components.password().is_some()
            || components.port().is_some()
        {
            return None;
        }
        let host = components.percent_encoded_host()?;
        if !Self::is_admin_host(&host.to_lowercase()) {
            return None;
        }
        components.url()
    }

    pub fn is_admin_host(host: &str) -> bool {
        let labels: Vec<&str> = host.split('.').collect();
        let count = labels.len();
        if !(count == 3 || count == 4)
            || labels[0] != "admin"
            || labels[count - 2] != "riseonly"
            || labels[count - 1] != "net"
        {
            return false;
        }
        count == 3 || Self::is_host_label(labels[1])
    }

    fn is_host_label(label: &str) -> bool {
        if !(1..=63).contains(&label.chars().count())
            || label.starts_with('-')
            || label.ends_with('-')
        {
            return false;
        }
        label
            .chars()
            .all(|scalar| matches!(scalar, 'a'..='z' | '0'..='9' | '-'))
    }

    fn is_plain_ascii(scalar: char) -> bool {
        (0x21..=0x7E).contains(&u32::from(scalar)) && scalar != '\\'
    }
}
