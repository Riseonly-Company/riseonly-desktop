use std::path::Path;

/// ends an open scope, run once when the access is dropped
pub type ScopeRelease = Box<dyn FnOnce()>;

pub trait SecurityScopeSource {
    /// starts accessing the security scoped resource of path, some release when the system granted access
    /// and release ends it, none when nothing was opened
    fn start(&self, path: &Path) -> Option<ScopeRelease> {
        let _ = path;
        None
    }
}

/// start accessing plus the defer that stops it, the scope stays open while this lives
/// and is stopped once on drop, only if it opened
pub struct SecurityScopedAccess {
    release: Option<ScopeRelease>,
}

impl SecurityScopedAccess {
    /// opens the scope of the file at path, not opening isnt an error
    /// the copy is then tried with the access the process already has
    pub fn start(path: &Path) -> SecurityScopedAccess {
        Self::with_source(&super::os::NativeSecurityScope, path)
    }

    /// start over a given layer
    pub fn with_source(source: &dyn SecurityScopeSource, path: &Path) -> SecurityScopedAccess {
        SecurityScopedAccess { release: source.start(path) }
    }

    /// true when the system opened a scope that the drop stops
    pub fn did_access(&self) -> bool {
        self.release.is_some()
    }
}

impl Drop for SecurityScopedAccess {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

/// free capacity for important usage of the volume holding path, in bytes
/// none when the system gave no answer and the share loader skips its free space check
pub fn volume_available_capacity_for_important_usage(path: &Path) -> Option<i64> {
    crate::crash_diagnostics::available_capacity_for_important_usage(path)
        .ok()
        .map(|bytes| i64::try_from(bytes).unwrap_or(i64::MAX))
}
