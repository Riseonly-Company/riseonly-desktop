use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DeepLinkError {
    #[error("not a URL")]
    Malformed,
    #[error("scheme {0} does not belong to this build")]
    ForeignScheme(String),
    #[error("no destination in the link")]
    Empty,
}

/// A link the OS handed us, parsed but not yet routed: it carries a target and
/// parameters, and the app decides what screen that means.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeepLink {
    pub target: String,
    pub segments: Vec<String>,
    pub query: Vec<(String, String)>,
}

impl DeepLink {
    /// Accepts `riseonly://chat/42?message=7` and, for links shared as web URLs,
    /// `https://riseonly.net/chat/42`.
    ///
    /// `expected_scheme` and `web_host` are per-environment: without them a staging
    /// build installed beside production would swallow production links.
    pub fn parse(raw: &str, expected_scheme: &str, web_host: &str) -> Result<Self, DeepLinkError> {
        let raw = raw.trim();
        let (scheme, rest) = raw.split_once("://").ok_or(DeepLinkError::Malformed)?;

        if scheme.is_empty() || rest.is_empty() {
            return Err(DeepLinkError::Malformed);
        }

        // Schemes are case-insensitive per RFC 3986, and a Windows registry
        // handler does hand over upper-case ones.
        let lowered = scheme.to_ascii_lowercase();
        let path = match lowered.as_str() {
            s if s.eq_ignore_ascii_case(expected_scheme) => rest,
            "http" | "https" => {
                let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
                if !host.eq_ignore_ascii_case(web_host) {
                    return Err(DeepLinkError::ForeignScheme(host.to_owned()));
                }
                path
            }
            _ => return Err(DeepLinkError::ForeignScheme(scheme.to_owned())),
        };

        let (path, query) = path.split_once('?').unwrap_or((path, ""));

        let segments: Vec<String> = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(decode)
            .collect();

        let target = segments.first().cloned().ok_or(DeepLinkError::Empty)?;

        let query = query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .filter_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                Some((decode(key), decode(value)))
            })
            .collect();

        Ok(Self {
            target,
            segments,
            query,
        })
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

fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            // Sliced as BYTES, never as `&str`: `%` before a multi-byte character puts a
            // `&str` slice boundary inside that character and panics on attacker-shaped argv.
            b'%' if index + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[index + 1..index + 3])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                {
                    Some(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    None => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percent_in_front_of_a_multi_byte_character_does_not_panic() {
        // Each of these ends the two-byte hex window inside a character.
        for raw in [
            "riseonly://chat/%aé",
            "riseonly://chat/%zд",
            "riseonly://user/%é",
            "riseonly://post/100%🙂",
            "riseonly://chat/%",
            "riseonly://chat/%f",
        ] {
            let parsed = DeepLink::parse(raw, "riseonly", "riseonly.net");
            assert!(parsed.is_ok(), "{raw} failed to parse at all");
        }
    }

    #[test]
    fn a_malformed_escape_is_kept_verbatim_rather_than_dropped() {
        let link = DeepLink::parse("riseonly://chat/%zz", "riseonly", "riseonly.net").unwrap();
        assert_eq!(link.segment(1), Some("%zz"));
    }

    fn parse(raw: &str) -> Result<DeepLink, DeepLinkError> {
        DeepLink::parse(raw, "riseonly", "riseonly.net")
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
    fn another_products_scheme_is_refused() {
        assert_eq!(
            parse("otherapp://chat/42"),
            Err(DeepLinkError::ForeignScheme("otherapp".into()))
        );
    }

    #[test]
    fn a_staging_build_does_not_swallow_a_production_link() {
        let staging = DeepLink::parse(
            "riseonly://chat/42",
            "riseonly-staging",
            "staging.riseonly.net",
        );
        assert!(
            matches!(staging, Err(DeepLinkError::ForeignScheme(_))),
            "each environment owns its own scheme for exactly this reason"
        );
    }

    #[test]
    fn a_web_url_on_a_foreign_host_is_refused() {
        assert_eq!(
            parse("https://evil.example/chat/42"),
            Err(DeepLinkError::ForeignScheme("evil.example".into()))
        );
    }

    #[test]
    fn percent_and_plus_encoding_are_decoded() {
        let link = parse("riseonly://search/hello%20world?q=a+b").unwrap();
        assert_eq!(link.segment(1), Some("hello world"));
        assert_eq!(link.parameter("q"), Some("a b"));
    }

    #[test]
    fn cyrillic_survives_percent_encoding() {
        let link = parse("riseonly://search/%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82").unwrap();
        assert_eq!(link.segment(1), Some("привет"));
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
    }

    #[test]
    fn trailing_and_repeated_slashes_do_not_create_empty_segments() {
        let link = parse("riseonly://chat//42/").unwrap();
        assert_eq!(link.segments, vec!["chat", "42"]);
    }

    #[test]
    fn the_scheme_is_matched_case_insensitively() {
        assert!(parse("RISEONLY://chat/42").is_ok());
        assert!(parse("HTTPS://RISEONLY.NET/chat/42").is_ok());
    }
}
