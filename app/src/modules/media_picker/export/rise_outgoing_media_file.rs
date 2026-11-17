use std::path::PathBuf;

use crate::modules::media_picker::export::video_note_composition::VideoNoteCaptureRecipe;

/// bytes prepared for upload, independent from the legacy optimistic websocket upload model
/// url may be replaced after deferred photos or compression
/// optimistic message identity stays the same
#[derive(Clone, Debug, PartialEq)]
pub struct RiseOutgoingMediaFile {
    pub url: PathBuf,
    pub filename: String,
    pub content_type: String,
    pub file_context: Option<String>,
    pub media_type: Option<String>,
    pub byte_count: Option<i64>,
    pub video_note: Option<VideoNoteCaptureRecipe>,
}

impl RiseOutgoingMediaFile {
    /// constructor with every optional member at its default, set the others on the public fields
    pub fn new(url: PathBuf, filename: String, content_type: String) -> Self {
        Self {
            url,
            filename,
            content_type,
            file_context: None,
            media_type: None,
            byte_count: None,
            video_note: None,
        }
    }
}
