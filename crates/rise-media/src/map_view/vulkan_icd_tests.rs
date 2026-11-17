use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::vulkan_icd::*;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    move |name: &str| map.get(name).cloned()
}

fn paths(list: &[&str]) -> Vec<PathBuf> {
    list.iter().map(PathBuf::from).collect()
}

#[test]
fn standard_search_follows_the_loader_order_with_base_directory_defaults() {
    let found = driver_search_paths(env(&[("HOME", "/home/u")]));
    assert_eq!(
        found,
        paths(&[
            "/home/u/.config/vulkan/icd.d",
            "/etc/xdg/vulkan/icd.d",
            "/etc/vulkan/icd.d",
            "/home/u/.local/share/vulkan/icd.d",
            "/usr/local/share/vulkan/icd.d",
            "/usr/share/vulkan/icd.d",
        ])
    );
}

#[test]
fn xdg_variables_replace_their_defaults_and_empty_counts_as_unset() {
    let found = driver_search_paths(env(&[
        ("HOME", "/home/u"),
        ("XDG_CONFIG_HOME", ""),
        ("XDG_DATA_HOME", "/data"),
        ("XDG_DATA_DIRS", "/app/share:/usr/share:"),
    ]));
    assert_eq!(
        found,
        paths(&[
            "/home/u/.config/vulkan/icd.d",
            "/etc/xdg/vulkan/icd.d",
            "/etc/vulkan/icd.d",
            "/data/vulkan/icd.d",
            "/app/share/vulkan/icd.d",
            "/usr/share/vulkan/icd.d",
        ])
    );
}

#[test]
fn without_home_the_home_roots_are_skipped() {
    let found = driver_search_paths(env(&[]));
    assert!(found.iter().all(|path| !path.starts_with("/home")));
    assert_eq!(found.first(), Some(&PathBuf::from("/etc/xdg/vulkan/icd.d")));
}

#[test]
fn added_driver_files_come_first_and_duplicates_are_read_once() {
    let found = driver_search_paths(env(&[
        ("VK_ADD_DRIVER_FILES", "/opt/gpu/icd.json:/etc/vulkan/icd.d"),
        ("XDG_CONFIG_DIRS", "/etc"),
    ]));
    assert_eq!(found[0], PathBuf::from("/opt/gpu/icd.json"));
    assert_eq!(found[1], PathBuf::from("/etc/vulkan/icd.d"));
    assert_eq!(found.iter().filter(|path| **path == Path::new("/etc/vulkan/icd.d")).count(), 1);
}

#[test]
fn driver_files_replace_the_search_and_win_over_the_legacy_name() {
    let found = driver_search_paths(env(&[
        ("HOME", "/home/u"),
        ("VK_DRIVER_FILES", "/a.json:/b"),
        ("VK_ICD_FILENAMES", "/legacy.json"),
        ("VK_ADD_DRIVER_FILES", "/added.json"),
    ]));
    assert_eq!(found, paths(&["/a.json", "/b"]));
}

#[test]
fn legacy_name_replaces_the_search_when_the_new_one_is_unset() {
    let found = driver_search_paths(env(&[("VK_DRIVER_FILES", ""), ("VK_ICD_FILENAMES", "/legacy.json")]));
    assert_eq!(found, paths(&["/legacy.json"]));
}

#[test]
fn a_manifest_file_or_a_directory_with_json_is_a_driver() {
    let search = paths(&["/missing", "/etc/vulkan/icd.d", "/usr/share/vulkan/icd.d"]);
    assert!(has_driver_manifest(&search, |_| false, |dir| dir == Path::new("/usr/share/vulkan/icd.d")));
    assert!(has_driver_manifest(&search, |path| path == Path::new("/missing"), |_| false));
    assert!(!has_driver_manifest(&search, |_| false, |_| false));
    assert!(!has_driver_manifest(&[], |_| true, |_| true));
}

#[test]
fn a_missing_directory_holds_no_manifest() {
    assert!(!dir_has_json_manifest(Path::new("/riseonly-no-such-dir/vulkan/icd.d")));
}

#[test]
fn the_reason_names_vulkan() {
    assert!(NO_VULKAN_DRIVER.contains("Vulkan"));
}
