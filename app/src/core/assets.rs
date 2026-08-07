use std::borrow::Cow;
use std::path::{Path, PathBuf};

use gpui::{AssetSource, Result, SharedString};

/// Everything under `assets/`, wherever this build put it.
///
/// The paths callers use are the ones the repository lays out —
/// `fonts/Inter-Regular.ttf`, `icons/lucide/arrow-left.svg` — so a component
/// never learns whether it is running from a bundle or from a checkout.
///
/// No `#[cfg(target_os)]`, deliberately: the candidate list is tried in order on
/// every host, and the macOS bundle layout simply happens to be first. A `cfg`
/// here would be the first one in `app/`, and the boundary check exists to keep
/// that number at zero.
pub struct RiseAssets {
    root: Option<PathBuf>,
}

impl RiseAssets {
    pub fn discover() -> Self {
        Self {
            root: locate_assets(),
        }
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn locales_directory(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|root| root.join("locales"))
    }

    fn resolve(&self, path: &str) -> Option<PathBuf> {
        // A path escaping the asset root is a bug in a caller, not a hostile
        // input — but resolving it would still read an arbitrary file, so it is
        // refused rather than normalised.
        if path.contains("..") || Path::new(path).is_absolute() {
            return None;
        }
        Some(self.root.as_ref()?.join(path))
    }
}

impl AssetSource for RiseAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let Some(full) = self.resolve(path) else {
            return Ok(None);
        };

        match std::fs::read(&full) {
            Ok(bytes) => Ok(Some(Cow::Owned(bytes))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let Some(full) = self.resolve(path) else {
            return Ok(Vec::new());
        };

        let Ok(entries) = std::fs::read_dir(&full) else {
            return Ok(Vec::new());
        };

        let mut names: Vec<SharedString> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| SharedString::from(entry.file_name().to_string_lossy().into_owned()))
            .collect();
        names.sort();
        Ok(names)
    }
}

/// `assets/locales` is the marker rather than `assets` itself: a stray empty
/// directory called `assets` beside the binary would otherwise win over the real
/// one and every icon would silently resolve to nothing.
fn locate_assets() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    let candidates = [
        exe_dir.join("../Resources/assets"),
        exe_dir.join("assets"),
        exe_dir.join("../assets"),
    ];

    for candidate in candidates {
        if candidate.join("locales").is_dir() {
            return std::fs::canonicalize(candidate).ok();
        }
    }

    for ancestor in exe_dir.ancestors() {
        let candidate = ancestor.join("assets");
        if candidate.join("locales").is_dir() {
            return std::fs::canonicalize(candidate).ok();
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_assets() -> RiseAssets {
        RiseAssets {
            root: std::fs::canonicalize(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets"),
            )
            .ok(),
        }
    }

    #[test]
    fn the_running_build_can_find_its_own_assets() {
        let assets = RiseAssets::discover();
        assert!(
            assets.root().is_some(),
            "no assets/locales above {:?}; fonts, icons and every string would be missing",
            std::env::current_exe()
        );
    }

    #[test]
    fn a_bundled_font_loads_by_the_path_the_ui_asks_for() {
        let assets = repo_assets();
        let bytes = assets
            .load("fonts/Inter-Regular.ttf")
            .unwrap()
            .expect("the regular face is bundled");
        assert!(bytes.len() > 10_000);
    }

    #[test]
    fn an_icon_loads_by_the_path_the_icon_component_builds() {
        let assets = repo_assets();
        assert!(
            assets
                .load("icons/lucide/arrow-left.svg")
                .unwrap()
                .is_some()
        );
        assert!(assets.load("icons/flags/ru.svg").unwrap().is_some());
    }

    #[test]
    fn a_missing_asset_is_none_rather_than_an_error() {
        let assets = repo_assets();
        assert!(assets.load("fonts/NotAFont.ttf").unwrap().is_none());
    }

    #[test]
    fn a_path_that_climbs_out_of_the_asset_root_is_refused() {
        let assets = repo_assets();
        assert!(assets.load("../../Cargo.toml").unwrap().is_none());
        assert!(assets.load("/etc/hosts").unwrap().is_none());
    }

    #[test]
    fn listing_a_directory_returns_its_entries_sorted() {
        let assets = repo_assets();
        let fonts = assets.list("fonts").unwrap();
        assert!(fonts.iter().any(|name| name.as_ref() == "Inter-Bold.ttf"));

        let mut sorted = fonts.clone();
        sorted.sort();
        assert_eq!(fonts, sorted);
    }

    #[test]
    fn listing_a_directory_that_is_not_there_is_empty_rather_than_an_error() {
        let assets = repo_assets();
        assert!(assets.list("nothing-here").unwrap().is_empty());
    }
}
