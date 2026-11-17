use std::sync::atomic::{AtomicBool, Ordering};

static SOUND_MUTED: AtomicBool = AtomicBool::new(false);

pub fn set_sound_muted(muted: bool) {
    SOUND_MUTED.store(muted, Ordering::Relaxed);
}

pub fn is_sound_muted() -> bool {
    SOUND_MUTED.load(Ordering::Relaxed)
}

/// overwrites an output block with silence while the switch is on
pub fn silence_if_muted(block: &mut [f32]) {
    if is_sound_muted() {
        block.fill(0.0);
    }
}
