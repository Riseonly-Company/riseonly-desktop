use std::fmt;
use std::sync::Arc;

use serde::de::{self, Deserialize, Deserializer, Visitor};
use serde::{Serialize, Serializer};

/// ascii bitmap of chars allowed in a url part for percent encoding
/// non ascii is never allowed so it always gets encoded as utf-8
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UrlAllowedCharacters {
    bits: u128,
}

const fn bits_of(chars: &str) -> u128 {
    let bytes = chars.as_bytes();
    let mut bits = 0u128;
    let mut i = 0;
    while i < bytes.len() {
        bits |= 1u128 << bytes[i];
        i += 1;
    }
    bits
}

const ALPHA_DIGIT: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const UNRESERVED: u128 = bits_of(ALPHA_DIGIT) | bits_of("-._~");
const SUB_DELIMS: u128 = bits_of("!$&'()*+,;=");

const RFC_USERINFO: u128 = UNRESERVED | SUB_DELIMS;
const RFC_HOST: u128 = UNRESERVED | SUB_DELIMS;
const RFC_IP_LITERAL: u128 = UNRESERVED | SUB_DELIMS | bits_of(":");
const RFC_PATH: u128 = UNRESERVED | SUB_DELIMS | bits_of(":@/");
const RFC_QUERY: u128 = RFC_PATH | bits_of("?");
const RFC_FRAGMENT: u128 = RFC_QUERY;

impl UrlAllowedCharacters {
    pub const URL_USER_ALLOWED: Self = Self { bits: UNRESERVED | SUB_DELIMS };
    pub const URL_PASSWORD_ALLOWED: Self = Self { bits: UNRESERVED | SUB_DELIMS };
    pub const URL_HOST_ALLOWED: Self = Self { bits: UNRESERVED | SUB_DELIMS | bits_of(":[]") };
    pub const URL_PATH_ALLOWED: Self = Self { bits: (UNRESERVED | SUB_DELIMS | bits_of(":@/")) & !bits_of(";") };
    pub const URL_QUERY_ALLOWED: Self = Self { bits: RFC_QUERY };
    pub const URL_FRAGMENT_ALLOWED: Self = Self { bits: RFC_FRAGMENT };

    /// set from these chars, ascii only
    pub fn characters_in(chars: &str) -> Self {
        Self { bits: ascii_bits(chars) }
    }

    pub fn union(self, chars: &str) -> Self {
        Self { bits: self.bits | ascii_bits(chars) }
    }

    pub fn subtracting(self, chars: &str) -> Self {
        Self { bits: self.bits & !ascii_bits(chars) }
    }

    pub fn contains(self, byte: u8) -> bool {
        contains_bit(self.bits, byte)
    }
}

fn ascii_bits(chars: &str) -> u128 {
    chars.bytes().filter(u8::is_ascii).fold(0u128, |bits, b| bits | (1u128 << b))
}

