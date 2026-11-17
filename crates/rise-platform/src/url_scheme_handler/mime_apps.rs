use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

const DEFAULT_APPLICATIONS: &str = "Default Applications";
const ADDED_ASSOCIATIONS: &str = "Added Associations";
const REMOVED_ASSOCIATIONS: &str = "Removed Associations";
const MIME_CACHE: &str = "MIME Cache";
const DESKTOP_ENTRY: &str = "Desktop Entry";
const MIME_APPS_LIST: &str = "mimeapps.list";
const MIME_INFO_CACHE: &str = "mimeinfo.cache";

/// the disk as the lookup needs it, a file that cant be read counts as absent
pub trait XdgFiles {
    fn read(&self, path: &Path) -> Option<String>;
    fn is_dir(&self, path: &Path) -> bool;
}

/// the environment variables involved as they were read
#[derive(Clone, Default, Debug)]
pub struct XdgValues {
    pub home: Option<String>,
    pub config_home: Option<String>,
    pub config_dirs: Option<String>,
    pub data_home: Option<String>,
    pub data_dirs: Option<String>,
    pub current_desktop: Option<String>,
}

impl XdgValues {
    /// the values of this process
    pub fn from_environment() -> Self {
        Self {
            home: std::env::var("HOME").ok(),
            config_home: std::env::var("XDG_CONFIG_HOME").ok(),
            config_dirs: std::env::var("XDG_CONFIG_DIRS").ok(),
            data_home: std::env::var("XDG_DATA_HOME").ok(),
            data_dirs: std::env::var("XDG_DATA_DIRS").ok(),
            current_desktop: std::env::var("XDG_CURRENT_DESKTOP").ok(),
        }
    }
}

pub struct XdgDisk;

impl XdgFiles for XdgDisk {
    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

/// directories to search, most important first
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct XdgSearchPaths {
    pub config_home: Option<PathBuf>,
    pub config_dirs: Vec<PathBuf>,
    pub data_home: Option<PathBuf>,
    pub data_dirs: Vec<PathBuf>,
    pub desktops: Vec<String>,
}

/// only an absolute path counts, whichever os runs the tests
fn absolute(value: &str) -> Option<PathBuf> {
    value.starts_with('/').then(|| PathBuf::from(value))
}

/// an unset or empty list means its default
fn directory_list(value: Option<&str>, default: &[&str]) -> Vec<PathBuf> {
    match value.filter(|value| !value.is_empty()) {
        Some(value) => value.split(':').filter_map(absolute).collect(),
        None => default.iter().copied().filter_map(absolute).collect(),
    }
}

/// an unset, empty or relative variable means $HOME plus the default
fn home_directory(value: Option<&str>, home: Option<&str>, default: &[&str]) -> Option<PathBuf> {
    value.and_then(absolute).or_else(|| {
        let mut path = absolute(home?)?;
        path.extend(default);
        Some(path)
    })
}

impl XdgSearchPaths {
    pub fn from_values(values: &XdgValues) -> Self {
        let home = values.home.as_deref();
        Self {
            config_home: home_directory(values.config_home.as_deref(), home, &[".config"]),
            config_dirs: directory_list(values.config_dirs.as_deref(), &["/etc/xdg"]),
            data_home: home_directory(values.data_home.as_deref(), home, &[".local", "share"]),
            data_dirs: directory_list(
                values.data_dirs.as_deref(),
                &["/usr/local/share", "/usr/share"],
            ),
            desktops: values
                .current_desktop
                .as_deref()
                .unwrap_or_default()
                .split(':')
                .filter(|name| !name.is_empty())
                .map(str::to_ascii_lowercase)
                .collect(),
        }
    }

    /// applications dirs under the data dirs, in the order desktop files shadow each other
    pub fn application_dirs(&self) -> Vec<PathBuf> {
        self.data_home
            .iter()
            .chain(&self.data_dirs)
            .map(|dir| dir.join("applications"))
            .collect()
    }

    /// every mimeapps.list of the spec table, highest precedence first
    pub fn mime_apps_lists(&self) -> Vec<PathBuf> {
        let mut names: Vec<String> = self
            .desktops
            .iter()
            .map(|desktop| format!("{desktop}-mimeapps.list"))
            .collect();
        names.push(MIME_APPS_LIST.to_owned());

        let applications = self.application_dirs();
        self.config_home
            .iter()
            .chain(&self.config_dirs)
            .chain(&applications)
            .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
            .collect()
    }
}

/// mime type of a scheme, None when its not a valid scheme
pub fn scheme_mime_type(scheme: &str) -> Option<String> {
    let mut characters = scheme.chars();
    let starts_with_letter = characters.next()?.is_ascii_alphabetic();
    let valid = characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    (starts_with_letter && valid).then(|| format!("x-scheme-handler/{}", scheme.to_ascii_lowercase()))
}

/// the semicolon separated values of key in group of a key file, a repeated group adds up
pub fn group_values<'a>(text: &'a str, group: &str, key: &str) -> Vec<&'a str> {
    let mut in_group = false;
    let mut values = Vec::new();
    for raw in text.lines() {
        let line = trimming_whitespaces_and_newlines(raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']')) {
            in_group = name == group;
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if trimming_whitespaces_and_newlines(name) == key {
            values.extend(
                value
                    .split(';')
                    .map(trimming_whitespaces_and_newlines)
                    .filter(|value| !value.is_empty()),
            );
        }
    }
    values
}

