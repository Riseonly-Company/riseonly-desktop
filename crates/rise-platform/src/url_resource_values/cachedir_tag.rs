use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::host_os::HostOs;

use super::url_resource_values::BackupFlag;

pub const CACHEDIR_TAG_NAME: &str = "CACHEDIR.TAG";

pub const CACHEDIR_TAG_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

pub const CACHEDIR_TAG_CONTENTS: &str = concat!(
    "Signature: 8a477f597d28d172789f06886806bc55\n",
    "# This file is a cache directory tag created by Riseonly.\n",
    "# For information about cache directory tags, see:\n",
    "#\thttps://bford.info/cachedir/\n",
);

/// true when the head of a file is a valid tag
/// none means missing or unreadable file
pub fn is_cachedir_tag(head: Option<&[u8]>) -> bool {
    head.is_some_and(|bytes| bytes.starts_with(CACHEDIR_TAG_SIGNATURE))
}

/// $XDG_CACHE_HOME if absolute else $HOME/.cache, like the base dir spec says
pub fn xdg_cache_home(value: Option<&OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    match value.map(Path::new) {
        Some(path) if path.is_absolute() => Some(path.to_path_buf()),
        _ => home.filter(|home| home.is_absolute()).map(|home| home.join(".cache")),
    }
}

/// everything the linux layer read about one item before it decides
#[derive(Clone, Copy, Debug)]
pub struct LinuxBackupItem<'a> {
    pub path: &'a Path,
    pub is_dir: bool,
    pub excluded: bool,
    pub cache_home: Option<&'a Path>,
    pub has_own_tag: bool,
    pub inside_tagged_dir: bool,
}

/// what the linux layer does next
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinuxBackupStep {
    Answer(BackupFlag),
    WriteTag,
    RemoveTag,
}

pub fn linux_backup_step(item: &LinuxBackupItem<'_>) -> LinuxBackupStep {
    let in_cache = item.cache_home.is_some_and(|home| item.path.starts_with(home));
    if item.excluded {
        if item.inside_tagged_dir {
            return LinuxBackupStep::Answer(BackupFlag::NotNeeded(HostOs::Linux));
        }
        if item.is_dir {
            return if item.has_own_tag {
                LinuxBackupStep::Answer(BackupFlag::Set)
            } else {
                LinuxBackupStep::WriteTag
            };
        }
        return LinuxBackupStep::Answer(if in_cache {
            BackupFlag::NotNeeded(HostOs::Linux)
        } else {
            BackupFlag::NoItemFlag(HostOs::Linux)
        });
    }
    if item.is_dir && item.has_own_tag {
        return LinuxBackupStep::RemoveTag;
    }
    LinuxBackupStep::Answer(if item.inside_tagged_dir || in_cache {
        BackupFlag::NoItemFlag(HostOs::Linux)
    } else {
        BackupFlag::NotNeeded(HostOs::Linux)
    })
}
