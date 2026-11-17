use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use super::paths::APPLICATION;

pub(super) fn per_user_temporary_directory() -> Option<PathBuf> {
    None
}

pub(super) fn system_logs_directory() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    Some(base.home_dir().join("Library").join("Logs").join(APPLICATION))
}

pub(super) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(path)
}
