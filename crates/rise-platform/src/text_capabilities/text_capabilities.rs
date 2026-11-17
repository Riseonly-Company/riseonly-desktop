use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::HostOs;

/// why the platform couldnt say if a text draws, callers treat the text as drawable
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextCoverageUnavailable {
    BranchNotWritten { host: HostOs },
    SystemCallFailed { host: HostOs, call: &'static str },
}

impl std::fmt::Display for TextCoverageUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BranchNotWritten { host } => {
                write!(formatter, "the {host:?} layer does not read glyph coverage yet")
            }
            Self::SystemCallFailed { host, call } => {
                write!(formatter, "{call} returned nothing on {host:?}")
            }
        }
    }
}

impl std::error::Error for TextCoverageUnavailable {}

pub const DRAWS_TEXT_CACHE_LIMIT: usize = 256;

/// true when every character of text draws with a real glyph from the system font cascade
/// a box means LastResort or empty glyphs. cached by text, unknown hosts say true
pub fn draws_text(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let known = cache.lock().ok().and_then(|answers| answers.get(text).copied());
    if let Some(answer) = known {
        return answer;
    }
    let answer = drawable_or_assumed(super::os::glyph_coverage(text, HostOs::current()));
    if let Ok(mut answers) = cache.lock() {
        if answers.len() >= DRAWS_TEXT_CACHE_LIMIT {
            answers.clear();
        }
        answers.insert(text.to_owned(), answer);
    }
    answer
}

/// answer of a glyph coverage read, a failed read counts as drawable
pub fn drawable_or_assumed(reading: Result<bool, TextCoverageUnavailable>) -> bool {
    match reading {
        Ok(drawable) => drawable,
        Err(reason) => {
            tracing::debug!(%reason, "glyph coverage unreadable, the text is shown as is");
            true
        }
    }
}

pub fn renders_flag_emoji() -> bool {
    let host = HostOs::current();
    match host {
        HostOs::Linux => {
            static INSTALLED: OnceLock<bool> = OnceLock::new();
            *INSTALLED.get_or_init(|| has_flag_emoji_font(&linux_font_dirs_from_env()))
        }
        HostOs::MacOs | HostOs::Windows => renders_flag_emoji_on(host),
    }
}

/// answer for host without touching the disk, linux counts as having no color emoji font
pub fn renders_flag_emoji_on(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs)
}

/// true when the file is the color emoji font gpui draws on linux, eg NotoColorEmoji.ttf
pub fn is_flag_emoji_font_file(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.starts_with("notocoloremoji") && lower.ends_with(".ttf")
}

/// true when a font file under dirs draws flags in color
pub fn has_flag_emoji_font(dirs: &[PathBuf]) -> bool {
    let mut found = false;
    walk_font_files(dirs, |file_name| {
        found = is_flag_emoji_font_file(file_name);
        found
    });
    found
}

pub const LINUX_MONOSPACED_FAMILIES: [&str; 3] =
    ["DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono"];

/// family the system monospaced font resolves to on this machine
/// cached after first call, linux reads the font dirs once
pub fn monospaced_family() -> &'static str {
    let host = HostOs::current();
    match host {
        HostOs::Linux => {
            static INSTALLED: OnceLock<&'static str> = OnceLock::new();
            *INSTALLED.get_or_init(|| {
                pick_linux_monospaced(&installed_monospaced_families(&linux_font_dirs_from_env()))
            })
        }
        HostOs::MacOs | HostOs::Windows => monospaced_family_on(host),
    }
}

/// monospaced family for host without touching the disk, linux gives the first choice
pub fn monospaced_family_on(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => "Menlo",
        HostOs::Windows => "Consolas",
        HostOs::Linux => LINUX_MONOSPACED_FAMILIES[0],
    }
}

