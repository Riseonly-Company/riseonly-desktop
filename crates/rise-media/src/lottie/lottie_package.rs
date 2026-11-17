use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use rise_core::foundation::foundation_data::base64_encoded_string;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::ns_cache::NSCache;
use serde_json::Value;

use super::zlib::{crc32, inflate_raw};

const KEY_IN_POINT: &str = "ip";
const KEY_OUT_POINT: &str = "op";
const KEY_FRAME_RATE: &str = "fr";
const KEY_WIDTH: &str = "w";
const KEY_HEIGHT: &str = "h";
const KEY_LAYERS: &str = "layers";
const KEY_ASSETS: &str = "assets";
const KEY_ASSET_PATH: &str = "p";
const KEY_ASSET_DIRECTORY: &str = "u";
const KEY_ASSET_EMBEDDED: &str = "e";
const KEY_MANIFEST_ANIMATIONS: &str = "animations";
const KEY_MANIFEST_ID: &str = "id";
const KEY_MANIFEST_SPEED: &str = "speed";

const MANIFEST_ENTRY: &str = "manifest.json";
const ANIMATION_DIRECTORIES: [&str; 2] = ["animations", "a"];
const IMAGE_DIRECTORIES: [&str; 2] = ["images", "i"];

#[derive(Clone, Debug)]
pub struct LottieAnimation {
    json: Arc<[u8]>,
    start_frame: f64,
    end_frame: f64,
    framerate: f64,
    width: f64,
    height: f64,
}

/// error when the bytes are not a lottie document
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a Lottie animation")]
pub struct LottieAnimationDecodingError;

impl LottieAnimation {
    /// decodes the model from json, needs layers plus timing and size fields
    /// the rest stays in the json for the renderer
    pub fn from_data(data: &[u8]) -> Result<Self, LottieAnimationDecodingError> {
        let root = JSONSerialization::json_object(data).ok_or(LottieAnimationDecodingError)?;
        Self::from_root(&root, Arc::from(data))
    }

    fn from_root(root: &Value, json: Arc<[u8]>) -> Result<Self, LottieAnimationDecodingError> {
        let object = root.as_object().ok_or(LottieAnimationDecodingError)?;
        if !object.get(KEY_LAYERS).is_some_and(Value::is_array) {
            return Err(LottieAnimationDecodingError);
        }
        let number = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_f64)
                .ok_or(LottieAnimationDecodingError)
        };
        Ok(Self {
            start_frame: number(KEY_IN_POINT)?,
            end_frame: number(KEY_OUT_POINT)?,
            framerate: number(KEY_FRAME_RATE)?,
            width: number(KEY_WIDTH)?,
            height: number(KEY_HEIGHT)?,
            json,
        })
    }

    /// the json the animation was decoded from, what rlottie parses
    pub fn json_data(&self) -> &Arc<[u8]> {
        &self.json
    }

    /// first frame number
    pub fn start_frame(&self) -> f64 {
        self.start_frame
    }

    /// last frame number
    pub fn end_frame(&self) -> f64 {
        self.end_frame
    }

    /// frame rate of the animation
    pub fn framerate(&self) -> f64 {
        self.framerate
    }

    /// duration in seconds from first to last frame
    pub fn duration(&self) -> f64 {
        if self.framerate > 0.0 {
            (self.end_frame - self.start_frame) / self.framerate
        } else {
            0.0
        }
    }

    /// composition width and height in points
    pub fn size(&self) -> (f64, f64) {
        (self.width, self.height)
    }
}

/// manifest fields a caller reads
#[derive(Clone, Debug, PartialEq)]
pub struct DotLottieConfiguration {
    pub id: String,
    pub speed: f64,
}

#[derive(Clone, Debug)]
pub struct DotLottieFileAnimation {
    pub animation: LottieAnimation,
    pub configuration: DotLottieConfiguration,
}

/// animations of one lottie archive in manifest order
#[derive(Clone, Debug)]
pub struct DotLottieFile {
    pub animations: Arc<[DotLottieFileAnimation]>,
}

