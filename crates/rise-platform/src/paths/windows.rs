use std::io;
use std::path::{Path, PathBuf};

use super::paths::{APPLICATION, ORGANISATION, QUALIFIER};

pub(super) fn per_user_temporary_directory() -> Option<PathBuf> {
    None
}

pub(super) fn system_logs_directory() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from(QUALIFIER, ORGANISATION, APPLICATION)?;
    Some(dirs.data_local_dir().join("logs"))
}

pub(super) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)
}
