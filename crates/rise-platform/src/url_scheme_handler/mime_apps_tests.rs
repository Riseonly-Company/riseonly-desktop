use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::mime_apps::{
    XdgFiles, XdgSearchPaths, XdgValues, default_scheme_handler, entry_is_hidden, group_values,
    has_scheme_handler,
    scheme_mime_type,
};

const ENTRY: &str = "[Desktop Entry]\nType=Application\nExec=tg %u\n";
const LISTED: &str = "[Desktop Entry]\nType=Application\nExec=tg %u\nMimeType=x-scheme-handler/tg;\n";
const CACHED: &str = "[MIME Cache]\nx-scheme-handler/tg=tg.desktop;\n";

#[derive(Default)]
struct Fake {
    files: HashMap<PathBuf, String>,
    dirs: HashSet<PathBuf>,
}

impl Fake {
    fn with(mut self, path: &str, text: &str) -> Self {
        self.files.insert(PathBuf::from(path), text.to_owned());
        self
    }

    fn with_dir(mut self, path: &str) -> Self {
        self.dirs.insert(PathBuf::from(path));
        self
    }
}

impl XdgFiles for Fake {
    fn read(&self, path: &Path) -> Option<String> {
        self.files.get(path).cloned()
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.contains(path)
    }
}

fn paths() -> XdgSearchPaths {
    XdgSearchPaths::from_values(&XdgValues {
        home: Some("/h".to_owned()),
        current_desktop: Some("GNOME:ubuntu".to_owned()),
        ..XdgValues::default()
    })
}

fn path_list(items: &[&str]) -> Vec<PathBuf> {
    items.iter().map(PathBuf::from).collect()
}

#[test]
fn unset_variables_take_the_defaults_of_the_base_directory_spec() {
    let found = paths();
    assert_eq!(found.config_home, Some(PathBuf::from("/h/.config")));
    assert_eq!(found.config_dirs, path_list(&["/etc/xdg"]));
    assert_eq!(found.data_home, Some(PathBuf::from("/h/.local/share")));
    assert_eq!(found.data_dirs, path_list(&["/usr/local/share", "/usr/share"]));
}

#[test]
fn empty_and_relative_values_do_not_count() {
    let found = XdgSearchPaths::from_values(&XdgValues {
        home: Some("/h".to_owned()),
        config_home: Some("relative".to_owned()),
        data_home: Some(String::new()),
        config_dirs: Some(String::new()),
        data_dirs: Some("/one:relative:/two".to_owned()),
        current_desktop: None,
    });
    assert_eq!(found.config_home, Some(PathBuf::from("/h/.config")));
    assert_eq!(found.data_home, Some(PathBuf::from("/h/.local/share")));
    assert_eq!(found.config_dirs, path_list(&["/etc/xdg"]));
    assert_eq!(found.data_dirs, path_list(&["/one", "/two"]));
}

#[test]
fn no_home_leaves_only_the_system_directories() {
    let found = XdgSearchPaths::from_values(&XdgValues::default());
    assert_eq!(found.config_home, None);
    assert_eq!(found.data_home, None);
    assert_eq!(found.application_dirs(), path_list(&["/usr/local/share/applications", "/usr/share/applications"]));
}

#[test]
fn lists_keep_the_order_of_the_specification() {
    let lists = paths().mime_apps_lists();
    assert_eq!(
        lists[..4],
        path_list(&[
            "/h/.config/gnome-mimeapps.list",
            "/h/.config/ubuntu-mimeapps.list",
            "/h/.config/mimeapps.list",
            "/etc/xdg/gnome-mimeapps.list",
        ])[..]
    );
    assert_eq!(lists.last(), Some(&PathBuf::from("/usr/share/applications/mimeapps.list")));
    assert!(lists.contains(&PathBuf::from("/h/.local/share/applications/mimeapps.list")));
}

#[test]
fn a_scheme_becomes_its_lowercase_mime_type() {
    assert_eq!(scheme_mime_type("TG").as_deref(), Some("x-scheme-handler/tg"));
    assert_eq!(scheme_mime_type("a+b-c.d").as_deref(), Some("x-scheme-handler/a+b-c.d"));
    assert_eq!(scheme_mime_type(""), None);
    assert_eq!(scheme_mime_type("1tg"), None);
    assert_eq!(scheme_mime_type("t/g"), None);
}