/// error when loading a dotlottie file fails
#[derive(Debug, thiserror::Error)]
pub enum DotLottieError {
    #[error("the archive could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a dotLottie archive")]
    InvalidArchive,
    #[error("the archive has no manifest")]
    MissingManifest,
    #[error("the archive has no animation")]
    NoAnimation,
}

impl DotLottieFile {
    pub fn loaded_from(filepath: &Path) -> Result<Self, DotLottieError> {
        let archive = std::fs::read(filepath)?;
        Self::from_archive(&archive)
    }

    fn from_archive(archive: &[u8]) -> Result<Self, DotLottieError> {
        let entries = ZipArchive::read(archive).ok_or(DotLottieError::InvalidArchive)?;
        let manifest = entries
            .data(archive, MANIFEST_ENTRY)
            .and_then(|data| JSONSerialization::json_object(&data))
            .ok_or(DotLottieError::MissingManifest)?;
        let declared = manifest
            .get(KEY_MANIFEST_ANIMATIONS)
            .and_then(Value::as_array)
            .ok_or(DotLottieError::MissingManifest)?;
        let mut animations = Vec::with_capacity(declared.len());
        for item in declared {
            let Some(id) = item.get(KEY_MANIFEST_ID).and_then(Value::as_str) else {
                continue;
            };
            let speed = item
                .get(KEY_MANIFEST_SPEED)
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            let Some(animation) = Self::animation(&entries, archive, id) else {
                continue;
            };
            animations.push(DotLottieFileAnimation {
                animation,
                configuration: DotLottieConfiguration {
                    id: id.to_owned(),
                    speed,
                },
            });
        }
        if animations.is_empty() {
            return Err(DotLottieError::NoAnimation);
        }
        Ok(Self {
            animations: animations.into(),
        })
    }

    fn animation(entries: &ZipArchive, archive: &[u8], id: &str) -> Option<LottieAnimation> {
        let data = ANIMATION_DIRECTORIES
            .iter()
            .find_map(|directory| entries.data(archive, &format!("{directory}/{id}.json")))?;
        let mut root = JSONSerialization::json_object(&data)?;
        if !Self::embed_images(&mut root, entries, archive) {
            return LottieAnimation::from_root(&root, Arc::from(data)).ok();
        }
        let json = JSONSerialization::data_with_json_object(&root, JSONWritingOptions::default())?;
        LottieAnimation::from_root(&root, Arc::from(json)).ok()
    }

    fn embed_images(root: &mut Value, entries: &ZipArchive, archive: &[u8]) -> bool {
        let Some(assets) = root.get_mut(KEY_ASSETS).and_then(Value::as_array_mut) else {
            return false;
        };
        let mut changed = false;
        for asset in assets.iter_mut() {
            let Some(object) = asset.as_object_mut() else {
                continue;
            };
            if object.get(KEY_ASSET_EMBEDDED).and_then(Value::as_i64) == Some(1) {
                continue;
            }
            let Some(name) = object.get(KEY_ASSET_PATH).and_then(Value::as_str) else {
                continue;
            };
            let Some(data) = IMAGE_DIRECTORIES
                .iter()
                .find_map(|directory| entries.data(archive, &format!("{directory}/{name}")))
            else {
                continue;
            };
            let uri = format!(
                "data:{};base64,{}",
                Self::image_media_type(name),
                base64_encoded_string(&data)
            );
            object.insert(KEY_ASSET_PATH.to_owned(), Value::String(uri));
            object.insert(KEY_ASSET_DIRECTORY.to_owned(), Value::String(String::new()));
            object.insert(KEY_ASSET_EMBEDDED.to_owned(), Value::from(1));
            changed = true;
        }
        changed
    }

    fn image_media_type(name: &str) -> &'static str {
        let extension = name
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => "image/png",
        }
    }
}

/// parsed archives by key
pub struct DotLottieCache {
    files: NSCache<String, DotLottieFile>,
}

impl DotLottieCache {
    const DEFAULT_CACHE_COUNT_LIMIT: usize = 100;