fn contains_bit(bits: u128, byte: u8) -> bool {
    byte.is_ascii() && bits & (1u128 << byte) != 0
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

fn push_escaped(out: &mut String, byte: u8) {
    out.push('%');
    out.push(char::from(HEX_UPPER[usize::from(byte >> 4)]));
    out.push(char::from(HEX_UPPER[usize::from(byte & 0x0f)]));
}

fn hex_value(symbol: u8) -> Option<u8> {
    match symbol {
        b'0'..=b'9' => Some(symbol - b'0'),
        b'a'..=b'f' => Some(symbol - b'a' + 10),
        b'A'..=b'F' => Some(symbol - b'A' + 10),
        _ => None,
    }
}

fn is_escape_at(raw: &[u8], index: usize) -> bool {
    raw.get(index) == Some(&b'%')
        && raw.get(index + 1).and_then(|b| hex_value(*b)).is_some()
        && raw.get(index + 2).and_then(|b| hex_value(*b)).is_some()
}

/// every utf-8 byte outside the set becomes %XX in upper hex
/// % too unless the set allows it
pub fn adding_percent_encoding(string: &str, with_allowed_characters: UrlAllowedCharacters) -> String {
    let mut out = String::with_capacity(string.len());
    for byte in string.bytes() {
        if with_allowed_characters.contains(byte) {
            out.push(char::from(byte));
        } else {
            push_escaped(&mut out, byte);
        }
    }
    out
}

/// decodes percent escapes, none on a broken escape or non utf-8 bytes
pub fn removing_percent_encoding(string: &str) -> Option<String> {
    let raw = string.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            if !is_escape_at(raw, i) {
                return None;
            }
            let high = hex_value(raw[i + 1])?;
            let low = hex_value(raw[i + 2])?;
            bytes.push((high << 4) | low);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

fn is_valid_component(part: &str, allowed: u128) -> bool {
    let raw = part.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && is_escape_at(raw, i) {
            i += 3;
        } else if contains_bit(allowed, raw[i]) {
            i += 1;
        } else {
            return false;
        }
    }
    true
}

/// valid component stays as is, invalid gives none
/// with encode_invalid every rejected byte gets percent encoded
/// whole component is repaired so eg a b%20c gives a%20b%2520c
fn checked_component(part: &str, allowed: u128, encode_invalid: bool) -> Option<String> {
    if is_valid_component(part, allowed) {
        return Some(part.to_owned());
    }
    if !encode_invalid {
        return None;
    }
    let mut out = String::with_capacity(part.len());
    for byte in part.bytes() {
        if contains_bit(allowed, byte) {
            out.push(char::from(byte));
        } else {
            push_escaped(&mut out, byte);
        }
    }
    Some(out)
}

fn is_valid_scheme(scheme: &str) -> bool {
    let mut bytes = scheme.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

/// percent encoded parts of a url reference, shared by Url and UrlComponents
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
struct UrlParts {
    scheme: Option<String>,
    user: Option<String>,
    password: Option<String>,
    host: Option<String>,
    port: Option<String>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl UrlParts {
    fn parse(input: &str, encode_invalid: bool) -> Option<Self> {
        let (before_fragment, fragment) = match input.find('#') {
            Some(at) => (&input[..at], Some(&input[at + 1..])),
            None => (input, None),
        };
        let (before_query, query) = match before_fragment.find('?') {
            Some(at) => (&before_fragment[..at], Some(&before_fragment[at + 1..])),
            None => (before_fragment, None),
        };

        let mut parts = Self::default();
        let mut rest = before_query;
        if let Some(colon) = rest.find(':') {
            let first_slash = rest.find('/').unwrap_or(rest.len());
            if colon < first_slash {
                let scheme = &rest[..colon];
                if !is_valid_scheme(scheme) {
                    return None;
                }
                parts.scheme = Some(scheme.to_string());
                rest = &rest[colon + 1..];
            }
        }

        if let Some(after_slashes) = rest.strip_prefix("//") {
            let authority_end = after_slashes.find('/').unwrap_or(after_slashes.len());
            parts.parse_authority(&after_slashes[..authority_end], encode_invalid)?;
            rest = &after_slashes[authority_end..];
        }

        parts.path = checked_component(rest, RFC_PATH, encode_invalid)?;
        parts.query = match query {
            Some(q) => Some(checked_component(q, RFC_QUERY, encode_invalid)?),
            None => None,
        };
        parts.fragment = match fragment {
            Some(f) => Some(checked_component(f, RFC_FRAGMENT, encode_invalid)?),
            None => None,
        };
        Some(parts)
    }

    fn parse_authority(&mut self, authority: &str, encode_invalid: bool) -> Option<()> {
        let (userinfo, host_port) = match authority.rfind('@') {
            Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
            None => (None, authority),
        };
        if let Some(userinfo) = userinfo {
            let (user, password) = match userinfo.find(':') {
                Some(at) => (&userinfo[..at], Some(&userinfo[at + 1..])),
                None => (userinfo, None),
            };
            self.user = Some(checked_component(user, RFC_USERINFO, encode_invalid)?);
            self.password = match password {
                Some(p) => Some(checked_component(p, RFC_USERINFO, encode_invalid)?),
                None => None,
            };
        }

        let (host, port) = if host_port.starts_with('[') {
            let close = host_port.find(']')?;
            let literal = &host_port[1..close];
            checked_component(literal, RFC_IP_LITERAL, false)?;
            let after = &host_port[close + 1..];
            let port = match after {
                "" => None,
                _ => Some(after.strip_prefix(':')?),
            };
            (host_port[..=close].to_string(), port)
        } else {
            let (host, port) = match host_port.rfind(':') {
                Some(at) => (&host_port[..at], Some(&host_port[at + 1..])),
                None => (host_port, None),
            };
            (encoded_host(host, encode_invalid)?, port)
        };
        if let Some(port) = port {
            if !port.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            if !port.is_empty() {
                port.parse::<i64>().ok()?;
                self.port = Some(port.to_string());
            }
        }
        self.host = Some(host);
        Some(())
    }

    fn has_authority(&self) -> bool {
        self.host.is_some() || self.user.is_some() || self.password.is_some() || self.port.is_some()
    }

    fn string(&self) -> String {
        let mut out = String::with_capacity(self.path.len() + 32);
        if let Some(scheme) = &self.scheme {
            out.push_str(scheme);
            out.push(':');
        }
        if self.has_authority() {
            out.push_str("//");
            if let Some(user) = &self.user {
                out.push_str(user);
            }
            if let Some(password) = &self.password {
                out.push(':');
                out.push_str(password);
            }
            if self.user.is_some() || self.password.is_some() {
                out.push('@');
            }
            if let Some(host) = &self.host {
                out.push_str(host);
            }
            if let Some(port) = &self.port {
                out.push(':');
                out.push_str(port);
            }
        }
        out.push_str(&self.path);
        if let Some(query) = &self.query {
            out.push('?');
            out.push_str(query);
        }
        if let Some(fragment) = &self.fragment {
            out.push('#');
            out.push_str(fragment);
        }
        out
    }

    /// resolves the reference against base, rfc 3986 5.2.2
    fn resolved(&self, base: &UrlParts) -> UrlParts {
        if self.scheme.is_some() {
            let mut target = self.clone();
            target.path = remove_dot_segments(&self.path);
            return target;
        }
        let mut target = UrlParts { fragment: self.fragment.clone(), scheme: base.scheme.clone(), ..UrlParts::default() };
        if self.has_authority() {
            target.user = self.user.clone();
            target.password = self.password.clone();
            target.host = self.host.clone();
            target.port = self.port.clone();
            target.path = remove_dot_segments(&self.path);
            target.query = self.query.clone();
            return target;
        }
        target.user = base.user.clone();
        target.password = base.password.clone();
        target.host = base.host.clone();
        target.port = base.port.clone();
        if self.path.is_empty() {
            target.path = base.path.clone();
            target.query = self.query.clone().or_else(|| base.query.clone());
        } else {
            target.path = if self.path.starts_with('/') {
                remove_dot_segments(&self.path)
            } else {
                remove_dot_segments(&merge_paths(base, &self.path))
            };
            target.query = self.query.clone();
        }
        target
    }
}

fn encoded_host(host: &str, encode_invalid: bool) -> Option<String> {
    if encode_invalid && !host.is_ascii() {
        if let Ok(url::Host::Domain(ascii)) = url::Host::parse(host) {
            return Some(ascii);
        }
    }
    checked_component(host, RFC_HOST, encode_invalid)
}

fn merge_paths(base: &UrlParts, relative: &str) -> String {
    if base.has_authority() && base.path.is_empty() {
        return format!("/{relative}");
    }
    match base.path.rfind('/') {
        Some(at) => format!("{}{relative}", &base.path[..=at]),
        None => relative.to_string(),
    }
}

/// removes dot segments, rfc 3986 5.2.4
fn remove_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut output = String::with_capacity(path.len());
    while !input.is_empty() {
        if let Some(rest) = input.strip_prefix("../") {
            input = rest;
        } else if let Some(rest) = input.strip_prefix("./") {
            input = rest;
        } else if input.starts_with("/./") {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") || input == "/.." {
            input = if input == "/.." { "/" } else { &input[3..] };
            match output.rfind('/') {
                Some(at) => output.truncate(at),
                None => output.clear(),
            }
        } else if input == "." || input == ".." {
            input = "";
        } else {
            let start = usize::from(input.starts_with('/'));
            let end = input[start..].find('/').map_or(input.len(), |at| at + start);
            output.push_str(&input[..end]);
            input = &input[end..];
        }
    }
    output
}

fn decoded(part: Option<&String>) -> Option<String> {
    part.and_then(|p| removing_percent_encoding(p))
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Url {
    parts: UrlParts,
    string: String,
    relative: Option<(String, Arc<Url>)>,
    non_utf8_file_path: Option<Arc<std::path::PathBuf>>,
}

impl Url {
    /// parses a url, invalid chars get percent encoded first
    /// none for empty or still broken after that (bad scheme or port or ip literal)
    pub fn from_string(string: &str) -> Option<Self> {
        Self::from_string_encoding_invalid_characters(string, true)
    }

    /// parses a url, with false a space or any char outside rfc 3986 gives none
    pub fn from_string_encoding_invalid_characters(string: &str, encoding_invalid_characters: bool) -> Option<Self> {
        if string.is_empty() {
            return None;
        }
        let parts = UrlParts::parse(string, encoding_invalid_characters)?;
        Some(Self::from_parts(parts))
    }

    /// parses relative to a base, relative_string keeps the reference and base_url the base
    pub fn from_string_relative_to(string: &str, relative_to: Option<&Url>) -> Option<Self> {
        let Some(base) = relative_to else {
            return Self::from_string(string);
        };
        if string.is_empty() {
            return None;
        }
        let reference = UrlParts::parse(string, true)?;
        let relative_string = reference.string();
        let resolved = reference.resolved(&base.parts);
        let mut url = Self::from_parts(resolved);
        url.relative = Some((relative_string, Arc::new(base.clone())));
        Some(url)
    }

    fn from_parts(parts: UrlParts) -> Self {
        let string = parts.string();
        Self { parts, string, relative: None, non_utf8_file_path: None }
    }

    pub(super) fn keeping_non_utf8_file_path(mut self, path: std::path::PathBuf) -> Self {
        self.non_utf8_file_path = Some(Arc::new(path));
        self
    }

    pub(super) fn non_utf8_file_path(&self) -> Option<&std::path::Path> {
        self.non_utf8_file_path.as_deref().map(std::path::PathBuf::as_path)
    }

    pub(super) fn percent_encoded_file_path(&self) -> &str {
        &self.parts.path
    }

    /// full url string
    pub fn absolute_string(&self) -> &str {
        &self.string
    }

    /// url string as written, maybe relative
    pub fn relative_string(&self) -> &str {
        match &self.relative {
            Some((relative, _)) => relative,
            None => &self.string,
        }
    }

    pub fn base_url(&self) -> Option<&Url> {
        self.relative.as_ref().map(|(_, base)| base.as_ref())
    }

    /// url made absolute against its base
    pub fn absolute_url(&self) -> Url {
        Self::from_parts(self.parts.clone())
    }

    /// scheme as written, not lowercased
    pub fn scheme(&self) -> Option<&str> {
        self.parts.scheme.as_deref()
    }

    /// host percent decoded, ip literal without brackets
    /// none when empty
    pub fn host(&self) -> Option<String> {
        let host = self.parts.host.as_ref()?;
        let bare = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
        let host = removing_percent_encoding(bare)?;
        (!host.is_empty()).then_some(host)
    }

    pub fn port(&self) -> Option<i64> {
        self.parts.port.as_ref().and_then(|p| p.parse().ok())
    }

    /// user, percent decoded
    pub fn user(&self) -> Option<String> {
        decoded(self.parts.user.as_ref())
    }

    /// password, percent decoded
    pub fn password(&self) -> Option<String> {
        decoded(self.parts.password.as_ref())
    }

    /// path percent decoded, trailing slash stripped except root /
    /// empty if decoding fails
    pub fn path(&self) -> String {
        let mut path = removing_percent_encoding(&self.parts.path).unwrap_or_default();
        if path.len() > 1 && path.ends_with('/') {
            path.pop();
        }
        path
    }

    /// query as is, still percent encoded
    pub fn query(&self) -> Option<&str> {
        self.parts.query.as_deref()
    }

    /// fragment as is, still percent encoded
    pub fn fragment(&self) -> Option<&str> {
        self.parts.fragment.as_deref()
    }

    pub fn is_file_url(&self) -> bool {
        self.parts.scheme.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("file"))
    }

    pub fn path_components(&self) -> Vec<String> {
        let path = self.path();
        let mut components = Vec::new();
        if path.starts_with('/') {
            components.push("/".to_string());
        }
        components.extend(path.split('/').filter(|s| !s.is_empty()).map(str::to_string));
        components
    }

    pub fn last_path_component(&self) -> String {
        let path = self.path();
        if path == "/" {
            return path;
        }
        path.rsplit('/').next().unwrap_or_default().to_string()
    }

    /// text after the last dot of the last component
    /// empty for a leading dot name or trailing dot
    pub fn path_extension(&self) -> String {
        let last = self.last_path_component();
        match last.rfind('.') {
            Some(at) if at > 0 && at + 1 < last.len() => last[at + 1..].to_string(),
            _ => String::new(),
        }
    }

    /// adds a path component, percent encoded and joined with exactly one /
    pub fn appending_path_component(&self, path_component: &str) -> Url {
        let encoded = adding_percent_encoding(path_component, UrlAllowedCharacters::URL_PATH_ALLOWED);
        let mut parts = self.parts.clone();
        let base_ends_with_slash = parts.path.ends_with('/');
        let addition = encoded.trim_start_matches('/');
        if !base_ends_with_slash && (!parts.path.is_empty() || parts.has_authority()) {
            parts.path.push('/');
        }
        parts.path.push_str(addition);
        Self::from_parts(parts)
    }

    pub fn appending_path_extension(&self, path_extension: &str) -> Url {
        if path_extension.is_empty() || self.last_path_component().is_empty() || self.last_path_component() == "/" {
            return self.absolute_url();
        }
        let encoded = adding_percent_encoding(path_extension, UrlAllowedCharacters::URL_PATH_ALLOWED);
        let mut parts = self.parts.clone();
        let had_slash = parts.path.len() > 1 && parts.path.ends_with('/');
        if had_slash {
            parts.path.pop();
        }
        parts.path.push('.');
        parts.path.push_str(&encoded);
        if had_slash {
            parts.path.push('/');
        }
        Self::from_parts(parts)
    }

    /// drops the last path component and keeps the trailing slash
    /// root and empty path get ../
    pub fn deleting_last_path_component(&self) -> Url {
        let mut parts = self.parts.clone();
        let trimmed = parts.path.trim_end_matches('/');
        parts.path = if trimmed.is_empty() {
            format!("{}../", parts.path)
        } else {
            match trimmed.rfind('/') {
                Some(at) => trimmed[..=at].to_string(),
                None => "./".to_string(),
            }
        };
        Self::from_parts(parts)
    }

    pub fn deleting_path_extension(&self) -> Url {
        let extension = self.path_extension();
        if extension.is_empty() {
            return self.absolute_url();
        }
        let mut parts = self.parts.clone();
        let had_slash = parts.path.len() > 1 && parts.path.ends_with('/');
        if had_slash {
            parts.path.pop();
        }
        if let Some(at) = parts.path.rfind('.') {
            parts.path.truncate(at);
        }
        if had_slash {
            parts.path.push('/');
        }
        Self::from_parts(parts)
    }

    /// url with dot segments removed from the path
    pub fn standardized(&self) -> Url {
        let mut parts = self.parts.clone();
        parts.path = remove_dot_segments(&parts.path);
        Self::from_parts(parts)
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.relative_string())
    }
}

impl Serialize for Url {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.string)
    }
}

