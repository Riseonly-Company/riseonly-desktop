use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_deeplink::{Environment, Link, environment_for_url_scheme, split_path};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DeepLinkError {
    #[error("not a URL")]
    Malformed,
    #[error("scheme {0} does not belong to this build")]
    ForeignScheme(String),
    #[error("host {0} does not belong to this build")]
    ForeignHost(String),
    #[error("no destination in the link")]
    Empty,
}

/// a link from the os, split and decoded but not routed yet
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeepLink {
    pub target: String,
    pub segments: Vec<String>,
    pub query: Vec<(String, String)>,
    pub fragment: Option<String>,
}

impl DeepLink {
    /// parses a link for the build whose app scheme is url_scheme
    /// the web host comes from the same canon environment so a caller cant pass a host that
    /// doesnt match its scheme
    pub fn parse(raw: &str, url_scheme: &str) -> Result<Self, DeepLinkError> {
        let environment = environment_for_url_scheme(url_scheme)
            .ok_or_else(|| DeepLinkError::ForeignScheme(url_scheme.to_owned()))?;
        Self::parse_in(raw, environment)
    }

    /// accepts riseonly://chat/42?message=7 and for shared web urls https://riseonly.net/chat/42
    /// each only for its own environment
    pub fn parse_in(raw: &str, environment: &Environment) -> Result<Self, DeepLinkError> {
        let raw = trimming_whitespaces_and_newlines(raw);
        let (scheme, rest) = raw.split_once("://").ok_or(DeepLinkError::Malformed)?;
        if scheme.is_empty() {
            return Err(DeepLinkError::Malformed);
        }

        let (rest, fragment) = match rest.split_once('#') {
            Some((rest, fragment)) => (rest, Some(fragment.to_owned())),
            None => (rest, None),
        };
        let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));

        if authority.contains('@') || authority.contains(':') {
            return Err(DeepLinkError::Malformed);
        }

        let is_app_link = environment.url_scheme.eq_ignore_ascii_case(scheme);
        let mut segments = Vec::new();
        if is_app_link {
            if authority.is_empty() {
                return Err(DeepLinkError::Empty);
            }
            segments.push(decode(authority)?.to_lowercase());
        } else if environment.web_scheme.eq_ignore_ascii_case(scheme) {
            if !environment.accepts_web_host(authority) {
                return Err(DeepLinkError::ForeignHost(authority.to_owned()));
            }
        } else {
            return Err(DeepLinkError::ForeignScheme(scheme.to_owned()));
        }

        for encoded in split_path(path).map_err(|_| DeepLinkError::Malformed)? {
            segments.push(decode(encoded)?);
        }

        let target = segments
            .first()
            .map(|token| token.to_lowercase())
            .ok_or(DeepLinkError::Empty)?;

        Ok(Self {
            target,
            segments,
            query: query_pairs(query)?,
            fragment,
        })
    }

    /// routes by the canon path form
    /// values arrive decoded but unvalidated
    /// alphabet and length checks belong to the screen that opens the route
    pub fn route(&self) -> Result<Link, rise_deeplink::DeepLinkError> {
        rise_deeplink::parse(&self.segments, &self.query)
    }

    pub fn segment(&self, index: usize) -> Option<&str> {
        self.segments.get(index).map(String::as_str)
    }

    pub fn parameter(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

fn query_pairs(query: &str) -> Result<Vec<(String, String)>, DeepLinkError> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Ok((decode(key)?, decode(value)?))
        })
        .collect()
}

