use std::path::Path;

use super::audio_metadata::*;

pub(super) struct NativeAudioMetadata;

impl AudioMetadataWriter for NativeAudioMetadata {
    fn support(&self) -> Result<(), AudioMetadataError> {
        Ok(())
    }

    fn export_m4a_passthrough(
        &self,
        source: &Path,
        destination: &Path,
        metadata: &M4aMetadata,
    ) -> Result<(), AudioMetadataError> {
        super::m4a_ilst::rewrite(source, destination, metadata)
    }
}
