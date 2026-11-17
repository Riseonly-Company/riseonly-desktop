use rise_canon::RiseValidation;
use rise_core::foundation::foundation_text::trimming_characters;
use rise_core::foundation::foundation_url::Url;
use rise_validate::{
    DEVELOPMENT_HOST_EXACT, DEVELOPMENT_HOST_SUFFIXES, DEVELOPMENT_IPV4_BLOCKS, DEVELOPMENT_IPV6_RULES, UrlPolicy,
};

use crate::core::lib::canon_rules::RiseValidationCanonRules;

pub enum BotMiniAppURLPolicy {}

impl BotMiniAppURLPolicy {
    const DEVELOPMENT_POLICY: &'static str = "mini_app_development";
    const PRODUCTION_POLICY: &'static str = "mini_app_production";
    const HTTP_SCHEME: &'static str = "http";

    pub fn allows(url: &Url, allows_development_http: bool) -> bool {
        Self::allows_policy(
            url,
            &RiseValidation::url_policy(if allows_development_http {
                Self::DEVELOPMENT_POLICY
            } else {
                Self::PRODUCTION_POLICY
            }),
        )
    }

    /// url parsing is the platform job, the policy only sees scheme and host and credentials
    /// plain http is allowed only to a development host, a public host over http never passes
    pub fn allows_policy(url: &Url, policy: &UrlPolicy) -> bool {
        if !policy.allow_credentials && (url.user().is_some() || url.password().is_some()) {
            return false;
        }
        if url.absolute_string().len() > policy.max_length_bytes {
            return false;
        }
        let Some(host) = url.host().map(|host| host.to_lowercase()).filter(|host| !host.is_empty()) else {
            return !policy.require_host;
        };

        let scheme = url.scheme().map(str::to_lowercase).unwrap_or_default();
        if policy.secure_schemes.contains(&scheme.as_str()) {
            return policy.allow_development_hosts || !Self::is_development_host(&host);
        }
        if policy.insecure_schemes.contains(&scheme.as_str()) {
            return Self::is_development_host(&host);
        }
        false
    }

    pub fn is_development_host(raw_host: &str) -> bool {
        let lowered = raw_host.to_lowercase();
        let host = trimming_characters(&lowered, |character| matches!(character, '.' | '[' | ']'));
        if DEVELOPMENT_HOST_EXACT.contains(&host) {
            return true;
        }
        if DEVELOPMENT_HOST_SUFFIXES.iter().any(|suffix| host.ends_with(suffix)) {
            return true;
        }
        if host.contains(':') { Self::is_development_ipv6(host) } else { Self::is_development_ipv4(host) }
    }

    fn is_development_ipv6(host: &str) -> bool {
        let groups: Vec<&str> = host.split(':').collect();
        let first = groups.first().and_then(|group| u32::from_str_radix(group, 16).ok()).unwrap_or(0);
        let second = groups.get(1).and_then(|group| u32::from_str_radix(group, 16).ok()).unwrap_or(0);
        DEVELOPMENT_IPV6_RULES.iter().any(|(first_mask, first_value, second_mask, second_value)| {
            (first & u32::from(*first_mask)) == u32::from(*first_value)
                && (second & u32::from(*second_mask)) == u32::from(*second_value)
        })
    }

    fn is_development_ipv4(host: &str) -> bool {
        let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
        if labels.len() != 4 {
            return false;
        }
        let octets: Vec<u32> = labels.iter().filter_map(|label| label.parse::<u32>().ok()).collect();
        if octets.len() != 4 || !octets.iter().all(|octet| *octet <= 255) {
            return false;
        }

        let address = (octets[0] << 24) | (octets[1] << 16) | (octets[2] << 8) | octets[3];
        DEVELOPMENT_IPV4_BLOCKS.iter().any(|(base, bits)| {
            let mask: u32 = if *bits == 0 { 0 } else { !0u32 << (32 - bits) };
            (address & mask) == *base
        })
    }

    pub fn allows_development_http(url: &Url) -> bool {
        if url.scheme().map(str::to_lowercase).as_deref() != Some(Self::HTTP_SCHEME) {
            return false;
        }
        let Some(host) = url.host() else {
            return false;
        };
        Self::is_development_host(&host)
    }
}
