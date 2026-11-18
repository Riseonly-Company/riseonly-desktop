use std::path::Path;
use std::sync::Arc;

use crate::app_info::AppIdentity;
use crate::secure_store::SecureStore;

pub(super) fn fallback_secure_store(_data_dir: &Path, _identity: &AppIdentity) -> Option<Arc<dyn SecureStore>> {
    None
}

pub(super) fn move_fallback_secrets(_data_dir: &Path, _identity: &AppIdentity, _target: &dyn SecureStore) -> usize {
    0
}
