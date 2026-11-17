use std::path::{MAIN_SEPARATOR, MAIN_SEPARATOR_STR, Path, PathBuf};

use super::foundation_url::{Url, UrlAllowedCharacters, adding_percent_encoding};

/// path style a file url is converted from and to
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathFlavor {
    Posix,
    Windows,
}

impl PathFlavor {
    pub const HOST: Self = if MAIN_SEPARATOR == '\\' { Self::Windows } else { Self::Posix };
}

/// where an absolute windows path is rooted
#[derive(Clone, PartialEq, Eq, Debug)]
enum WindowsRoot<'a> {
    Disk { letter: char, rest: &'a str },
    Unc { host: &'a str, rest: &'a str },
}

fn is_windows_separator(c: char) -> bool {
    c == '\\' || c == '/'
}

fn windows_disk(path: &str) -> Option<WindowsRoot<'_>> {
    let mut chars = path.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    let rest = &path[2..];
    rest.chars().next().is_none_or(is_windows_separator).then_some(WindowsRoot::Disk { letter, rest })
}

fn windows_unc(after_prefix: &str) -> Option<WindowsRoot<'_>> {
    let host_end = after_prefix.find(is_windows_separator).unwrap_or(after_prefix.len());
    let host = &after_prefix[..host_end];
    if host.is_empty() {
        return None;
    }
    Some(WindowsRoot::Unc { host, rest: &after_prefix[host_end..] })
}

/// splits an absolute windows path into its root
/// none for relative or drive relative
fn windows_root(path: &str) -> Option<WindowsRoot<'_>> {
    if let Some(verbatim) = path.strip_prefix(r"\\?\") {
        if let Some(unc) = verbatim.strip_prefix(r"UNC\") {
            return windows_unc(unc);
        }
        return windows_disk(verbatim);
    }
    let mut chars = path.chars();
    if chars.next().is_some_and(is_windows_separator) && chars.next().is_some_and(is_windows_separator) {
        return windows_unc(&path[2..]);
    }
    windows_disk(path)
}

fn encoded_segments(rest: &str, separator: fn(char) -> bool) -> String {
    rest.split(separator)
        .map(|segment| adding_percent_encoding(segment, UrlAllowedCharacters::URL_PATH_ALLOWED))
        .collect::<Vec<_>>()
        .join("/")
}

/// file url string for an absolute path, none if the path is not absolute
pub fn file_url_string(path: &str, flavor: PathFlavor) -> Option<String> {
    match flavor {
        PathFlavor::Posix => {
            if !path.starts_with('/') {
                return None;
            }
            Some(format!("file://{}", encoded_segments(path, |c| c == '/')))
        }
        PathFlavor::Windows => match windows_root(path)? {
            WindowsRoot::Disk { letter, rest } => {
                let tail = if rest.is_empty() { "/".to_string() } else { encoded_segments(rest, is_windows_separator) };
                Some(format!("file:///{letter}:{tail}"))
            }
            WindowsRoot::Unc { host, rest } => {
                let host = adding_percent_encoding(host, UrlAllowedCharacters::URL_HOST_ALLOWED);
                let tail = if rest.is_empty() { "/".to_string() } else { encoded_segments(rest, is_windows_separator) };
                Some(format!("file://{host}{tail}"))
            }
        },
    }
}

