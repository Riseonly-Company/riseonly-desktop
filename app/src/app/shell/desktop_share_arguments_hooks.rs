use std::path::{Path, PathBuf};

use rise_core::foundation::foundation_url::Url;

const FLAG_PREFIX: char = '-';

/// launch arguments or a batch of opened urls sorted by who takes each
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesktopShareArguments {
    pub links: Vec<Url>,
    pub files: Vec<PathBuf>,
    pub texts: Vec<String>,
}

impl DesktopShareArguments {
    pub fn is_empty(&self) -> bool {
        self.links.is_empty() && self.files.is_empty() && self.texts.is_empty()
    }

    pub fn has_shared_content(&self) -> bool {
        !self.files.is_empty() || !self.texts.is_empty()
    }

    pub fn extend(&mut self, other: Self) {
        self.links.extend(other.links);
        self.files.extend(other.files);
        self.texts.extend(other.texts);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DesktopShareArgument {
    Link(Url),
    File(PathBuf),
    Text(String),
}

fn classify(raw: &str) -> DesktopShareArgument {
    if Path::new(raw).is_absolute() {
        return DesktopShareArgument::File(PathBuf::from(raw));
    }
    if !raw.contains(char::is_whitespace)
        && let Some(url) = Url::from_string(raw)
        && url.scheme().is_some()
    {
        if url.is_file_url()
            && let Some(path) = url.to_file_path()
        {
            return DesktopShareArgument::File(path);
        }
        return DesktopShareArgument::Link(url);
    }
    DesktopShareArgument::Text(raw.to_owned())
}

/// process argv, the executable isnt content and neither is a flag or the word right after it
pub fn split_launch_arguments(arguments: &[String]) -> DesktopShareArguments {
    let mut split = DesktopShareArguments::default();
    let mut follows_flag = false;
    for raw in arguments.iter().skip(1) {
        if raw.is_empty() {
            continue;
        }
        if raw.starts_with(FLAG_PREFIX) {
            follows_flag = true;
            continue;
        }
        match classify(raw) {
            DesktopShareArgument::Link(url) => split.links.push(url),
            DesktopShareArgument::File(path) => split.files.push(path),
            DesktopShareArgument::Text(text) => {
                if !follows_flag {
                    split.texts.push(text);
                }
            }
        }
        follows_flag = false;
    }
    split
}

/// what on_open_urls delivers,
/// a link of the scheme or file url for open with and dock drop,
/// anything else isnt content
pub fn split_opened_urls(urls: &[String]) -> DesktopShareArguments {
    let mut split = DesktopShareArguments::default();
    for raw in urls {
        let Some(url) = Url::from_string(raw) else {
            continue;
        };
        if url.scheme().is_none() {
            continue;
        }
        match url.is_file_url().then(|| url.to_file_path()).flatten() {
            Some(path) => split.files.push(path),
            None => split.links.push(url),
        }
    }
    split
}
