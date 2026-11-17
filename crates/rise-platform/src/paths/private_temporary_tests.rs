use std::path::{Path, PathBuf};

use super::private_temporary::*;

const USER: u32 = 1000;

fn directory(owner: u32, mode: u32) -> DirectoryEntry {
    DirectoryEntry {
        is_directory: true,
        owner,
        mode,
    }
}

#[test]
fn an_owned_closed_directory_is_private() {
    assert_eq!(privacy_verdict(&directory(USER, 0o700), USER), PrivacyVerdict::Private);
    assert_eq!(privacy_verdict(&directory(USER, 0o500), USER), PrivacyVerdict::Private);
}

#[test]
fn an_owned_directory_open_to_others_is_tightened() {
    for mode in [0o755, 0o750, 0o701, 0o1777] {
        assert_eq!(
            privacy_verdict(&directory(USER, mode), USER),
            PrivacyVerdict::TightenMode,
            "{mode:o}"
        );
    }
}

#[test]
fn a_directory_of_another_user_is_refused_whatever_its_mode() {
    for mode in [0o700, 0o777] {
        assert_eq!(privacy_verdict(&directory(0, mode), USER), PrivacyVerdict::Refuse);
        assert_eq!(privacy_verdict(&directory(1001, mode), USER), PrivacyVerdict::Refuse);
    }
}

#[test]
fn a_symlink_or_a_file_is_refused_even_when_owned() {
    let entry = DirectoryEntry {
        is_directory: false,
        owner: USER,
        mode: 0o700,
    };

    assert_eq!(privacy_verdict(&entry, USER), PrivacyVerdict::Refuse);
}

#[test]
fn the_cache_tmp_comes_first_and_the_runtime_directory_second() {
    let candidates = temporary_candidates(
        Some(Path::new("/run/user/1000/riseonly")),
        Some(Path::new("/home/u/.cache/riseonly")),
    );

    assert_eq!(
        candidates,
        [
            PathBuf::from("/home/u/.cache/riseonly/tmp"),
            PathBuf::from("/run/user/1000/riseonly"),
        ]
    );
}

#[test]
fn without_a_cache_directory_only_the_runtime_directory_is_tried() {
    let candidates = temporary_candidates(Some(Path::new("/run/user/1000/riseonly")), None);

    assert_eq!(candidates, [PathBuf::from("/run/user/1000/riseonly")]);
}

#[test]
fn without_a_runtime_directory_only_the_cache_tmp_is_tried() {
    let candidates = temporary_candidates(None, Some(Path::new("/home/u/.cache/riseonly")));

    assert_eq!(candidates, [PathBuf::from("/home/u/.cache/riseonly/tmp")]);
}

#[test]
fn the_first_usable_candidate_wins() {
    let candidates = temporary_candidates(
        Some(Path::new("/run/user/1000/riseonly")),
        Some(Path::new("/home/u/.cache/riseonly")),
    );

    assert_eq!(
        choose_temporary_directory(&candidates, |_| true),
        Some(PathBuf::from("/home/u/.cache/riseonly/tmp"))
    );
    assert_eq!(
        choose_temporary_directory(&candidates, |candidate| candidate.ends_with("riseonly")),
        Some(PathBuf::from("/run/user/1000/riseonly"))
    );
}

#[test]
fn no_usable_candidate_keeps_the_per_user_cache_path_never_the_shared_tmp() {
    let candidates = temporary_candidates(
        Some(Path::new("/run/user/1000/riseonly")),
        Some(Path::new("/home/u/.cache/riseonly")),
    );

    assert_eq!(
        choose_temporary_directory(&candidates, |_| false),
        Some(PathBuf::from("/home/u/.cache/riseonly/tmp"))
    );
}

#[test]
fn no_candidate_at_all_answers_none() {
    assert_eq!(choose_temporary_directory(&[], |_| true), None);
    assert!(temporary_candidates(None, None).is_empty());
}
