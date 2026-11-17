use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, EXECUTION_STATE,
};

use super::display_sleep::*;

const HOLD: EXECUTION_STATE =
    EXECUTION_STATE(ES_CONTINUOUS.0 | ES_DISPLAY_REQUIRED.0 | ES_SYSTEM_REQUIRED.0);

pub(super) struct NativeDisplaySleep;

impl DisplaySleepInhibitor for NativeDisplaySleep {
    fn inhibit(&self, _reason: &str) -> Result<SystemInhibition, DisplaySleepUnavailable> {
        let (answer_tx, answer_rx) = mpsc::sync_channel::<bool>(1);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("rise-display-sleep".to_owned())
            .spawn(move || {
                let previous = unsafe { SetThreadExecutionState(HOLD) };
                let held = previous != EXECUTION_STATE(0);
                if answer_tx.send(held).is_err() || !held {
                    if held {
                        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
                    }
                    return;
                }
                let _ = release_rx.recv();
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            })
            .map_err(|error| {
                DisplaySleepUnavailable::SystemRefused(format!(
                    "no thread to hold SetThreadExecutionState: {error}"
                ))
            })?;
        match answer_rx.recv() {
            Ok(true) => Ok(SystemInhibition::new(HoldThread {
                release: Some(release_tx),
                thread: Some(thread),
            })),
            Ok(false) => {
                let _ = thread.join();
                Err(DisplaySleepUnavailable::SystemRefused(
                    "SetThreadExecutionState returned NULL".to_owned(),
                ))
            }
            Err(_) => {
                let _ = thread.join();
                Err(DisplaySleepUnavailable::SystemRefused(
                    "the hold thread ended before SetThreadExecutionState answered".to_owned(),
                ))
            }
        }
    }
}

struct HoldThread {
    release: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for HoldThread {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