impl<'de> Deserialize<'de> for Url {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UrlVisitor;

        impl Visitor<'_> for UrlVisitor {
            type Value = Url;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a URL string")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Url, E> {
                Url::from_string(value).ok_or_else(|| E::invalid_value(de::Unexpected::Str(value), &self))
            }
        }

        deserializer.deserialize_str(UrlVisitor)
    }
}

/// one query item, name and value
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct UrlQueryItem {
    pub name: String,
    pub value: Option<String>,
}

impl UrlQueryItem {
    /// new query item
    pub fn new(name: impl Into<String>, value: Option<String>) -> Self {
        Self { name: name.into(), value }
    }
}

const QUERY_ITEM_ALLOWED: u128 = RFC_QUERY & !bits_of("&=");

/// url parts kept percent encoded, plain getters decode and plain setters encode
/// the percent_encoded pair passes text through
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct UrlComponents {
    parts: UrlParts,
}

impl UrlComponents {
    /// empty components
    pub fn new() -> Self {
        Self::default()
    }

    /// parses strictly, none for a space or any char outside rfc 3986
    /// empty string gives empty components
    pub fn from_string(string: &str) -> Option<Self> {
        Self::from_string_encoding_invalid_characters(string, false)
    }

    /// parses with the choice to encode invalid chars
    pub fn from_string_encoding_invalid_characters(string: &str, encoding_invalid_characters: bool) -> Option<Self> {
        UrlParts::parse(string, encoding_invalid_characters).map(|parts| Self { parts })
    }

