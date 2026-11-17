#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChatComposerCaptureMode {
    Voice,
    VideoNote,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChatComposerRecordingUIPhase {
    Idle,
    Holding,
    Locked,
}
