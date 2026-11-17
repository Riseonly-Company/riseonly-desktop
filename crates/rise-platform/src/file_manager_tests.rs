use std::path::{Path, PathBuf};

use crate::file_manager::{
    CocoaErrorCode, DirectoryEnumerationOptions, FileAttributeType, FileManager, retrying_share_conflict,
};

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "rise-file-manager-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test sandbox is writable");
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).expect("test file is writable");
}

#[test]
fn remove_item_of_a_missing_path_is_an_error() {
    let sandbox = Sandbox::new("remove-missing");
    let error = FileManager
        .remove_item(&sandbox.path("absent"))
        .expect_err("Foundation throws for a missing item");

    assert_eq!(error.code, CocoaErrorCode::FileNoSuchFile);
    assert_eq!(error.code.raw_value(), 4);
}

#[test]
fn remove_item_deletes_a_directory_tree() {
    let sandbox = Sandbox::new("remove-tree");
    let dir = sandbox.path("a/b");
    FileManager
        .create_directory(&dir, true)
        .expect("intermediate directories are created");
    write(&dir.join("f.bin"), b"x");

    FileManager
        .remove_item(&sandbox.path("a"))
        .expect("a directory is removed with its contents");
    assert!(!FileManager.file_exists(&sandbox.path("a")));
}

#[test]
fn create_directory_without_intermediates_fails_on_an_existing_directory() {
    let sandbox = Sandbox::new("create-existing");
    let dir = sandbox.path("d");
    FileManager.create_directory(&dir, false).expect("first create");

    assert!(FileManager.create_directory(&dir, true).is_ok());
    let error = FileManager
        .create_directory(&dir, false)
        .expect_err("an existing directory without intermediates is an error");
    assert_eq!(error.code, CocoaErrorCode::FileWriteFileExists);
}

#[test]
fn create_directory_over_a_file_fails_even_with_intermediates() {
    let sandbox = Sandbox::new("create-over-file");
    let file = sandbox.path("f");
    write(&file, b"x");

    let error = FileManager
        .create_directory(&file, true)
        .expect_err("a file already sits there");
    assert_eq!(error.code, CocoaErrorCode::FileWriteFileExists);
}

#[test]
fn create_directory_without_intermediates_needs_the_parent() {
    let sandbox = Sandbox::new("create-no-parent");
    let error = FileManager
        .create_directory(&sandbox.path("missing/child"), false)
        .expect_err("the parent does not exist");
    assert_eq!(error.code, CocoaErrorCode::FileNoSuchFile);
}

#[test]
fn move_item_never_replaces_the_destination() {
    let sandbox = Sandbox::new("move-exists");
    let from = sandbox.path("from");
    let to = sandbox.path("to");
    write(&from, b"new");
    write(&to, b"old");

    let error = FileManager
        .move_item(&from, &to)
        .expect_err("Foundation refuses to overwrite");
    assert_eq!(error.code, CocoaErrorCode::FileWriteFileExists);
    assert_eq!(std::fs::read(&to).expect("destination kept"), b"old");
    assert!(FileManager.file_exists(&from));
}

#[test]
fn move_item_moves_and_a_missing_source_is_an_error() {
    let sandbox = Sandbox::new("move");
    let from = sandbox.path("from");
    let to = sandbox.path("to");
    write(&from, b"payload");

    FileManager.move_item(&from, &to).expect("move succeeds");
    assert!(!FileManager.file_exists(&from));
    assert_eq!(std::fs::read(&to).expect("moved file"), b"payload");

    let error = FileManager
        .move_item(&from, &sandbox.path("again"))
        .expect_err("the source is gone");
    assert_eq!(error.code, CocoaErrorCode::FileNoSuchFile);
}

#[test]
fn copy_item_copies_a_tree_and_refuses_an_existing_destination() {
    let sandbox = Sandbox::new("copy");
    let from = sandbox.path("src");
    FileManager.create_directory(&from.join("inner"), true).expect("tree");
    write(&from.join("inner/a.txt"), b"a");
    let to = sandbox.path("dst");

    FileManager.copy_item(&from, &to).expect("copy succeeds");
    assert_eq!(std::fs::read(to.join("inner/a.txt")).expect("copied"), b"a");

    let error = FileManager
        .copy_item(&from, &to)
        .expect_err("destination exists");
    assert_eq!(error.code, CocoaErrorCode::FileWriteFileExists);

    let missing = FileManager
        .copy_item(&sandbox.path("absent"), &sandbox.path("x"))
        .expect_err("source missing");
    assert_eq!(missing.code, CocoaErrorCode::FileReadNoSuchFile);
}