    /// components of a url, optionally resolved against its base
    pub fn from_url(url: &Url, resolving_against_base_url: bool) -> Option<Self> {
        if resolving_against_base_url || url.relative.is_none() {
            return Some(Self { parts: url.parts.clone() });
        }
        Self::from_string(url.relative_string())
    }

    /// the url, none if the parts dont make one
    pub fn url(&self) -> Option<Url> {
        let string = self.string()?;
        if string.is_empty() {
            return None;
        }
        Url::from_string_encoding_invalid_characters(&string, false)
    }

    pub fn url_relative_to(&self, relative_to: Option<&Url>) -> Option<Url> {
        let string = self.string()?;
        Url::from_string_relative_to(&string, relative_to)
    }

    /// the string, none when an authority sits next to a relative path
    /// or a path with no authority starts with //
    pub fn string(&self) -> Option<String> {
        let path = &self.parts.path;
        if self.parts.has_authority() && !path.is_empty() && !path.starts_with('/') {
            return None;
        }
        if !self.parts.has_authority() && path.starts_with("//") {
            return None;
        }
        if self.parts.scheme.is_none() && !self.parts.has_authority() {
            let first_segment = path.split('/').next().unwrap_or_default();
            if first_segment.contains(':') {
                return None;
            }
        }
        Some(self.parts.string())
    }

