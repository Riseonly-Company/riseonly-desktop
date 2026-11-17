use std::sync::Arc;

use futures::future::BoxFuture;
use rise_core::foundation::foundation_url::Url;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;

/// result of the outgoing media compressor
/// declared here because sai reads it below the picker, the compressor file re-exports it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingMediaCompressorResult {
    pub url: Url,
    pub content_type: String,
    pub byte_count: i64,
    pub pixel_width: Option<i64>,
    pub pixel_height: Option<i64>,
}

/// compression progress from 0 to 1, called from the compression thread
pub type OutgoingMediaProgress = Arc<dyn Fn(f64) + Send + Sync>;

/// probe the compressor polls between stages to see if its cancelled
pub type OutgoingMediaIsCancelled = Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>;

pub struct SaiOutgoingMediaHooks {
    pub compress_if_needed: Box<
        dyn Fn(
                SaiWsOptimisticLocalFile,
                MediaCompressionConfiguration,
                Option<OutgoingMediaProgress>,
                OutgoingMediaIsCancelled,
            ) -> BoxFuture<'static, anyhow::Result<OutgoingMediaCompressorResult>>
            + Send
            + Sync,
    >,
}

pub(crate) static SAI_OUTGOING_MEDIA_HOOKS: FeatureHookSlot<SaiOutgoingMediaHooks> = FeatureHookSlot::new("SAI_OUTGOING_MEDIA_HOOKS");

/// installed once, a second install keeps the first
pub fn set_sai_outgoing_media_hooks(hooks: SaiOutgoingMediaHooks) {
    let _ = SAI_OUTGOING_MEDIA_HOOKS.set(hooks);
}

pub(crate) fn sai_outgoing_media_hooks() -> Option<&'static SaiOutgoingMediaHooks> {
    SAI_OUTGOING_MEDIA_HOOKS.get()
}

/// compresses a file if needed
/// without the picker it fails so the send reports
/// a file error instead of uploading the raw source
pub(crate) fn sai_outgoing_media_compress_if_needed(
    local_file: SaiWsOptimisticLocalFile,
    configuration: MediaCompressionConfiguration,
    on_progress: Option<OutgoingMediaProgress>,
    is_cancelled: OutgoingMediaIsCancelled,
) -> BoxFuture<'static, anyhow::Result<OutgoingMediaCompressorResult>> {
    match sai_outgoing_media_hooks() {
        Some(hooks) => {
            (hooks.compress_if_needed)(local_file, configuration, on_progress, is_cancelled)
        }
        None => Box::pin(async { Err(anyhow::Error::new(SaiOutgoingMediaCompressorUnavailable)) }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("outgoing media compressor is not installed")]
pub(crate) struct SaiOutgoingMediaCompressorUnavailable;