#[test]
fn file_exists_reports_directories() {
    let sandbox = Sandbox::new("exists");
    let file = sandbox.path("f");
    write(&file, b"x");
    let mut is_directory = true;

    assert!(FileManager.file_exists_is_directory(&file, &mut is_directory));
    assert!(!is_directory);
    assert!(FileManager.file_exists_is_directory(&sandbox.root, &mut is_directory));
    assert!(is_directory);
    assert!(!FileManager.file_exists(&sandbox.path("absent")));
}

#[test]
fn contents_of_directory_lists_names_and_skips_hidden_on_request() {
    let sandbox = Sandbox::new("contents");
    write(&sandbox.path("visible"), b"1");
    write(&sandbox.path(".hidden"), b"2");

    let mut names = FileManager
        .contents_of_directory_at_path(&sandbox.root)
        .expect("listing");
    names.sort();
    assert_eq!(names, vec![".hidden".to_string(), "visible".to_string()]);

    let urls = FileManager
        .contents_of_directory_at(&sandbox.root, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
        .expect("listing");
    assert_eq!(urls, vec![sandbox.path("visible")]);

    let error = FileManager
        .contents_of_directory_at_path(&sandbox.path("absent"))
        .expect_err("missing directory");
    assert_eq!(error.code, CocoaErrorCode::FileReadNoSuchFile);
}

#[test]
fn attributes_of_item_reports_size_type_and_dates() {
    let sandbox = Sandbox::new("attributes");
    let file = sandbox.path("f");
    write(&file, b"12345");

    let attributes = FileManager.attributes_of_item(&file).expect("attributes");
    assert_eq!(attributes.size, 5);
    assert_eq!(attributes.r#type, FileAttributeType::TypeRegular);
    assert!(attributes.modification_date.is_some());

    let directory = FileManager
        .attributes_of_item(&sandbox.root)
        .expect("attributes");
    assert_eq!(directory.r#type, FileAttributeType::TypeDirectory);

    let error = FileManager
        .attributes_of_item(&sandbox.path("absent"))
        .expect_err("missing item");
    assert_eq!(error.code, CocoaErrorCode::FileReadNoSuchFile);
}

#[test]
fn create_file_replaces_contents() {
    let sandbox = Sandbox::new("create-file");
    let file = sandbox.path("f");
    assert!(FileManager.create_file(&file, Some(b"first")));
    assert!(FileManager.create_file(&file, None));
    assert_eq!(std::fs::read(&file).expect("file"), b"");
    assert!(!FileManager.create_file(&sandbox.path("missing/f"), None));
}

#[test]
fn cocoa_error_codes_round_trip_their_raw_values() {
    for code in [
        CocoaErrorCode::FileNoSuchFile,
        CocoaErrorCode::FileReadNoSuchFile,
        CocoaErrorCode::FileWriteFileExists,
        CocoaErrorCode::FileWriteOutOfSpace,
        CocoaErrorCode::FileReadCorruptFile,
        CocoaErrorCode::CoderReadCorrupt,
    ] {
        assert_eq!(CocoaErrorCode::from_raw(code.raw_value()), Some(code));
    }
    assert_eq!(CocoaErrorCode::from_raw(1), None);
}

#[test]
fn share_conflict_is_retried_until_the_holder_lets_go() {
    let mut calls = 0;
    let result = retrying_share_conflict(|| {
        calls += 1;
        if calls < 3 { Err(std::io::Error::from_raw_os_error(32)) } else { Ok(calls) }
    });
    assert_eq!(result.ok(), Some(3));
}

#[test]
fn share_conflict_gives_up_after_the_last_retry() {
    let mut calls = 0;
    let result: std::io::Result<()> = retrying_share_conflict(|| {
        calls += 1;
        Err(std::io::Error::from_raw_os_error(33))
    });
    assert_eq!(result.err().and_then(|error| error.raw_os_error()), Some(33));
    assert_eq!(calls, 9);
}

#[test]
fn other_errors_are_not_retried() {
    let mut calls = 0;
    let result: std::io::Result<()> = retrying_share_conflict(|| {
        calls += 1;
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    });
    assert!(result.is_err());
    assert_eq!(calls, 1);
}
