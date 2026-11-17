use std::sync::OnceLock;

use rise_canon::RiseGuideSignal;

pub struct GuideSignalBusHooks {
    pub emit: fn(RiseGuideSignal),
}

static HOOKS: OnceLock<GuideSignalBusHooks> = OnceLock::new();

impl GuideSignalBusHooks {
    /// installs the hooks once, a second call is refused and gives them back
    /// first installer wins
    pub fn set(hooks: Self) -> Result<(), Self> {
        HOOKS.set(hooks)
    }

    pub fn is_installed() -> bool {
        HOOKS.get().is_some()
    }

    pub(crate) fn emit(signal: RiseGuideSignal) {
        if let Some(hooks) = HOOKS.get() {
            (hooks.emit)(signal);
        }
    }
}
