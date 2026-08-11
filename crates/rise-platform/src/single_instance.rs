use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use fs4::fs_std::FileExt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SingleInstanceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub enum Instance {
    /// This process owns the app. It must serve the channel for as long as it
    /// runs, or a later launch will hand its arguments to nobody.
    Primary(PrimaryInstance),
    /// Another process already owns it and has been handed our arguments.
    HandedOff,
}

/// Decides whether this process is the app or a messenger for the running one.
///
/// A second launch writes `arguments` to the primary's inbox and answers `HandedOff`.
/// The lock is advisory, so the OS reclaims it if the primary crashes. `Err` means the
/// lock syscall failed, never contention — the caller decides whether to start unguarded.
pub fn acquire(runtime_dir: &Path, arguments: &[String]) -> Result<Instance, SingleInstanceError> {
    std::fs::create_dir_all(runtime_dir)?;

    let lock_path = runtime_dir.join("instance.lock");
    let inbox_path = runtime_dir.join("instance.inbox");

    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;

    // Contention is Ok(false); Err means the syscall itself failed.
    match FileExt::try_lock_exclusive(&lock) {
        Ok(true) => {
            // A previous run may have left a message nobody read.
            let _ = std::fs::remove_file(&inbox_path);
            Ok(Instance::Primary(PrimaryInstance {
                _lock: lock,
                inbox: inbox_path,
                stopped: Arc::new(AtomicBool::new(false)),
            }))
        }
        Ok(false) => {
            hand_off(&inbox_path, arguments)?;
            Ok(Instance::HandedOff)
        }
        // Never folded into contention: flock returns ENOLCK on NFS and SMB, so every
        // launch on a network home would report HandedOff and exit without a window.
        Err(error) => Err(SingleInstanceError::Io(error)),
    }
}

fn hand_off(inbox: &Path, arguments: &[String]) -> Result<(), SingleInstanceError> {
    let mut file = File::options().create(true).append(true).open(inbox)?;

    // One line per launch: two near-simultaneous launches must not interleave into one record.
    let line = arguments
        .iter()
        .map(|argument| argument.replace('\n', " "))
        .collect::<Vec<_>>()
        .join("\u{1f}");

    writeln!(file, "{line}")?;
    Ok(())
}

pub struct PrimaryInstance {
    _lock: File,
    inbox: PathBuf,
    stopped: Arc<AtomicBool>,
}

impl PrimaryInstance {
    /// Drains everything handed over since the last call. Non-blocking: the caller polls it.
    pub fn take_pending(&self) -> Vec<Vec<String>> {
        if self.stopped.load(Ordering::Acquire) || !self.inbox.exists() {
            return Vec::new();
        }

        let Ok(file) = File::open(&self.inbox) else {
            return Vec::new();
        };

        let handovers: Vec<Vec<String>> = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.split('\u{1f}').map(str::to_owned).collect())
            .collect();

        let _ = std::fs::remove_file(&self.inbox);
        handovers
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rise-instance-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn primary(instance: Instance) -> PrimaryInstance {
        match instance {
            Instance::Primary(primary) => primary,
            Instance::HandedOff => panic!("expected to become primary"),
        }
    }

    #[test]
    fn a_lock_syscall_that_fails_is_an_error_not_a_handover() {
        // An uncreatable lock file stands in for the network-filesystem ENOLCK case.
        let dir = runtime_dir("syscall").join("lock.file");
        std::fs::write(&dir, b"not a directory").unwrap();

        let outcome = acquire(&dir, &["riseonly".into()]);
        assert!(
            !matches!(outcome, Ok(Instance::HandedOff)),
            "a failure to lock must never be reported as another instance owning the app"
        );

        std::fs::remove_file(&dir).ok();
    }

    #[test]
    fn the_first_launch_becomes_primary() {
        let dir = runtime_dir("first");
        let instance = acquire(&dir, &["riseonly".into()]).unwrap();
        assert!(matches!(instance, Instance::Primary(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_second_launch_hands_off_instead_of_opening_the_app() {
        let dir = runtime_dir("second");
        let first = primary(acquire(&dir, &["riseonly".into()]).unwrap());

        let second = acquire(&dir, &["riseonly".into(), "riseonly://chat/42".into()]).unwrap();
        assert!(matches!(second, Instance::HandedOff));

        let pending = first.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0][1], "riseonly://chat/42");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn several_handovers_arrive_in_order_and_stay_separate() {
        let dir = runtime_dir("several");
        let first = primary(acquire(&dir, &["riseonly".into()]).unwrap());

        for index in 0..5 {
            acquire(
                &dir,
                &["riseonly".into(), format!("riseonly://post/{index}")],
            )
            .unwrap();
        }

        let pending = first.take_pending();
        assert_eq!(pending.len(), 5);
        for (index, handover) in pending.iter().enumerate() {
            assert_eq!(handover[1], format!("riseonly://post/{index}"));
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn draining_twice_does_not_replay_the_same_handover() {
        let dir = runtime_dir("drain");
        let first = primary(acquire(&dir, &["riseonly".into()]).unwrap());
        acquire(&dir, &["riseonly".into(), "riseonly://a".into()]).unwrap();

        assert_eq!(first.take_pending().len(), 1);
        assert!(
            first.take_pending().is_empty(),
            "a drained link must not open twice"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_argument_containing_a_newline_cannot_forge_a_second_handover() {
        let dir = runtime_dir("forge");
        let first = primary(acquire(&dir, &["riseonly".into()]).unwrap());

        acquire(
            &dir,
            &["riseonly".into(), "riseonly://a\nriseonly://evil".into()],
        )
        .unwrap();

        let pending = first.take_pending();
        assert_eq!(
            pending.len(),
            1,
            "a newline in an argument must not split into two launches"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stale_inbox_from_a_previous_run_is_discarded() {
        let dir = runtime_dir("stale");
        std::fs::write(dir.join("instance.inbox"), "riseonly\u{1f}riseonly://old\n").unwrap();

        let first = primary(acquire(&dir, &["riseonly".into()]).unwrap());
        assert!(
            first.take_pending().is_empty(),
            "a link nobody read last run must not open on this one"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn releasing_the_primary_lets_the_next_launch_take_over() {
        let dir = runtime_dir("release");
        let first = primary(acquire(&dir, &["riseonly".into()]).unwrap());
        drop(first);

        let second = acquire(&dir, &["riseonly".into()]).unwrap();
        assert!(
            matches!(second, Instance::Primary(_)),
            "a closed app must not lock the user out"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stopped_primary_stops_draining() {
        let dir = runtime_dir("stopped");
        let first = primary(acquire(&dir, &["riseonly".into()]).unwrap());
        first.stop();

        acquire(&dir, &["riseonly".into(), "riseonly://a".into()]).unwrap();
        assert!(first.take_pending().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}
