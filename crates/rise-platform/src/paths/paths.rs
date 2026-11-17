use std::path::{Path, PathBuf};

use rise_deeplink::{ENVIRONMENTS, Environment};
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

/// every dir the app writes to, resolved once
/// data is in local app data and not roaming, cloud syncing one sqlite file corrupts it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppPaths {
    data: PathBuf,
    cache: PathBuf,
    logs: PathBuf,
}

impl AppPaths {
    /// environment must come from the deeplink environment helper, never a caller string
    pub fn resolve(environment: &Environment) -> Result<Self, PathsError> {
        let dirs = directories::ProjectDirs::from(QUALIFIER, ORGANISATION, APPLICATION)
            .ok_or(PathsError::NoHome)?;

        Ok(Self::from_roots(
            dirs.data_local_dir(),
            dirs.cache_dir(),
            environment,
        ))
    }

    pub fn from_roots(data_root: &Path, cache_root: &Path, environment: &Environment) -> Self {
        let suffix = directory_suffix(environment);
        let data = suffixed(data_root, &suffix);
        let cache = suffixed(cache_root, &suffix);
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

    /// takes a hash and not an id, an account id must never become a path component
    pub fn account_dir(&self, account_hash: &str) -> PathBuf {
        self.data.join("accounts").join(account_hash)
    }

    /// creates the data and cache and logs dirs
    /// new dirs are private to the user (0700 where the os has modes), existing ones keep their mode
    pub fn create_all(&self) -> Result<(), PathsError> {
        for directory in [&self.data, &self.cache, &self.logs] {
            super::os::create_private_dir_all(directory)?;
        }
        Ok(())
    }

    /// true when any path component is a dir a cloud service is known to sync
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

pub const TEMPORARY_COMPONENT: &str = "tmp";

/// temp dir for this app, each user gets their own
/// linux /tmp is shared so the os layer gives a per user one
/// dir exists on return so callers create below it
pub fn temporary_directory() -> PathBuf {
    let per_user = super::os::per_user_temporary_directory();
    let directory = temporary_directory_from(per_user, &std::env::temp_dir());
    let _ = super::os::create_private_dir_all(&directory);
    directory
}

/// policy of temporary_directory, the os per user dir if there is one
/// else the app subdir of the system temp dir
pub fn temporary_directory_from(per_user: Option<PathBuf>, system_temporary: &Path) -> PathBuf {
    per_user.unwrap_or_else(|| temporary_directory_in(system_temporary))
}

/// temporary_directory below a given system temp dir
pub fn temporary_directory_in(system_temporary: &Path) -> PathBuf {
    system_temporary.join(format!("{QUALIFIER}.{ORGANISATION}.{APPLICATION}"))
}

pub fn system_logs_directory() -> Option<PathBuf> {
    super::os::system_logs_directory()
}

pub const DOWNLOADS_COMPONENT: &str = "Downloads";

/// users downloads dir, where a save dialog starts for exports (eg a resume pdf)
/// mac and windows ask their folder apis, linux reads XDG_DOWNLOAD_DIR which may be absent
/// none when no existing dir is found
pub fn downloads_directory() -> Option<PathBuf> {
    let user_dirs = directories::UserDirs::new()?;
    downloads_directory_from(
        user_dirs.download_dir().map(Path::to_path_buf),
        user_dirs.home_dir(),
        |path| path.is_dir(),
    )
}

/// policy of downloads_directory, the dir the os names else Downloads under home
/// only if it exists as a directory
pub fn downloads_directory_from(
    named: Option<PathBuf>,
    home: &Path,
    is_directory: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let fallback = home.join(DOWNLOADS_COMPONENT);
    named
        .filter(|path| path.as_path() != home && is_directory(path.as_path()))
        .or_else(|| is_directory(fallback.as_path()).then_some(fallback))
}

/// no suffix for the environment owning the apex web host, production builds already use that dir
/// others get the capitalised canon name like Dev or Staging
pub fn directory_suffix(environment: &Environment) -> String {
    let owns_apex_host = ENVIRONMENTS
        .iter()
        .all(|other| other.web_host.ends_with(environment.web_host));
    if owns_apex_host {
        return String::new();
    }
    let mut letters = environment.name.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
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
    use rise_deeplink::environment_for_app_env;

    fn dev() -> &'static Environment {
        environment_for_app_env(Some("dev"))
    }

    fn staging() -> &'static Environment {
        environment_for_app_env(Some("staging"))
    }