    pub fn scheme(&self) -> Option<&str> {
        self.parts.scheme.as_deref()
    }

    /// sets the scheme, an invalid one is just ignored
    pub fn set_scheme(&mut self, scheme: Option<&str>) {
        if scheme.is_some_and(|s| !is_valid_scheme(s)) {
            return;
        }
        self.parts.scheme = scheme.map(str::to_string);
    }

    pub fn user(&self) -> Option<String> {
        decoded(self.parts.user.as_ref())
    }

    pub fn set_user(&mut self, user: Option<&str>) {
        self.parts.user = user.map(|u| adding_percent_encoding(u, UrlAllowedCharacters::URL_USER_ALLOWED));
    }

    pub fn password(&self) -> Option<String> {
        decoded(self.parts.password.as_ref())
    }

    pub fn set_password(&mut self, password: Option<&str>) {
        self.parts.password =
            password.map(|p| adding_percent_encoding(p, UrlAllowedCharacters::URL_PASSWORD_ALLOWED));
    }

    /// host percent decoded, ip literal without brackets
    pub fn host(&self) -> Option<String> {
        let host = self.parts.host.as_ref()?;
        let bare = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
        removing_percent_encoding(bare)
    }

    pub fn set_host(&mut self, host: Option<&str>) {
        self.parts.host = host.map(|h| {
            if h.contains(':') && !h.starts_with('[') {
                format!("[{h}]")
            } else {
                adding_percent_encoding(h, UrlAllowedCharacters::URL_HOST_ALLOWED)
            }
        });
    }