/// local path a file url points to, none for a non file url or bad escapes
/// or on windows a url with no drive or a unc one with no share
pub fn file_path_string(url: &Url, flavor: PathFlavor) -> Option<String> {
    if !url.is_file_url() {
        return None;
    }
    let path = url.path();
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    match flavor {
        PathFlavor::Posix => path.starts_with('/').then_some(path),
        PathFlavor::Windows => {
            let host = url.host().filter(|host| !host.eq_ignore_ascii_case("localhost"));
            match host {
                None => {
                    let after_slash = path.strip_prefix('/')?;
                    let mut chars = after_slash.chars();
                    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
                    if !matches!(chars.next(), Some(':' | '|')) {
                        return None;
                    }
                    let rest = &after_slash[2..];
                    if !(rest.is_empty() || rest.starts_with('/')) {
                        return None;
                    }
                    let rest = if rest.is_empty() { "\\".to_string() } else { rest.replace('/', "\\") };
                    Some(format!("{letter}:{rest}"))
                }
                Some(host) => {
                    if path.len() < 2 || !path.starts_with('/') {
                        return None;
                    }
                    Some(format!(r"\\{host}{}", path.replace('/', "\\")))
                }
            }
        }
    }
}

fn posix_path_from_non_utf8_escapes(encoded_path: &str) -> Option<PathBuf> {
    if PathFlavor::HOST != PathFlavor::Posix || !encoded_path.starts_with('/') {
        return None;
    }
    let escapes_are_whole = encoded_path.split('%').skip(1).all(|after_percent| {
        let digits = after_percent.as_bytes();
        digits.len() >= 2 && digits[0].is_ascii_hexdigit() && digits[1].is_ascii_hexdigit()
    });
    if !escapes_are_whole {
        return None;
    }
    let parsed = ::url::Url::parse(&format!("file://{encoded_path}")).ok()?;
    let path = parsed.to_file_path().ok()?;
    (!path.as_os_str().as_encoded_bytes().contains(&0)).then_some(path)
}

/// file url string for an absolute posix path given as bytes
/// linux names can be non utf-8 (old koi8 archives) and the escapes keep all of them
pub fn posix_file_url_string_from_bytes(path: &[u8]) -> Option<String> {
    if path.first() != Some(&b'/') {
        return None;
    }
    let mut out = String::with_capacity(path.len() + "file://".len());
    out.push_str("file://");
    for byte in path {
        if UrlAllowedCharacters::URL_PATH_ALLOWED.contains(*byte) {
            out.push(char::from(*byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    Some(out)
}

/// true if value is an absolute local path in that flavor (eg /a on posix or C:\a on windows)
pub fn is_absolute_path_in(value: &str, flavor: PathFlavor) -> bool {
    match flavor {
        PathFlavor::Posix => value.starts_with('/'),
        PathFlavor::Windows => windows_root(value).is_some(),
    }
}

/// true if value is an absolute local path on this system
pub fn is_local_absolute_path(value: &str) -> bool {
    is_absolute_path_in(value, PathFlavor::HOST)
}

impl Url {
    /// url from a path, relative paths made absolute against the current dir
    /// none only when it cant be a file url, eg windows device path without a drive
    /// no directory probe so no trailing slash
    pub fn from_file_path(path: &Path) -> Option<Url> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::path::absolute(path)
                .or_else(|_| std::env::current_dir())
                .unwrap_or_else(|_| PathBuf::from(MAIN_SEPARATOR_STR))
        };
        if absolute.to_str().is_none() && PathFlavor::HOST == PathFlavor::Posix {
            let string = posix_file_url_string_from_bytes(absolute.as_os_str().as_encoded_bytes())?;
            let url = Url::from_string_encoding_invalid_characters(&string, true)?;
            return Some(url.keeping_non_utf8_file_path(absolute));
        }
        let string = file_url_string(&absolute.to_string_lossy(), PathFlavor::HOST)?;
        Url::from_string_encoding_invalid_characters(&string, true)
    }

    /// local path of a file url (eg /C:/Users/a gives C:\Users\a)
    pub fn to_file_path(&self) -> Option<PathBuf> {
        if let Some(path) = self.non_utf8_file_path() {
            return Some(path.to_path_buf());
        }
        match file_path_string(self, PathFlavor::HOST) {
            Some(path) => Some(PathBuf::from(path)),
            None if self.is_file_url() => posix_path_from_non_utf8_escapes(self.percent_encoded_file_path()),
            None => None,
        }
    }
}
