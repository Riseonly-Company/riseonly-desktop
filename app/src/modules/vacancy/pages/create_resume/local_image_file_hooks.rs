use std::path::Path;

use rise_canon::RiseUploadFormat;
use rise_core::foundation::foundation_url::Url;

const LOCAL_IMAGE_FORMATS: [RiseUploadFormat; 4] = [
    RiseUploadFormat::IMAGE_PNG,
    RiseUploadFormat::IMAGE_HEIC,
    RiseUploadFormat::IMAGE_WEBP,
    RiseUploadFormat::IMAGE_JPEG,
];

/// stem plus the extension of the content type, jpeg for an unknown one
pub fn named_file(stem: &str, content_type: &str) -> String {
    let file_extension = LOCAL_IMAGE_FORMATS
        .iter()
        .find(|format| format.mime == content_type)
        .map_or(RiseUploadFormat::IMAGE_JPEG.extension, |format| format.extension);
    format!("{}.{file_extension}", if stem.is_empty() { "avatar" } else { stem })
}

/// local image format a disk extension belongs to
pub fn format_of_extension(path_extension: &str) -> Option<RiseUploadFormat> {
    let lowered = path_extension.to_lowercase();
    LOCAL_IMAGE_FORMATS
        .into_iter()
        .find(|format| format.disk_extensions.iter().any(|disk_extension| *disk_extension == lowered))
}

/// upload fields of a dropped or pasted image file, none for anything else
pub fn local_image_file(path: &Path) -> Option<(Url, String, String)> {
    let format = format_of_extension(path.extension()?.to_str()?)?;
    let local_url = Url::from_file_path(path)?;
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    Some((local_url, named_file(&stem, format.mime), format.mime.to_owned()))
}
