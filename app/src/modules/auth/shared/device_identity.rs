use rise_platform::{DeviceLocale, HostOs};

use crate::modules::auth::engine::core::rise_auth_rpc::DeviceEnvelope;

pub struct DeviceIdentity;

impl DeviceIdentity {
    pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    pub fn platform(host: HostOs) -> &'static str {
        match host {
            HostOs::MacOs => "macos",
            HostOs::Windows => "windows",
            HostOs::Linux => "linux",
        }
    }

    pub fn display_name(host: HostOs) -> String {
        format!("Riseonly Desktop ({})", host_label(host))
    }

    pub fn user_agent(host: HostOs) -> String {
        format!("Riseonly/{} ({}; desktop)", Self::VERSION, host_label(host))
    }

    pub fn envelope(host: HostOs, device_token: Option<String>) -> DeviceEnvelope {
        DeviceEnvelope {
            device_info: Self::display_name(host),
            device_type: "desktop".to_owned(),
            platform: Self::platform(host).to_owned(),
            user_agent: Self::user_agent(host),
            browser: false,
            device_token,
        }
    }

    pub fn current_envelope(device_token: Option<String>) -> DeviceEnvelope {
        Self::envelope(HostOs::current(), device_token)
    }

    pub fn locale_hints(locale: &DeviceLocale) -> (Option<String>, Vec<String>) {
        (locale.region.clone(), locale.languages.clone())
    }
}

fn host_label(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => "macOS",
        HostOs::Windows => "Windows",
        HostOs::Linux => "Linux",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_has_a_name_the_server_will_store() {
        for host in HostOs::ALL {
            let envelope = DeviceIdentity::envelope(host, None);
            assert!(!envelope.platform.is_empty());
            assert!(
                envelope.platform.len() <= 32,
                "the gateway caps platform at 32"
            );
            assert!(envelope.device_info.len() <= 1024);
            assert!(envelope.user_agent.len() <= 1024);
            assert_eq!(envelope.device_type, "desktop");
            assert!(!envelope.browser);
        }
    }

    #[test]
    fn the_three_platforms_are_distinguishable_in_a_session_list() {
        let names: Vec<String> = HostOs::ALL
            .iter()
            .map(|host| DeviceIdentity::display_name(*host))
            .collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn the_user_agent_carries_the_version_so_a_session_row_says_which_build() {
        assert!(DeviceIdentity::user_agent(HostOs::MacOs).contains(DeviceIdentity::VERSION));
    }

    #[test]
    fn an_unknown_locale_produces_no_hints_rather_than_an_invented_market() {
        let (region, languages) = DeviceIdentity::locale_hints(&DeviceLocale::default());
        assert_eq!(region, None);
        assert!(languages.is_empty());
    }

    #[test]
    fn a_known_locale_travels_intact() {
        let locale = DeviceLocale::parse(&["ru-KZ", "en"]);
        let (region, languages) = DeviceIdentity::locale_hints(&locale);
        assert_eq!(region.as_deref(), Some("KZ"));
        assert_eq!(languages, vec!["ru", "en"]);
    }
}
