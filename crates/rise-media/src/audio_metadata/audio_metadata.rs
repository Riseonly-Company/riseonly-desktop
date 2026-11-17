use std::path::Path;

use rise_platform::PlatformSupport;

/// the itunes tags written to the exported file, title and artist and album artist and a jpeg cover
/// a field left none or empty is not written
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct M4aMetadata {
    pub song_name: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub cover_art_jpeg: Option<Vec<u8>>,
}

impl M4aMetadata {
    /// the items in the order they are appended, empty strings and an empty cover are skipped
    pub fn items(&self) -> Vec<M4aMetadataItem<'_>> {
        let mut items = Vec::new();
        if let Some(value) = self.song_name.as_deref().filter(|value| !value.is_empty()) {
            items.push(M4aMetadataItem::Text(ITUNES_SONG_NAME, value));
        }
        if let Some(value) = self.artist.as_deref().filter(|value| !value.is_empty()) {
            items.push(M4aMetadataItem::Text(ITUNES_ARTIST, value));
        }
        if let Some(value) = self.album_artist.as_deref().filter(|value| !value.is_empty()) {
            items.push(M4aMetadataItem::Text(ITUNES_ALBUM_ARTIST, value));
        }
        if let Some(value) = self.cover_art_jpeg.as_deref().filter(|value| !value.is_empty()) {
            items.push(M4aMetadataItem::CoverJpeg(value));
        }
        items
    }

    /// true when the metadata replaces the source tags wholesale
    /// an empty array leaves the source tags as they are
    pub fn replaces_source_tags(&self) -> bool {
        !self.items().is_empty()
    }
}

/// one itunes ilst entry
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum M4aMetadataItem<'a> {
    Text([u8; 4], &'a str),
    CoverJpeg(&'a [u8]),
}

pub const ITUNES_SONG_NAME: [u8; 4] = [0xA9, b'n', b'a', b'm'];
pub const ITUNES_ARTIST: [u8; 4] = [0xA9, b'A', b'R', b'T'];
pub const ITUNES_ALBUM_ARTIST: [u8; 4] = *b"aART";
pub const ITUNES_COVER_ART: [u8; 4] = *b"covr";

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AudioMetadataError {
    #[error("the audio export is unavailable on this host")]
    Unavailable,
    #[error("the audio export was cancelled")]
    Cancelled,
    #[error("{0}")]
    Failed(String),
}

pub trait AudioMetadataWriter {
    fn support(&self) -> Result<(), AudioMetadataError> {
        Err(branch_not_written())
    }

    fn export_m4a_passthrough(
        &self,
        _source: &Path,
        _destination: &Path,
        _metadata: &M4aMetadata,
    ) -> Result<(), AudioMetadataError> {
        Err(branch_not_written())
    }
}

fn branch_not_written() -> AudioMetadataError {
    AudioMetadataError::Unavailable
}

pub fn audio_metadata_support() -> PlatformSupport {
    match super::os::NativeAudioMetadata.support() {
        Ok(()) => PlatformSupport::Performed,
        Err(_) => PlatformSupport::Unsupported,
    }
}

pub fn export_m4a_passthrough(
    source: &Path,
    destination: &Path,
    metadata: &M4aMetadata,
) -> Result<(), AudioMetadataError> {
    if source == destination {
        return Err(AudioMetadataError::Failed("the export destination is the source file".to_owned()));
    }
    super::os::NativeAudioMetadata.support()?;
    match std::fs::remove_file(destination) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(AudioMetadataError::Failed(error.to_string())),
    }
    let result = super::os::NativeAudioMetadata.export_m4a_passthrough(source, destination, metadata);
    if result.is_err() {
        std::fs::remove_file(destination).ok();
    }
    result
}
