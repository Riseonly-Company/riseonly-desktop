use crate::core::lib::sai::sai_outgoing_media_hooks::{SaiOutgoingMediaHooks, set_sai_outgoing_media_hooks};
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;

pub fn install_outgoing_media_compressor_hooks() {
    set_sai_outgoing_media_hooks(SaiOutgoingMediaHooks {
        compress_if_needed: Box::new(|local_file, configuration, on_progress, is_cancelled| {
            Box::pin(OutgoingMediaCompressor::compress_if_needed_optimistic(
                local_file,
                None,
                configuration,
                on_progress,
                Some(is_cancelled),
            ))
        }),
    });
}