/// best family of installed by the LINUX_MONOSPACED_FAMILIES order
/// none installed keeps the first choice and gpui falls back to the ui font
pub fn pick_linux_monospaced(installed: &[&'static str]) -> &'static str {
    LINUX_MONOSPACED_FAMILIES
        .iter()
        .find(|family| installed.contains(family))
        .copied()
        .unwrap_or(LINUX_MONOSPACED_FAMILIES[0])
}

/// linux_font_dirs for this process HOME, XDG_DATA_HOME and XDG_DATA_DIRS
fn linux_font_dirs_from_env() -> Vec<PathBuf> {
    let home = std::env::var("HOME").ok();
    let data_home = std::env::var("XDG_DATA_HOME").ok();
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok();
    linux_font_dirs(home.as_deref(), data_home.as_deref(), data_dirs.as_deref())
}

/// dirs gpui font database reads on linux, no duplicates
pub fn linux_font_dirs(
    home: Option<&str>,
    xdg_data_home: Option<&str>,
    xdg_data_dirs: Option<&str>,
) -> Vec<PathBuf> {
    let mut dirs = vec!["/usr/share/fonts".to_owned(), "/usr/local/share/fonts".to_owned()];
    let home = home.map(|home| home.trim_end_matches('/')).filter(|home| !home.is_empty());
    if let Some(home) = home {
        dirs.push(format!("{home}/.fonts"));
        dirs.push(format!("{home}/.local/share/fonts"));
    }
    let absolute = |dir: &&str| dir.starts_with('/');
    let data_dirs = xdg_data_dirs.into_iter().flat_map(|dirs| dirs.split(':'));
    for dir in xdg_data_home.into_iter().chain(data_dirs).filter(absolute) {
        dirs.push(format!("{}/fonts", dir.trim_end_matches('/')));
    }
    dirs.push("/run/host/fonts".to_owned());
    dirs.push("/run/host/user-fonts".to_owned());
    let mut unique: Vec<PathBuf> = Vec::with_capacity(dirs.len());
    for dir in dirs.into_iter().map(PathBuf::from) {
        if !unique.contains(&dir) {
            unique.push(dir);
        }
    }
    unique
}

/// monospaced family a font file carries by the usual distro names, eg DejaVuSansMono-Bold.ttf
pub fn monospaced_family_of_file(file_name: &str) -> Option<&'static str> {
    let lower = file_name.to_ascii_lowercase();
    let is_font = [".ttf", ".otf", ".ttc"].iter().any(|ext| lower.ends_with(ext));
    if !is_font {
        return None;
    }
    [("dejavusansmono", 0), ("liberationmono", 1), ("notosansmono", 2)]
        .iter()
        .find(|(prefix, _)| lower.starts_with(prefix))
        .map(|(_, index)| LINUX_MONOSPACED_FAMILIES[*index])
}

const FONT_DIR_DEPTH: usize = 4;
const FONT_WALK_BUDGET: usize = 20_000;

/// families of LINUX_MONOSPACED_FAMILIES that have a font file under dirs
pub fn installed_monospaced_families(dirs: &[PathBuf]) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    walk_font_files(dirs, |file_name| {
        let family = monospaced_family_of_file(file_name);
        if let Some(family) = family.filter(|family| !found.contains(family)) {
            found.push(family);
        }
        found.len() == LINUX_MONOSPACED_FAMILIES.len()
    });
    found
}

/// hands every file name under dirs to visit, depth and entries are capped
/// visit returning true stops the walk
fn walk_font_files(dirs: &[PathBuf], mut visit: impl FnMut(&str) -> bool) {
    let mut budget = FONT_WALK_BUDGET;
    let mut pending: Vec<(PathBuf, usize)> = dirs.iter().map(|dir| (dir.clone(), 0)).collect();
    while let Some((dir, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let path = entry.path();
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                if depth < FONT_DIR_DEPTH {
                    pending.push((path, depth + 1));
                }
                continue;
            }
            if entry.file_name().to_str().is_some_and(&mut visit) {
                return;
            }
        }
    }
}
