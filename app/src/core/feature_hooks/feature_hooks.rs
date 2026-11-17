use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static FEATURE_HOOKS_INSTALLED: AtomicBool = AtomicBool::new(false);

pub fn mark_feature_hooks_installed() {
    FEATURE_HOOKS_INSTALLED.store(true, Ordering::Release);
}

pub fn feature_hooks_installed() -> bool {
    FEATURE_HOOKS_INSTALLED.load(Ordering::Acquire)
}

/// a once lock with the slot name
/// first install wins, reading an empty slot after the hooks are marked installed fails a debug assert
pub struct FeatureHookSlot<T> {
    cell: OnceLock<T>,
    name: &'static str,
}

impl<T> FeatureHookSlot<T> {
    pub const fn new(name: &'static str) -> Self {
        Self { cell: OnceLock::new(), name }
    }

    /// first install wins, a second one hands the hooks back
    pub fn set(&self, hooks: T) -> Result<(), T> {
        self.cell.set(hooks)
    }

    pub fn get(&self) -> Option<&T> {
        let hooks = self.cell.get();
        debug_assert!(
            hooks.is_some() || !feature_hooks_installed(),
            "feature hook slot {} is empty after install_feature_hooks(): its <topic>_hooks_install.rs is missing",
            self.name
        );
        hooks
    }

    /// true if the slot is filled, without the debug assertion
    pub fn is_installed(&self) -> bool {
        self.cell.get().is_some()
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}