    fn prod() -> &'static Environment {
        environment_for_app_env(Some("prod"))
    }

    fn roots() -> (PathBuf, PathBuf) {
        (
            PathBuf::from("/home/u/.local/share/Riseonly"),
            PathBuf::from("/home/u/.cache/Riseonly"),
        )
    }

    #[test]
    fn the_temporary_directory_is_the_app_subdirectory_of_the_system_one() {
        let system = Path::new("/var/folders/ab/T");
        let directory = temporary_directory_in(system);

        assert_eq!(directory, Path::new("/var/folders/ab/T/net.riseonly.Riseonly"));
        assert_ne!(temporary_directory(), std::env::temp_dir());
        assert!(temporary_directory().is_absolute());
    }

    #[test]
    fn a_per_user_directory_wins_over_the_shared_system_one() {
        let per_user = PathBuf::from("/home/u/.cache/riseonly").join(TEMPORARY_COMPONENT);

        assert_eq!(temporary_directory_from(Some(per_user.clone()), Path::new("/tmp")), per_user);
    }

    #[test]
    fn without_a_per_user_directory_the_app_subdirectory_of_the_system_one_is_used() {
        assert_eq!(
            temporary_directory_from(None, Path::new("/var/folders/ab/T")),
            Path::new("/var/folders/ab/T/net.riseonly.Riseonly"),
        );
    }

    #[test]
    fn the_downloads_directory_the_os_names_wins() {
        let home = PathBuf::from("home-u");
        let named = PathBuf::from("home-u-files");

        assert_eq!(
            downloads_directory_from(Some(named.clone()), &home, |_| true),
            Some(named)
        );
    }

    #[test]
    fn without_a_named_downloads_directory_the_home_one_is_used() {
        let home = PathBuf::from("home-u");
        let fallback = home.join(DOWNLOADS_COMPONENT);
        let expected = fallback.clone();

        assert_eq!(
            downloads_directory_from(None, &home, move |path| path == fallback.as_path()),
            Some(expected)
        );
    }

    #[test]
    fn a_downloads_directory_disabled_to_home_or_missing_is_no_answer() {
        let home = PathBuf::from("home-u");
        let fallback = home.join(DOWNLOADS_COMPONENT);
        let only_fallback = |path: &Path| path == fallback.as_path();

        assert_eq!(
            downloads_directory_from(Some(home.clone()), &home, only_fallback),
            Some(fallback.clone())
        );
        assert_eq!(
            downloads_directory_from(Some(PathBuf::from("gone")), &home, |_| false),
            None
        );
    }

    #[test]
    fn a_resolved_downloads_directory_exists() {
        if let Some(directory) = downloads_directory() {
            assert!(directory.is_dir(), "{}", directory.display());
        }
    }

    #[test]
    fn create_all_makes_every_directory_and_tolerates_existing_ones() {
        let root = std::env::temp_dir().join(format!(
            "{QUALIFIER}.{ORGANISATION}.{APPLICATION}.paths-test-{}",
            std::process::id()
        ));
        let paths = AppPaths::from_roots(&root.join("data"), &root.join("cache"), prod());

        paths.create_all().expect("first create");
        paths.create_all().expect("second create over existing directories");

        assert!(paths.data_dir().is_dir());
        assert!(paths.cache_dir().is_dir());
        assert!(paths.logs_dir().is_dir());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn production_uses_the_bare_directory() {
        let (data, cache) = roots();
        let paths = AppPaths::from_roots(&data, &cache, prod());
        assert_eq!(paths.data_dir(), data);
        assert_eq!(paths.cache_dir(), cache);
    }

    #[test]
    fn each_environment_gets_a_sibling_directory_not_a_nested_one() {
        let (data, cache) = roots();
        let dev = AppPaths::from_roots(&data, &cache, dev());

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
        let dev = AppPaths::from_roots(&data, &cache, dev());
        let staging = AppPaths::from_roots(&data, &cache, staging());
        let prod = AppPaths::from_roots(&data, &cache, prod());

        assert_ne!(dev.data_dir(), staging.data_dir());
        assert_ne!(dev.data_dir(), prod.data_dir());
        assert_ne!(staging.data_dir(), prod.data_dir());
    }

    #[test]
    fn suffixes_keep_the_directory_names_earlier_builds_created() {
        assert_eq!(directory_suffix(dev()), "Dev");
        assert_eq!(directory_suffix(staging()), "Staging");
        assert_eq!(directory_suffix(prod()), "");
    }

    #[test]
    fn logs_live_under_data_so_one_cleanup_removes_both() {
        let (data, cache) = roots();
        let paths = AppPaths::from_roots(&data, &cache, prod());
        assert!(paths.logs_dir().starts_with(paths.data_dir()));
    }

    #[test]
    fn an_account_directory_is_hashed_and_scoped() {
        let (data, cache) = roots();
        let paths = AppPaths::from_roots(&data, &cache, prod());
        let account = paths.account_dir("9f2b1c");

        assert!(account.starts_with(paths.data_dir()));
        assert!(account.ends_with("9f2b1c"));
    }

    #[test]
    fn different_accounts_never_share_a_directory() {
        let (data, cache) = roots();
        let paths = AppPaths::from_roots(&data, &cache, prod());
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
        if let Ok(paths) = AppPaths::resolve(prod()) {
            assert!(
                !AppPaths::is_cloud_synced(paths.data_dir()),
                "resolved {} into a synced folder",
                paths.data_dir().display()
            );
        }
    }
}