#[test]
fn group_values_read_only_their_group_and_key() {
    let text = "# note\n[Other]\nk=no.desktop\n[Group]\n k = a.desktop; b.desktop;\nother=c\n\n[Group]\nk=d.desktop\n";
    assert_eq!(group_values(text, "Group", "k"), ["a.desktop", "b.desktop", "d.desktop"]);
    assert!(group_values(text, "Missing", "k").is_empty());
}

#[test]
fn hidden_counts_only_in_the_desktop_entry_group() {
    assert!(entry_is_hidden("[Desktop Entry]\nHidden=true\n"));
    assert!(!entry_is_hidden("[Desktop Entry]\nHidden=false\n"));
    assert!(!entry_is_hidden("[Desktop Action a]\nHidden=true\n"));
}

#[test]
fn nothing_on_disk_is_no_handler() {
    assert!(!has_scheme_handler("tg", &paths(), &Fake::default()));
}

#[test]
fn an_invalid_scheme_is_no_handler() {
    assert!(!has_scheme_handler("t/g", &paths(), &Fake::default()));
}

#[test]
fn a_default_needs_an_installed_entry_associated_with_the_type() {
    let list = "[Default Applications]\nx-scheme-handler/tg=tg.desktop\n";
    let associated = Fake::default()
        .with("/h/.config/mimeapps.list", list)
        .with("/usr/share/applications/tg.desktop", LISTED);
    assert!(has_scheme_handler("tg", &paths(), &associated));

    let unassociated = Fake::default()
        .with("/h/.config/mimeapps.list", list)
        .with("/usr/share/applications/tg.desktop", ENTRY);
    assert!(!has_scheme_handler("tg", &paths(), &unassociated));

    let uninstalled = Fake::default().with("/h/.config/mimeapps.list", list);
    assert!(!has_scheme_handler("tg", &paths(), &uninstalled));
}

#[test]
fn another_scheme_in_the_list_is_no_handler() {
    let files = Fake::default()
        .with("/h/.config/mimeapps.list", "[Added Associations]\nx-scheme-handler/tgx=tg.desktop\n")
        .with("/usr/share/applications/tg.desktop", LISTED);
    assert!(!has_scheme_handler("tg", &paths(), &files));
}

#[test]
fn a_desktop_specific_list_names_defaults_but_adds_nothing() {
    let added = Fake::default()
        .with("/h/.config/gnome-mimeapps.list", "[Added Associations]\nx-scheme-handler/tg=tg.desktop;\n")
        .with("/usr/share/applications/tg.desktop", ENTRY);
    assert!(!has_scheme_handler("tg", &paths(), &added));

    let default = Fake::default()
        .with("/h/.config/gnome-mimeapps.list", "[Default Applications]\nx-scheme-handler/tg=tg.desktop;\n")
        .with("/usr/share/applications/tg.desktop", LISTED);
    assert!(has_scheme_handler("tg", &paths(), &default));
}

#[test]
fn an_added_association_in_the_user_list_is_a_handler() {
    let files = Fake::default()
        .with("/h/.config/mimeapps.list", "[Added Associations]\nx-scheme-handler/tg=tg.desktop;\n")
        .with("/h/.local/share/applications/tg.desktop", ENTRY);
    assert!(has_scheme_handler("tg", &paths(), &files));
}

#[test]
fn an_addition_ignores_a_file_only_above_its_level() {
    let files = Fake::default()
        .with("/usr/share/applications/mimeapps.list", "[Added Associations]\nx-scheme-handler/tg=tg.desktop;\n")
        .with("/h/.local/share/applications/tg.desktop", ENTRY);
    assert!(!has_scheme_handler("tg", &paths(), &files));
}

#[test]
fn the_cache_of_an_applications_directory_is_a_handler() {
    let files = Fake::default()
        .with("/usr/share/applications/mimeinfo.cache", CACHED)
        .with("/usr/share/applications/tg.desktop", LISTED);
    assert!(has_scheme_handler("tg", &paths(), &files));
}

#[test]
fn a_stale_cache_is_no_handler() {
    let files = Fake::default().with("/usr/share/applications/mimeinfo.cache", CACHED);
    assert!(!has_scheme_handler("tg", &paths(), &files));
}

#[test]
fn a_copy_above_shadows_what_the_cache_below_says() {
    let files = Fake::default()
        .with("/usr/share/applications/mimeinfo.cache", CACHED)
        .with("/usr/share/applications/tg.desktop", LISTED)
        .with("/h/.local/share/applications/tg.desktop", ENTRY);
    assert!(!has_scheme_handler("tg", &paths(), &files));
}

