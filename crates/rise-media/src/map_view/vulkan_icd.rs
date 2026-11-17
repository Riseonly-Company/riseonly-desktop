use std::path::{Path, PathBuf};

pub const DRIVER_FILES_VAR: &str = "VK_DRIVER_FILES";
pub const LEGACY_DRIVER_FILES_VAR: &str = "VK_ICD_FILENAMES";
pub const ADD_DRIVER_FILES_VAR: &str = "VK_ADD_DRIVER_FILES";
pub const ICD_SUBDIR: &str = "vulkan/icd.d";
pub const SYSCONF_DIR: &str = "/etc";
pub const DEFAULT_XDG_CONFIG_DIRS: &str = "/etc/xdg";
pub const DEFAULT_XDG_DATA_DIRS: &str = "/usr/local/share:/usr/share";
pub const NO_VULKAN_DRIVER: &str = "no Vulkan driver manifest in the loader's search paths (install the GPU's Vulkan driver)";

/// manifest files and dirs the loader reads, in its order
/// var reads one env variable, empty counts as unset
pub fn driver_search_paths(var: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let set = |name: &str| var(name).filter(|value| !value.is_empty());
    if let Some(replaced) = set(DRIVER_FILES_VAR).or_else(|| set(LEGACY_DRIVER_FILES_VAR)) {
        return split_list(&replaced).map(PathBuf::from).collect();
    }

    let mut paths: Vec<PathBuf> = set(ADD_DRIVER_FILES_VAR)
        .map(|added| split_list(&added).map(PathBuf::from).collect())
        .unwrap_or_default();
    let home = set("HOME").map(PathBuf::from);
    let under = |root: &Path| root.join(ICD_SUBDIR);

    let config_home = set("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    paths.extend(config_home.as_deref().map(under));
    let config_dirs = set("XDG_CONFIG_DIRS").unwrap_or_else(|| DEFAULT_XDG_CONFIG_DIRS.to_owned());
    paths.extend(split_list(&config_dirs).map(|root| under(Path::new(root))));
    paths.push(under(Path::new(SYSCONF_DIR)));

    let data_home = set("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    paths.extend(data_home.as_deref().map(under));
    let data_dirs = set("XDG_DATA_DIRS").unwrap_or_else(|| DEFAULT_XDG_DATA_DIRS.to_owned());
    paths.extend(split_list(&data_dirs).map(|root| under(Path::new(root))));

    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    paths
}

/// true if one of the paths is a manifest file or a dir holding a json manifest
pub fn has_driver_manifest(
    paths: &[PathBuf],
    is_file: impl Fn(&Path) -> bool,
    dir_has_json: impl Fn(&Path) -> bool,
) -> bool {
    paths.iter().any(|path| is_file(path) || dir_has_json(path))
}

/// true if dir has a json entry, missing or unreadable dir holds none
pub fn dir_has_json_manifest(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.path().extension().is_some_and(|extension| extension == "json"))
    })
}

fn split_list(list: &str) -> impl Iterator<Item = &str> {
    list.split(':').filter(|entry| !entry.is_empty())
}