/// an entry with Hidden=true was deleted, whatever lower dirs hold
pub fn entry_is_hidden(text: &str) -> bool {
    group_values(text, DESKTOP_ENTRY, "Hidden").contains(&"true")
}

/// a-b.desktop also names a/b.desktop under an applications dir
fn read_entry(files: &dyn XdgFiles, dir: &Path, id: &str) -> Option<String> {
    if let Some(text) = files.read(&dir.join(id)) {
        return Some(text);
    }
    id.match_indices('-').find_map(|(at, _)| {
        let (prefix, rest) = (&id[..at], &id[at + 1..]);
        if prefix.is_empty() || rest.is_empty() {
            return None;
        }
        let nested = dir.join(prefix);
        files
            .is_dir(&nested)
            .then(|| read_entry(files, &nested, rest))
            .flatten()
    })
}

/// first dir of search holding the id unless Hidden=true deleted it there
fn visible_entry(files: &dyn XdgFiles, search: &[PathBuf], id: &str) -> Option<String> {
    if !id.ends_with(".desktop") || id.contains('/') {
        return None;
    }
    search
        .iter()
        .find_map(|dir| read_entry(files, dir, id))
        .filter(|text| !entry_is_hidden(text))
}

/// true if anything is associated with the type, by the spec listing algorithm
/// also gives the ids removed on the way
/// a dir is one level, a copy in a level above shadows the id
fn walk_associations(
    mime_type: &str,
    paths: &XdgSearchPaths,
    application_dirs: &[PathBuf],
    files: &dyn XdgFiles,
) -> (bool, HashSet<String>) {
    let mut removed: HashSet<String> = HashSet::new();
    let exists_in = |search: &[PathBuf], id: &str| {
        search.iter().any(|dir| read_entry(files, dir, id).is_some())
    };

    for dir in paths.config_home.iter().chain(&paths.config_dirs) {
        let Some(text) = files.read(&dir.join(MIME_APPS_LIST)) else {
            continue;
        };
        let has_added = group_values(&text, ADDED_ASSOCIATIONS, mime_type)
            .into_iter()
            .any(|id| !removed.contains(id) && visible_entry(files, application_dirs, id).is_some());
        if has_added {
            return (true, removed);
        }
        removed.extend(group_values(&text, REMOVED_ASSOCIATIONS, mime_type).into_iter().map(str::to_owned));
    }

    for (level, dir) in application_dirs.iter().enumerate() {
        let above = &application_dirs[..level];
        let usable = |removed: &HashSet<String>, search: &[PathBuf], id: &str| {
            !removed.contains(id)
                && !exists_in(above, id)
                && visible_entry(files, search, id).is_some()
        };
        if let Some(text) = files.read(&dir.join(MIME_APPS_LIST)) {
            let has_added = group_values(&text, ADDED_ASSOCIATIONS, mime_type)
                .into_iter()
                .any(|id| usable(&removed, &application_dirs[level..], id));
            if has_added {
                return (true, removed);
            }
            removed.extend(group_values(&text, REMOVED_ASSOCIATIONS, mime_type).into_iter().map(str::to_owned));
        }
        let cached = files.read(&dir.join(MIME_INFO_CACHE)).is_some_and(|text| {
            group_values(&text, MIME_CACHE, mime_type)
                .into_iter()
                .any(|id| usable(&removed, std::slice::from_ref(dir), id))
        });
        if cached {
            return (true, removed);
        }
    }
    (false, removed)
}

/// whether the system could pick an app for the scheme
/// wrong no sends to the web link, wrong yes strands the person
/// so a default counts only when associated with the type
pub fn has_scheme_handler(scheme: &str, paths: &XdgSearchPaths, files: &dyn XdgFiles) -> bool {
    let Some(mime_type) = scheme_mime_type(scheme) else {
        return false;
    };
    let application_dirs = paths.application_dirs();
    let (associated, removed) = walk_associations(&mime_type, paths, &application_dirs, files);
    if associated {
        return true;
    }
    paths.mime_apps_lists().iter().any(|path| {
        files.read(path).is_some_and(|text| {
            group_values(&text, DEFAULT_APPLICATIONS, &mime_type).into_iter().any(|id| {
                !removed.contains(id)
                    && visible_entry(files, &application_dirs, id).is_some_and(|entry| {
                        group_values(&entry, DESKTOP_ENTRY, "MimeType").contains(&mime_type.as_str())
                    })
            })
        })
    })
}

/// id that Default Applications names for the scheme
/// first in list order that is installed and declares the type
pub fn default_scheme_handler(
    scheme: &str,
    paths: &XdgSearchPaths,
    files: &dyn XdgFiles,
) -> Option<String> {
    let mime_type = scheme_mime_type(scheme)?;
    let application_dirs = paths.application_dirs();
    let (_, removed) = walk_associations(&mime_type, paths, &application_dirs, files);
    paths.mime_apps_lists().iter().find_map(|path| {
        let text = files.read(path)?;
        group_values(&text, DEFAULT_APPLICATIONS, &mime_type)
            .into_iter()
            .find(|id| {
                !removed.contains(*id)
                    && visible_entry(files, &application_dirs, id).is_some_and(|entry| {
                        group_values(&entry, DESKTOP_ENTRY, "MimeType").contains(&mime_type.as_str())
                    })
            })
            .map(str::to_owned)
    })
}