/// strict percent decoding, a broken escape or invalid utf8 refuses the whole link
/// a plus stays a plus
fn decode(raw: &str) -> Result<String, DeepLinkError> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'%' {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        let byte = bytes
            .get(index + 1..index + 3)
            .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .ok_or(DeepLinkError::Malformed)?;
        out.push(byte);
        index += 3;
    }

    String::from_utf8(out).map_err(|_| DeepLinkError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_deeplink::RouteKind;

    fn parse(raw: &str) -> Result<DeepLink, DeepLinkError> {
        DeepLink::parse(raw, "riseonly")
    }

    #[test]
    fn a_percent_in_front_of_a_multi_byte_character_is_refused_without_a_panic() {
        for raw in [
            "riseonly://chat/%a\u{e9}",
            "riseonly://chat/%z\u{434}",
            "riseonly://user/%\u{e9}",
            "riseonly://post/100%\u{1f642}",
            "riseonly://chat/%",
            "riseonly://chat/%f",
        ] {
            assert_eq!(parse(raw), Err(DeepLinkError::Malformed), "{raw}");
        }
    }

    #[test]
    fn a_malformed_escape_refuses_the_link_as_ios_does() {
        assert_eq!(parse("riseonly://chat/%zz"), Err(DeepLinkError::Malformed));
        assert_eq!(parse("riseonly://chat/%+f"), Err(DeepLinkError::Malformed));
        assert_eq!(parse("riseonly://chat/1?q=%zz"), Err(DeepLinkError::Malformed));
    }

    #[test]
    fn an_escape_at_the_very_end_is_decoded() {
        let link = parse("riseonly://search/a%20").unwrap();
        assert_eq!(link.segment(1), Some("a "));
    }

    #[test]
    fn a_custom_scheme_link_parses_into_target_and_segments() {
        let link = parse("riseonly://chat/42").unwrap();
        assert_eq!(link.target, "chat");
        assert_eq!(link.segment(1), Some("42"));
    }

    #[test]
    fn query_parameters_are_available_by_name() {
        let link = parse("riseonly://chat/42?message=7&from=push").unwrap();
        assert_eq!(link.parameter("message"), Some("7"));
        assert_eq!(link.parameter("from"), Some("push"));
        assert_eq!(link.parameter("absent"), None);
    }

    #[test]
    fn a_web_url_on_our_host_is_accepted_for_shared_links() {
        let link = parse("https://riseonly.net/post/99").unwrap();
        assert_eq!(link.target, "post");
        assert_eq!(link.segment(1), Some("99"));
    }

    #[test]
    fn the_www_alias_of_production_is_accepted() {
        assert!(parse("https://www.riseonly.net/p/99").is_ok());
    }

    #[test]
    fn another_products_scheme_is_refused() {
        assert_eq!(
            parse("otherapp://chat/42"),
            Err(DeepLinkError::ForeignScheme("otherapp".into()))
        );
    }

    #[test]
    fn a_scheme_outside_canon_is_no_build_at_all() {
        assert_eq!(
            DeepLink::parse("otherapp://chat/42", "otherapp"),
            Err(DeepLinkError::ForeignScheme("otherapp".into()))
        );
    }

    #[test]
    fn a_staging_build_does_not_swallow_a_production_link() {
        let staging = DeepLink::parse("riseonly://chat/42", "riseonly-staging");
        assert!(
            matches!(staging, Err(DeepLinkError::ForeignScheme(_))),
            "each environment owns its own scheme for exactly this reason"
        );
        assert_eq!(
            DeepLink::parse("https://riseonly.net/chat/42", "riseonly-staging"),
            Err(DeepLinkError::ForeignHost("riseonly.net".into()))
        );
    }

    #[test]
    fn a_web_url_on_a_foreign_host_is_refused() {
        assert_eq!(
            parse("https://evil.example/chat/42"),
            Err(DeepLinkError::ForeignHost("evil.example".into()))
        );
    }

    #[test]
    fn user_info_and_ports_are_refused_like_on_ios() {
        assert_eq!(
            parse("https://riseonly.net:443/p/1"),
            Err(DeepLinkError::Malformed)
        );
        assert_eq!(
            parse("https://me@riseonly.net/p/1"),
            Err(DeepLinkError::Malformed)
        );
    }

    #[test]
    fn percent_encoding_is_decoded_and_plus_is_kept() {
        let link = parse("riseonly://search/hello%20world?q=a+b").unwrap();
        assert_eq!(link.segment(1), Some("hello world"));
        assert_eq!(link.parameter("q"), Some("a+b"));
    }

    #[test]
    fn an_encoded_slash_stays_inside_its_segment() {
        let link = parse("riseonly://search/a%2Fb").unwrap();
        assert_eq!(link.segments, vec!["search", "a/b"]);
    }

    #[test]
    fn cyrillic_survives_percent_encoding() {
        let link = parse("riseonly://search/%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82").unwrap();
        assert_eq!(
            link.segment(1),
            Some("\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}")
        );
    }

    #[test]
    fn malformed_input_is_rejected_rather_than_guessed() {
        for raw in ["", "not a url", "riseonly:/chat", "://chat", "riseonly://"] {
            assert!(parse(raw).is_err(), "{raw} should not parse");
        }
    }

    #[test]
    fn a_scheme_with_no_path_has_no_destination() {
        assert_eq!(parse("riseonly:///"), Err(DeepLinkError::Empty));
        assert_eq!(parse("https://riseonly.net"), Err(DeepLinkError::Empty));
    }

    #[test]
    fn trailing_and_repeated_slashes_refuse_the_link_as_canon_says() {
        assert_eq!(parse("riseonly://chat//42"), Err(DeepLinkError::Malformed));
        assert_eq!(parse("riseonly://chat/42/"), Err(DeepLinkError::Malformed));
    }

    #[test]
    fn the_scheme_and_host_are_matched_case_insensitively() {
        let link = parse("RISEONLY://Chat/42").unwrap();
        assert_eq!(link.target, "chat");
        assert!(parse("HTTPS://RISEONLY.NET/chat/42").is_ok());
    }

    #[test]
    fn the_fragment_is_kept_apart_from_path_and_query() {
        let link = parse("https://riseonly.net/p/123?x=1#comments").unwrap();
        assert_eq!(link.segments, vec!["p", "123"]);
        assert_eq!(link.parameter("x"), Some("1"));
        assert_eq!(link.fragment.as_deref(), Some("comments"));
    }

    #[test]
    fn the_route_comes_from_canon_aliases_included() {
        let short = parse("riseonly://p/9").unwrap().route().unwrap();
        let long = parse("https://riseonly.net/post/9").unwrap().route().unwrap();
        assert_eq!(short.kind(), RouteKind::Post);
        assert_eq!(short, long);
    }
}
