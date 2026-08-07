use std::path::{Path, PathBuf};

use rise_navigation::RootRoute;

/// How many files one drop may carry.
///
/// A drag from a file manager can hold a whole folder's worth. The cap is what
/// stops a slip of the wrist from queueing four hundred uploads, and it is
/// enforced by refusing the drop rather than by silently taking the first ten:
/// a user who dropped forty files and got ten would not know which ten.
pub const MAX_FILES: usize = 20;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DropRejection {
    /// The OS handed over a drag with nothing in it.
    Empty,
    /// Nothing on screen here takes files.
    NoAcceptor,
    /// A directory, a bundle, or anything else that is not a single file.
    NotAFile(PathBuf),
    TooMany {
        offered: usize,
        limit: usize,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AcceptedDrop {
    pub paths: Vec<PathBuf>,
    pub screen_id: &'static str,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DropOutcome {
    Accepted(AcceptedDrop),
    Rejected(DropRejection),
}

/// Decides what a drop onto the window means, given what is on screen.
///
/// `is_file` is passed in rather than read from the filesystem, because a policy
/// that stats a path is a policy that cannot be tested and that touches the disk
/// on the frame path. The shell hands it `Path::is_file`.
pub fn evaluate(
    paths: &[PathBuf],
    target: Option<&RootRoute>,
    is_file: impl Fn(&Path) -> bool,
) -> DropOutcome {
    if paths.is_empty() {
        return DropOutcome::Rejected(DropRejection::Empty);
    }

    let Some(target) = target.filter(|route| route.accepts_files()) else {
        return DropOutcome::Rejected(DropRejection::NoAcceptor);
    };

    if paths.len() > MAX_FILES {
        return DropOutcome::Rejected(DropRejection::TooMany {
            offered: paths.len(),
            limit: MAX_FILES,
        });
    }

    if let Some(offender) = paths.iter().find(|path| !is_file(path)) {
        return DropOutcome::Rejected(DropRejection::NotAFile(offender.clone()));
    }

    DropOutcome::Accepted(AcceptedDrop {
        paths: paths.to_vec(),
        screen_id: target.screen_id(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat() -> RootRoute {
        RootRoute::Chat {
            chat_id: "c1".into(),
        }
    }

    fn files(count: usize) -> Vec<PathBuf> {
        (0..count)
            .map(|n| PathBuf::from(format!("/tmp/{n}.png")))
            .collect()
    }

    fn always_a_file(_: &Path) -> bool {
        true
    }

    #[test]
    fn a_drop_onto_a_screen_that_takes_files_is_accepted_whole() {
        let outcome = evaluate(&files(3), Some(&chat()), always_a_file);
        assert_eq!(
            outcome,
            DropOutcome::Accepted(AcceptedDrop {
                paths: files(3),
                screen_id: "Chat",
            })
        );
    }

    #[test]
    fn a_drop_onto_a_screen_that_takes_nothing_is_refused_rather_than_queued() {
        assert_eq!(
            evaluate(&files(1), Some(&RootRoute::Settings), always_a_file),
            DropOutcome::Rejected(DropRejection::NoAcceptor)
        );
        assert_eq!(
            evaluate(&files(1), None, always_a_file),
            DropOutcome::Rejected(DropRejection::NoAcceptor)
        );
    }

    #[test]
    fn an_empty_drag_is_refused_before_anything_else_is_asked() {
        assert_eq!(
            evaluate(&[], Some(&chat()), always_a_file),
            DropOutcome::Rejected(DropRejection::Empty)
        );
    }

    #[test]
    fn too_many_files_are_refused_rather_than_silently_truncated() {
        let offered = MAX_FILES + 1;
        assert_eq!(
            evaluate(&files(offered), Some(&chat()), always_a_file),
            DropOutcome::Rejected(DropRejection::TooMany {
                offered,
                limit: MAX_FILES,
            })
        );
        assert!(matches!(
            evaluate(&files(MAX_FILES), Some(&chat()), always_a_file),
            DropOutcome::Accepted(_)
        ));
    }

    #[test]
    fn a_directory_is_named_in_the_rejection_so_the_user_can_be_told_which() {
        let paths = vec![PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/folder")];
        let outcome = evaluate(&paths, Some(&chat()), |path| path.extension().is_some());
        assert_eq!(
            outcome,
            DropOutcome::Rejected(DropRejection::NotAFile(PathBuf::from("/tmp/folder")))
        );
    }

    #[test]
    fn only_the_screens_with_a_composer_take_files() {
        let accepting: Vec<&str> = [
            RootRoute::Onboarding,
            RootRoute::SignIn,
            RootRoute::SignUp,
            RootRoute::Settings,
            RootRoute::Tab(rise_navigation::RootTab::Feed),
            chat(),
            RootRoute::ChatTopic {
                chat_id: "c".into(),
                topic_id: "t".into(),
            },
            RootRoute::Post {
                post_id: "p".into(),
            },
            RootRoute::UserProfile {
                user_id: "u".into(),
            },
        ]
        .iter()
        .filter(|route| route.accepts_files())
        .map(|route| route.screen_id())
        .collect();

        assert_eq!(accepting, vec!["Chat", "ChatTopics"]);
    }
}