    pub fn shared_cache() -> &'static DotLottieCache {
        static SHARED: LazyLock<DotLottieCache> = LazyLock::new(|| {
            let files = NSCache::new();
            files.set_count_limit(DotLottieCache::DEFAULT_CACHE_COUNT_LIMIT);
            DotLottieCache { files }
        });
        &SHARED
    }

    /// archive for a key if cached
    pub fn file(&self, for_key: &str) -> Option<DotLottieFile> {
        self.files.object(&for_key.to_owned())
    }

    pub fn set_file(&self, file: DotLottieFile, for_key: &str) {
        self.files.set_object(file, for_key.to_owned());
    }

    pub fn clear_cache(&self) {
        self.files.remove_all_objects();
    }
}

/// one stored or deflated entry found through the central directory
struct ZipEntry {
    method: u16,
    checksum: u32,
    data_offset: usize,
    compressed_size: usize,
    expanded_size: usize,
}

/// central directory of a plain archive
/// zip64 and encryption and unknown compression are refused since the app archives never use them
struct ZipArchive {
    entries: HashMap<String, ZipEntry>,
}

impl ZipArchive {
    const END_OF_DIRECTORY_SIGNATURE: u32 = 0x0605_4b50;
    const DIRECTORY_SIGNATURE: u32 = 0x0201_4b50;
    const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
    const END_OF_DIRECTORY_BYTES: usize = 22;
    const MAXIMUM_COMMENT_BYTES: usize = 0xFFFF;

    fn read(archive: &[u8]) -> Option<Self> {
        let end = Self::end_of_directory(archive)?;
        let count = usize::from(u16_at(archive, end + 10)?);
        let mut offset = usize::try_from(u32_at(archive, end + 16)?).ok()?;
        let mut entries = HashMap::with_capacity(count);
        for _ in 0..count {
            if u32_at(archive, offset)? != Self::DIRECTORY_SIGNATURE {
                return None;
            }
            let flags = u16_at(archive, offset + 8)?;
            let method = u16_at(archive, offset + 10)?;
            let checksum = u32_at(archive, offset + 16)?;
            let compressed_size = usize::try_from(u32_at(archive, offset + 20)?).ok()?;
            let expanded_size = usize::try_from(u32_at(archive, offset + 24)?).ok()?;
            let name_size = usize::from(u16_at(archive, offset + 28)?);
            let extra_size = usize::from(u16_at(archive, offset + 30)?);
            let comment_size = usize::from(u16_at(archive, offset + 32)?);
            let local = usize::try_from(u32_at(archive, offset + 42)?).ok()?;
            let name_bytes = archive.get(offset + 46..offset + 46 + name_size)?;
            offset += 46 + name_size + extra_size + comment_size;
            if flags & 0x0001 != 0 || (method != 0 && method != 8) {
                continue;
            }
            let Ok(name) = std::str::from_utf8(name_bytes) else {
                continue;
            };
            if u32_at(archive, local)? != Self::LOCAL_SIGNATURE {
                return None;
            }
            let local_name = usize::from(u16_at(archive, local + 26)?);
            let local_extra = usize::from(u16_at(archive, local + 28)?);
            let data_offset = local + 30 + local_name + local_extra;
            if archive.len() < data_offset.checked_add(compressed_size)? {
                return None;
            }
            entries.insert(
                name.to_owned(),
                ZipEntry {
                    method,
                    checksum,
                    data_offset,
                    compressed_size,
                    expanded_size,
                },
            );
        }
        Some(Self { entries })
    }

    fn end_of_directory(archive: &[u8]) -> Option<usize> {
        let last = archive.len().checked_sub(Self::END_OF_DIRECTORY_BYTES)?;
        let first = last.saturating_sub(Self::MAXIMUM_COMMENT_BYTES);
        (first..=last)
            .rev()
            .find(|&offset| u32_at(archive, offset) == Some(Self::END_OF_DIRECTORY_SIGNATURE))
    }

    fn data(&self, archive: &[u8], name: &str) -> Option<Vec<u8>> {
        let entry = self.entries.get(name)?;
        let compressed = archive.get(entry.data_offset..entry.data_offset + entry.compressed_size)?;
        let output = match entry.method {
            0 if entry.compressed_size == entry.expanded_size => compressed.to_vec(),
            8 => inflate_raw(compressed, entry.expanded_size)?,
            _ => return None,
        };
        (crc32(&output) == entry.checksum).then_some(output)
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let pair = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([pair[0], pair[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let quad = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]))
}