    pub fn port(&self) -> Option<i64> {
        self.parts.port.as_ref().and_then(|p| p.parse().ok())
    }

    pub fn set_port(&mut self, port: Option<i64>) {
        if port.is_some_and(|p| p < 0) {
            return;
        }
        self.parts.port = port.map(|p| p.to_string());
    }

    /// path percent decoded, empty if decoding fails
    pub fn path(&self) -> String {
        removing_percent_encoding(&self.parts.path).unwrap_or_default()
    }

    pub fn set_path(&mut self, path: &str) {
        self.parts.path = adding_percent_encoding(path, UrlAllowedCharacters::URL_PATH_ALLOWED.union(";"));
    }

    pub fn query(&self) -> Option<String> {
        decoded(self.parts.query.as_ref())
    }

    pub fn set_query(&mut self, query: Option<&str>) {
        self.parts.query = query.map(|q| adding_percent_encoding(q, UrlAllowedCharacters::URL_QUERY_ALLOWED));
    }

    pub fn fragment(&self) -> Option<String> {
        decoded(self.parts.fragment.as_ref())
    }

    pub fn set_fragment(&mut self, fragment: Option<&str>) {
        self.parts.fragment =
            fragment.map(|f| adding_percent_encoding(f, UrlAllowedCharacters::URL_FRAGMENT_ALLOWED));
    }

