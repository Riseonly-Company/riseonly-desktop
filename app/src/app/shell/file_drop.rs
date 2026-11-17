use std::path::{Path, PathBuf};

pub const CHAT_ATTACHMENTS_PRESET: &str = "chat_attachments";

/// max files a chat attachment drop may carry, zero refuses every drop
pub fn max_files() -> usize {
    rise_limits::preset(CHAT_ATTACHMENTS_PRESET)
        .and_then(|preset| usize::try_from(preset.max_selection_count).ok())
        .unwrap_or(0)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DropRejection {
    Empty,
    NotAFile(PathBuf),
    TooMany { offered: usize, limit: usize },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DropOutcome {
    Accepted(Vec<PathBuf>),
    Rejected(DropRejection),
}

/// asked only by a surface that takes files, it decides by mounting its drop target
/// a split window has no single route the pointer is over
pub fn evaluate(paths: &[PathBuf], is_file: impl Fn(&Path) -> bool) -> DropOutcome {
    if paths.is_empty() {
        return DropOutcome::Rejected(DropRejection::Empty);
    }

    let limit = max_files();
    if paths.len() > limit {
        return DropOutcome::Rejected(DropRejection::TooMany {
            offered: paths.len(),
            limit,
        });
    }

    if let Some(offender) = paths.iter().find(|path| !is_file(path)) {
        return DropOutcome::Rejected(DropRejection::NotAFile(offender.clone()));
    }

    DropOutcome::Accepted(paths.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(count: usize) -> Vec<PathBuf> {
        (0..count)
            .map(|n| PathBuf::from(format!("/tmp/{n}.png")))
            .collect()
    }

    fn always_a_file(_: &Path) -> bool {
        true
    }

    #[test]
    fn a_drop_within_the_limit_is_accepted_whole() {
        assert_eq!(evaluate(&files(3), always_a_file), DropOutcome::Accepted(files(3)));
    }

    #[test]
    fn an_empty_drag_is_refused_before_anything_else_is_asked() {
        assert_eq!(evaluate(&[], always_a_file), DropOutcome::Rejected(DropRejection::Empty));
    }

    #[test]
    fn too_many_files_are_refused_rather_than_silently_truncated() {
        let limit = max_files();
        assert!(limit > 0, "the chat_attachments preset is in the canon");
        let offered = limit + 1;
        assert_eq!(
            evaluate(&files(offered), always_a_file),
            DropOutcome::Rejected(DropRejection::TooMany { offered, limit })
        );
        assert!(matches!(evaluate(&files(limit), always_a_file), DropOutcome::Accepted(_)));
    }

    #[test]
    fn a_directory_is_named_in_the_rejection_so_the_user_can_be_told_which() {
        let paths = vec![PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/folder")];
        let outcome = evaluate(&paths, |path| path.extension().is_some());
        assert_eq!(outcome, DropOutcome::Rejected(DropRejection::NotAFile(PathBuf::from("/tmp/folder"))));
    }
}
