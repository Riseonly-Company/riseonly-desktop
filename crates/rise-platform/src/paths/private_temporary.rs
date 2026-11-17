use std::path::{Path, PathBuf};

use super::paths::TEMPORARY_COMPONENT;

pub const PRIVATE_DIRECTORY_MODE: u32 = 0o700;

pub const SHARED_ACCESS_BITS: u32 = 0o077;

/// what the os layer read about one candidate with symlink metadata, so a symlink is never followed
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirectoryEntry {
    pub is_directory: bool,
    pub owner: u32,
    pub mode: u32,
}

/// what the os layer does with a candidate it just created or found
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrivacyVerdict {
    Private,
    TightenMode,
    Refuse,
}

pub fn privacy_verdict(entry: &DirectoryEntry, user: u32) -> PrivacyVerdict {
    if !entry.is_directory || entry.owner != user {
        return PrivacyVerdict::Refuse;
    }
    if entry.mode & SHARED_ACCESS_BITS == 0 {
        PrivacyVerdict::Private
    } else {
        PrivacyVerdict::TightenMode
    }
}

/// candidates in order of preference, cache tmp first and runtime dir second
/// the cache dir itself is kept by the app so only its tmp child is used
pub fn temporary_candidates(runtime: Option<&Path>, cache: Option<&Path>) -> Vec<PathBuf> {
    cache
        .map(|cache| cache.join(TEMPORARY_COMPONENT))
        .into_iter()
        .chain(runtime.map(Path::to_path_buf))
        .collect()
}

/// first candidate the usable check accepts (it creates the dir and checks it)
/// if none, the first candidate so the caller fails on its own write and never falls back to shared tmp
pub fn choose_temporary_directory(
    candidates: &[PathBuf],
    mut usable: impl FnMut(&Path) -> bool,
) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|candidate| usable(candidate))
        .or_else(|| candidates.first())
        .cloned()
}