    pub fn percent_encoded_user(&self) -> Option<&str> {
        self.parts.user.as_deref()
    }

    pub fn percent_encoded_password(&self) -> Option<&str> {
        self.parts.password.as_deref()
    }

    pub fn percent_encoded_host(&self) -> Option<&str> {
        self.parts.host.as_deref()
    }

    pub fn percent_encoded_path(&self) -> &str {
        &self.parts.path
    }

    pub fn set_percent_encoded_path(&mut self, path: &str) {
        self.parts.path = checked_component(path, RFC_PATH, true).unwrap_or_default();
    }

    pub fn percent_encoded_query(&self) -> Option<&str> {
        self.parts.query.as_deref()
    }

    /// sets the encoded query, text outside rfc 3986 gets encoded
    pub fn set_percent_encoded_query(&mut self, query: Option<&str>) {
        self.parts.query = query.and_then(|q| checked_component(q, RFC_QUERY, true));
    }

    pub fn percent_encoded_fragment(&self) -> Option<&str> {
        self.parts.fragment.as_deref()
    }

    /// query items split on & then on the first =, decoded
    /// + stays as is and an empty query has no items
    pub fn query_items(&self) -> Option<Vec<UrlQueryItem>> {
        let query = self.parts.query.as_ref()?;
        Some(split_query_items(query, |piece| {
            removing_percent_encoding(piece).unwrap_or_else(|| piece.to_string())
        }))
    }

    /// sets query items, names and values percent encoded
    /// & and = too
    pub fn set_query_items(&mut self, query_items: Option<Vec<UrlQueryItem>>) {
        self.parts.query = query_items.map(|items| {
            join_query_items(&items, |piece| adding_percent_encoding(piece, UrlAllowedCharacters { bits: QUERY_ITEM_ALLOWED }))
        });
    }

    pub fn percent_encoded_query_items(&self) -> Option<Vec<UrlQueryItem>> {
        let query = self.parts.query.as_ref()?;
        Some(split_query_items(query, str::to_string))
    }

    pub fn set_percent_encoded_query_items(&mut self, query_items: Option<Vec<UrlQueryItem>>) {
        self.parts.query = query_items.map(|items| {
            join_query_items(&items, |piece| checked_component(piece, QUERY_ITEM_ALLOWED, true).unwrap_or_default())
        });
    }
}

fn split_query_items(query: &str, convert: impl Fn(&str) -> String) -> Vec<UrlQueryItem> {
    if query.is_empty() {
        return Vec::new();
    }
    query
        .split('&')
        .map(|pair| match pair.find('=') {
            Some(at) => UrlQueryItem::new(convert(&pair[..at]), Some(convert(&pair[at + 1..]))),
            None => UrlQueryItem::new(convert(pair), None),
        })
        .collect()
}

fn join_query_items(items: &[UrlQueryItem], convert: impl Fn(&str) -> String) -> String {
    let mut out = String::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push('&');
        }
        out.push_str(&convert(&item.name));
        if let Some(value) = &item.value {
            out.push('=');
            out.push_str(&convert(value));
        }
    }
    out
}