#[test]
fn a_removed_association_hides_the_cache_and_the_lists_below() {
    let removed = "[Removed Associations]\nx-scheme-handler/tg=tg.desktop;\n";
    let cached = Fake::default()
        .with("/h/.config/mimeapps.list", removed)
        .with("/usr/share/applications/mimeinfo.cache", CACHED)
        .with("/usr/share/applications/tg.desktop", LISTED);
    assert!(!has_scheme_handler("tg", &paths(), &cached));

    let added_below = Fake::default()
        .with("/h/.config/mimeapps.list", removed)
        .with("/etc/xdg/mimeapps.list", "[Added Associations]\nx-scheme-handler/tg=tg.desktop;\n")
        .with("/usr/share/applications/tg.desktop", LISTED);
    assert!(!has_scheme_handler("tg", &paths(), &added_below));
}

#[test]
fn a_removal_below_does_not_touch_an_addition_above() {
    let files = Fake::default()
        .with("/h/.config/mimeapps.list", "[Added Associations]\nx-scheme-handler/tg=tg.desktop;\n")
        .with("/etc/xdg/mimeapps.list", "[Removed Associations]\nx-scheme-handler/tg=tg.desktop;\n")
        .with("/usr/share/applications/tg.desktop", LISTED);
    assert!(has_scheme_handler("tg", &paths(), &files));
}

#[test]
fn a_hidden_entry_masks_the_copy_below_it() {
    let files = Fake::default()
        .with("/h/.config/mimeapps.list", "[Added Associations]\nx-scheme-handler/tg=tg.desktop;\n")
        .with("/h/.local/share/applications/tg.desktop", "[Desktop Entry]\nHidden=true\n")
        .with("/usr/share/applications/tg.desktop", LISTED);
    assert!(!has_scheme_handler("tg", &paths(), &files));
}

#[test]
fn a_dash_in_the_id_can_be_a_subdirectory() {
    let files = Fake::default()
        .with("/usr/share/applications/mimeinfo.cache", "[MIME Cache]\nx-scheme-handler/tg=vendor-tg.desktop;\n")
        .with_dir("/usr/share/applications/vendor")
        .with("/usr/share/applications/vendor/tg.desktop", LISTED);
    assert!(has_scheme_handler("tg", &paths(), &files));
}

#[test]
fn an_id_that_is_not_a_plain_desktop_file_is_ignored() {
    let files = Fake::default()
        .with("/usr/share/applications/mimeinfo.cache", "[MIME Cache]\nx-scheme-handler/tg=../tg.desktop;tg.sh;\n")
        .with("/usr/share/tg.desktop", LISTED)
        .with("/usr/share/applications/tg.sh", LISTED);
    assert!(!has_scheme_handler("tg", &paths(), &files));
}

const DEFAULT_TG: &str = "[Default Applications]\nx-scheme-handler/tg=gone.desktop;tg.desktop;\n";

#[test]
fn the_default_is_the_first_installed_id_that_declares_the_type() {
    let files = Fake::default()
        .with("/h/.config/mimeapps.list", DEFAULT_TG)
        .with("/h/.local/share/applications/tg.desktop", LISTED);
    assert_eq!(default_scheme_handler("tg", &paths(), &files).as_deref(), Some("tg.desktop"));
}

#[test]
fn a_default_that_does_not_declare_the_type_is_no_default() {
    let files = Fake::default()
        .with("/h/.config/mimeapps.list", DEFAULT_TG)
        .with("/h/.local/share/applications/tg.desktop", ENTRY);
    assert_eq!(default_scheme_handler("tg", &paths(), &files), None);
}

#[test]
fn an_association_alone_is_no_default() {
    let files = Fake::default()
        .with("/h/.local/share/applications/mimeinfo.cache", CACHED)
        .with("/h/.local/share/applications/tg.desktop", LISTED);
    assert!(has_scheme_handler("tg", &paths(), &files));
    assert_eq!(default_scheme_handler("tg", &paths(), &files), None);
}

#[test]
fn a_removed_id_is_no_default() {
    let list = "[Removed Associations]\nx-scheme-handler/tg=tg.desktop;\n[Default Applications]\nx-scheme-handler/tg=tg.desktop;\n";
    let files = Fake::default()
        .with("/h/.config/mimeapps.list", list)
        .with("/h/.local/share/applications/tg.desktop", LISTED);
    assert_eq!(default_scheme_handler("tg", &paths(), &files), None);
}
