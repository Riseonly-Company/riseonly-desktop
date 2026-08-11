use std::borrow::Cow;
use std::path::{Path, PathBuf};

use gpui::{AssetSource, Result, SharedString};

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
        if path.contains("..") || Path::new(path).is_absolute() {
            return None;
        }
        Some(self.root.as_ref()?.join(path))
    }

    fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let Some(full) = self.resolve(path) else {
            return Ok(None);
        };

        match std::fs::read(&full) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

impl AssetSource for RiseAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(plain) = rise_ui::IconUi::plain_path_for_filled(path) {
            return Ok(self
                .read(&plain)?
                .map(|bytes| Cow::Owned(rise_ui::IconUi::fill_document(&bytes))));
        }

        Ok(self.read(path)?.map(Cow::Owned))
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
    fn a_filled_icon_resolves_even_though_no_such_file_exists() {
        let assets = repo_assets();
        let path = "icons/lucide/heart.filled.svg";
        assert!(
            !assets.resolve(path).unwrap().is_file(),
            "the point of this path is that nothing is on disk behind it"
        );

        let bytes = assets
            .load(path)
            .unwrap()
            .expect("the plain document stands in for it");
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("fill=\"currentColor\""));
    }

    #[test]
    fn a_filled_path_with_no_plain_document_is_still_none() {
        let assets = repo_assets();
        assert!(
            assets
                .load("icons/lucide/not-an-icon.filled.svg")
                .unwrap()
                .is_none()
        );
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
