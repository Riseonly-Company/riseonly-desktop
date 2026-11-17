use std::fs::{DirBuilder, Permissions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::paths::{APPLICATION, ORGANISATION, QUALIFIER};
use super::private_temporary::{
    DirectoryEntry, PRIVATE_DIRECTORY_MODE, PrivacyVerdict, choose_temporary_directory,
    privacy_verdict, temporary_candidates,
};

pub(super) fn per_user_temporary_directory() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from(QUALIFIER, ORGANISATION, APPLICATION)?;
    let candidates = temporary_candidates(dirs.runtime_dir(), Some(dirs.cache_dir()));
    let user = current_user();
    choose_temporary_directory(&candidates, |candidate| {
        let private = user.is_some_and(|user| make_private(candidate, user));
        if !private {
            tracing::warn!(path = %candidate.display(), "temporary directory is not private to this user");
        }
        private
    })
}

pub(super) fn system_logs_directory() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from(QUALIFIER, ORGANISATION, APPLICATION)?;
    Some(dirs.state_dir().unwrap_or(dirs.data_local_dir()).join("logs"))
}

pub(super) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIRECTORY_MODE)
        .create(path)
}

fn current_user() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|metadata| metadata.uid())
}

/// creates the dir as 0700 if missing, then accepts it only if it is a real dir of this user
/// one this user owns but left open to others is set to 0700 first
fn make_private(path: &Path, user: u32) -> bool {
    if create_private_dir_all(path).is_err() {
        return false;
    }
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let entry = DirectoryEntry {
        is_directory: metadata.file_type().is_dir(),
        owner: metadata.uid(),
        mode: metadata.mode() & 0o7777,
    };
    match privacy_verdict(&entry, user) {
        PrivacyVerdict::Private => true,
        PrivacyVerdict::TightenMode => {
            std::fs::set_permissions(path, Permissions::from_mode(PRIVATE_DIRECTORY_MODE)).is_ok()
        }
        PrivacyVerdict::Refuse => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("riseonly-{name}-{}", std::process::id()))
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[test]
    fn created_directories_are_private_to_the_user() {
        let root = scratch("private-dir");
        let leaf = root.join("data").join("logs");

        create_private_dir_all(&leaf).expect("create");
        create_private_dir_all(&leaf).expect("create over an existing directory");

        for directory in [&root, &root.join("data"), &leaf] {
            assert_eq!(mode_of(directory), 0o700, "{}", directory.display());
        }
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn the_process_user_owns_what_it_creates() {
        let root = scratch("owner");
        create_private_dir_all(&root).expect("create");

        let user = current_user().expect("procfs");
        assert_eq!(std::fs::metadata(&root).expect("metadata").uid(), user);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn an_open_directory_of_the_user_is_tightened_and_used() {
        let root = scratch("tighten");
        std::fs::create_dir_all(&root).expect("create");
        std::fs::set_permissions(&root, Permissions::from_mode(0o755)).expect("open it");

        assert!(make_private(&root, current_user().expect("procfs")));
        assert_eq!(mode_of(&root), 0o700);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_symlink_to_a_directory_is_refused() {
        let root = scratch("symlink");
        let target = root.join("target");
        let link = root.join("link");
        create_private_dir_all(&target).expect("create");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        assert!(!make_private(&link, current_user().expect("procfs")));
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_directory_is_refused_for_another_user() {
        let root = scratch("other-user");
        create_private_dir_all(&root).expect("create");

        let other = current_user().expect("procfs").wrapping_add(1);
        assert!(!make_private(&root, other));
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
