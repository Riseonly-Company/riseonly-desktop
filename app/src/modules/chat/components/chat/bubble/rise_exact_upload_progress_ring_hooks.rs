use futures::future::LocalBoxFuture;
use gpui::App;
use rise_engine::rise_upload::rise_upload_progress::{RiseUploadKey, RiseUploadProgress, RiseUploadProgressStream};

use crate::core::feature_hooks::FeatureHookSlot;

/// slot for the upload ring and status label to read the active conversation domain
/// and its upload progress updates
pub struct RiseExactUploadProgressRingHooks {
    pub upload_progress_updates:
        fn(&App, RiseUploadKey) -> Option<LocalBoxFuture<'static, RiseUploadProgressStream<Option<RiseUploadProgress>>>>,
}

pub(crate) static RISE_EXACT_UPLOAD_PROGRESS_RING_HOOKS: FeatureHookSlot<RiseExactUploadProgressRingHooks> = FeatureHookSlot::new("RISE_EXACT_UPLOAD_PROGRESS_RING_HOOKS");

pub fn set_rise_exact_upload_progress_ring_hooks(hooks: RiseExactUploadProgressRingHooks) {
    let _ = RISE_EXACT_UPLOAD_PROGRESS_RING_HOOKS.set(hooks);
}

pub fn rise_exact_upload_progress_ring_hooks() -> Option<&'static RiseExactUploadProgressRingHooks> {
    RISE_EXACT_UPLOAD_PROGRESS_RING_HOOKS.get()
}
