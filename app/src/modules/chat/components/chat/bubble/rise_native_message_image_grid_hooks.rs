use futures::future::LocalBoxFuture;
use futures::stream::LocalBoxStream;
use gpui::App;
use rise_engine::rise_upload::rise_upload_progress::{RiseUploadKey, RiseUploadProgress};

use crate::core::feature_hooks::FeatureHookSlot;

/// stream of one upload slot, none in an element means the slot finished and left the hub
pub type RiseUploadProgressUpdates = LocalBoxFuture<'static, LocalBoxStream<'static, Option<RiseUploadProgress>>>;

pub struct RiseNativeMessageImageGridHooks {
    pub upload_progress_updates: fn(key: RiseUploadKey, cx: &mut App) -> Option<RiseUploadProgressUpdates>,
}

pub(crate) static RISE_NATIVE_MESSAGE_IMAGE_GRID_HOOKS: FeatureHookSlot<RiseNativeMessageImageGridHooks> = FeatureHookSlot::new("RISE_NATIVE_MESSAGE_IMAGE_GRID_HOOKS");

/// installs the hooks once, a second call keeps the first ones
pub fn set_rise_native_message_image_grid_hooks(hooks: RiseNativeMessageImageGridHooks) {
    let _ = RISE_NATIVE_MESSAGE_IMAGE_GRID_HOOKS.set(hooks);
}

pub(crate) fn rise_native_message_image_grid_hooks() -> Option<&'static RiseNativeMessageImageGridHooks> {
    RISE_NATIVE_MESSAGE_IMAGE_GRID_HOOKS.get()
}

impl RiseNativeMessageImageGridHooks {
    pub fn upload_progress_updates_or_default(key: RiseUploadKey, cx: &mut App) -> Option<RiseUploadProgressUpdates> {
        rise_native_message_image_grid_hooks().and_then(|hooks| (hooks.upload_progress_updates)(key, cx))
    }
}
