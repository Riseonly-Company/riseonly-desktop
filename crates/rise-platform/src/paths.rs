use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathsError {
    #[error("no home directory for this user")]
    NoHome,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub const QUALIFIER: &str = "net";
pub const ORGANISATION: &str = "riseonly";
pub const APPLICATION: &str = "Riseonly";

/// Every directory the app writes to, resolved once.
///
/// The data directory must sit outside any folder the user's cloud service syncs —
/// two machines syncing one SQLite file corrupt it. Hence `data_local_dir`
/// (LOCALAPPDATA on Windows, not the roaming profile).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppPaths {
    data: PathBuf,
    cache: PathBuf,
    logs: PathBuf,
}

impl AppPaths {
    pub fn resolve(environment_suffix: &str) -> Result<Self, PathsError> {
        let dirs = directories::ProjectDirs::from(QUALIFIER, ORGANISATION, APPLICATION)
            .ok_or(PathsError::NoHome)?;

        Ok(Self::from_roots(
            dirs.data_local_dir(),
            dirs.cache_dir(),
            environment_suffix,
        ))
    }

    pub fn from_roots(data_root: &Path, cache_root: &Path, environment_suffix: &str) -> Self {
        let data = suffixed(data_root, environment_suffix);
        let cache = suffixed(cache_root, environment_suffix);
        let logs = data.join("logs");
        Self { data, cache, logs }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    pub fn logs_dir(&self) -> &Path {
        &self.logs
    }

    /// Takes a hash, not an id: an account id must never become a path component.
    pub fn account_dir(&self, account_hash: &str) -> PathBuf {
        self.data.join("accounts").join(account_hash)
    }

    pub fn create_all(&self) -> Result<(), PathsError> {
        std::fs::create_dir_all(&self.data)?;
        std::fs::create_dir_all(&self.cache)?;
        std::fs::create_dir_all(&self.logs)?;
        Ok(())
    }

    /// Whether any path component names a directory a cloud service is known to sync.
    pub fn is_cloud_synced(path: &Path) -> bool {
        const SYNCED: [&str; 6] = [
            "Dropbox",
            "OneDrive",
            "Google Drive",
            "Mobile Documents",
            "Yandex.Disk",
            "pCloud",
        ];

        path.components().any(|component| {
            let name = component.as_os_str().to_string_lossy();
            SYNCED.iter().any(|marker| name.contains(marker))
        })
    }
}

fn suffixed(root: &Path, suffix: &str) -> PathBuf {
    if suffix.is_empty() {
        return root.to_path_buf();
    }

    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(APPLICATION);

    root.with_file_name(format!("{name}{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> (PathBuf, PathBuf) {
        (
            PathBuf::from("/home/u/.local/share/Riseonly"),
            PathBuf::from("/home/u/.cache/Riseonly"),
        )
    }

    #[test]
    fn production_uses_the_bare_directory() {
        let (data, cache) = roots();
        let paths = AppPaths::from_roots(&data, &cache, "");
        assert_eq!(paths.data_dir(), data);
        assert_eq!(paths.cache_dir(), cache);
    }

    #[test]
    fn each_environment_gets_a_sibling_directory_not_a_nested_one() {
        let (data, cache) = roots();
        let dev = AppPaths::from_roots(&data, &cache, "Dev");

        assert_eq!(
            dev.data_dir(),
            Path::new("/home/u/.local/share/RiseonlyDev")
        );
        assert!(
            !dev.data_dir().starts_with(&data),
            "a nested directory would be wiped along with production data"
        );
    }

    #[test]
    fn environments_never_share_a_data_directory() {
        let (data, cache) = roots();
        let dev = AppPaths::from_roots(&data, &cache, "Dev");
        let staging = AppPaths::from_roots(&data, &cache, "Staging");
        let prod = AppPaths::from_roots(&data, &cache, "");

        assert_ne!(dev.data_dir(), staging.data_dir());
        assert_ne!(dev.data_dir(), prod.data_dir());
        assert_ne!(staging.data_dir(), prod.data_dir());
    }

    #[test]
    fn logs_live_under_data_so_one_cleanup_removes_both() {
        let (data, cache) = roots();
        let paths = AppPaths::from_roots(&data, &cache, "");
        assert!(paths.logs_dir().starts_with(paths.data_dir()));
    }

    #[test]
    fn an_account_directory_is_hashed_and_scoped() {
        let (data, cache) = roots();
        let paths = AppPaths::from_roots(&data, &cache, "");
        let account = paths.account_dir("9f2b1c");

        assert!(account.starts_with(paths.data_dir()));
        assert!(account.ends_with("9f2b1c"));
    }

    #[test]
    fn different_accounts_never_share_a_directory() {
        let (data, cache) = roots();
        let paths = AppPaths::from_roots(&data, &cache, "");
        assert_ne!(paths.account_dir("aaa"), paths.account_dir("bbb"));
    }

    #[test]
    fn cloud_synced_locations_are_recognised() {
        for path in [
            "/Users/u/Dropbox/Riseonly",
            "/Users/u/Library/Mobile Documents/Riseonly",
            "C:\\Users\\u\\OneDrive\\Riseonly",
            "/home/u/Yandex.Disk/Riseonly",
        ] {
            assert!(
                AppPaths::is_cloud_synced(Path::new(path)),
                "{path} should be flagged"
            );
        }
    }

    #[test]
    fn an_ordinary_location_is_not_flagged() {
        assert!(!AppPaths::is_cloud_synced(Path::new(
            "/home/u/.local/share/Riseonly"
        )));
        assert!(!AppPaths::is_cloud_synced(Path::new(
            "/Users/u/Library/Application Support/Riseonly"
        )));
    }

    #[test]
    fn the_real_data_directory_is_not_cloud_synced() {
        if let Ok(paths) = AppPaths::resolve("") {
            assert!(
                !AppPaths::is_cloud_synced(paths.data_dir()),
                "resolved {} into a synced folder",
                paths.data_dir().display()
            );
        }
    }
}
