use std::path::{Path, PathBuf};

use image::DynamicImage;
use image::imageops::FilterType;
use rise_canon::RiseLimits;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::audio_metadata::{AudioMetadataError, M4aMetadata, export_m4a_passthrough};
use rise_platform::file_manager::FileManager;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapEncoder};
use crate::modules::music::shared::utils::music::music_cover_letterbox::MusicCoverLetterbox;
use crate::modules::share_extension::shared::rise_outgoing_media::RiseOutgoingImageEncoder;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MusicTrackMetadataError {
    #[error("export unavailable")]
    ExportUnavailable,
    #[error("{0}")]
    ExportFailed(String),
}

/// writes title, artist and cover into an exported track
/// passthrough, audio stream copied byte for byte so tagging an m4a loses no quality
pub enum MusicTrackMetadataWriter {}

impl MusicTrackMetadataWriter {
    const ARTWORK_COMPRESSION: f64 = 0.85;
    const EXPORT_FAILED: &'static str = "export failed";

    /// artwork lives in moov and is read whole before playback
    /// a full res photo would bloat the file and memory
    fn maximum_artwork_edge() -> f64 {
        RiseLimits::client_value("music_track_metadata_writer_maximum_artwork_edge") as f64
    }

    /// tag edit runs on the account runtime since the media layer reads and writes the whole file
    /// errors are a metadata error or cancelled if the export was cancelled
    pub async fn write(
        source: &Path,
        destination: &Path,
        title: Option<&str>,
        artist: Option<&str>,
        artwork: Option<&CleverImage>,
    ) -> anyhow::Result<()> {
        let metadata = Self::metadata_items(title, artist, artwork);
        let source: PathBuf = source.to_path_buf();
        let destination: PathBuf = destination.to_path_buf();
        let outcome = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let _ = FileManager.remove_item(&destination);
                export_m4a_passthrough(&source, &destination, &metadata)
            })
            .await
            .map_err(|_| anyhow::Error::new(CancellationError))?;

        match outcome {
            Ok(()) => Ok(()),
            Err(AudioMetadataError::Cancelled) => Err(anyhow::Error::new(CancellationError)),
            Err(AudioMetadataError::Unavailable) => {
                Err(anyhow::Error::new(MusicTrackMetadataError::ExportUnavailable))
            }
            Err(AudioMetadataError::Failed(description)) => {
                let description = if description.is_empty() {
                    Self::EXPORT_FAILED.to_owned()
                } else {
                    description
                };
                Err(anyhow::Error::new(MusicTrackMetadataError::ExportFailed(description)))
            }
        }
    }

    fn metadata_items(
        title: Option<&str>,
        artist: Option<&str>,
        artwork: Option<&CleverImage>,
    ) -> M4aMetadata {
        let title = title.filter(|title| !title.is_empty()).map(str::to_owned);
        let artist = artist.filter(|artist| !artist.is_empty()).map(str::to_owned);
        M4aMetadata {
            song_name: title,
            album_artist: artist.clone(),
            artist,
            cover_art_jpeg: artwork.and_then(Self::artwork_jpeg_data),
        }
    }

    /// shared with the mp3 writer so mp3 and m4a get the same cover budget
    pub fn artwork_jpeg_data(original: &CleverImage) -> Option<Vec<u8>> {
        let trimmed = MusicCoverLetterbox::trimmed(original);
        let image = DynamicImage::ImageRgba8(CleverImageBitmapEncoder::rgba(&trimmed)?);
        let width = f64::from(image.width());
        let height = f64::from(image.height());
        let longest = width.max(height);
        let maximum_artwork_edge = Self::maximum_artwork_edge();
        if longest <= maximum_artwork_edge {
            return RiseOutgoingImageEncoder::white_backed_jpeg_data(&image, Self::ARTWORK_COMPRESSION);
        }
        let scale = maximum_artwork_edge / longest;
        let resized_width = ((width * scale).round() as u32).max(1);
        let resized_height = ((height * scale).round() as u32).max(1);
        let resized = image.resize_exact(resized_width, resized_height, FilterType::CatmullRom);
        RiseOutgoingImageEncoder::white_backed_jpeg_data(&resized, Self::ARTWORK_COMPRESSION)
    }
}
