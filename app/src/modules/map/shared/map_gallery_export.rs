use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::channel::oneshot;
use gpui::RenderImage;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, Frame, ImageDecoder, ImageReader, RgbImage, RgbaImage};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::map_view::LatLng;
use rise_media::video_export::{self, ExportJob, ExportPreset};
use rise_platform::paths::temporary_directory;
use rise_platform::photo_library::{self, ImageContentMode, ImageDeliveryMode, ImageSpec, LibraryImage};
use smallvec::SmallVec;

use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::map::shared::map_gallery_scout::MapGalleryDraft;
use crate::modules::map::stores::map_actions::map_photos_types::{MapMediaKind, MapPhotoGrid};
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};

/// exports a gallery draft to an uploadable file
/// jpeg for images and 720p mp4 for video
/// the gallery asset is never modified or removed
pub enum MapGalleryExport {}

impl MapGalleryExport {
    pub async fn item(for_draft: &MapGalleryDraft) -> Option<MediaPickerItem> {
        let folder = temporary_directory().join("map-drafts");
        if let Err(error) = std::fs::create_dir_all(&folder) {
            tracing::debug!(target: "riseonly", %error, "map drafts folder not created");
        }
        let name = Uuid::new_v4().uuid_string().to_lowercase();

        let (path, kind) = match for_draft.media_kind {
            MapMediaKind::Image => {
                let path = folder.join(format!("{name}.jpg"));
                if !Self::write_image(for_draft, &path).await {
                    return None;
                }
                (path, MediaPickerItemKind::Image)
            }
            MapMediaKind::Video => {
                let path = folder.join(format!("{name}.mp4"));
                if !Self::write_video(&for_draft.id, &folder, &path).await {
                    return None;
                }
                (path, MediaPickerItemKind::Video)
            }
        };
        let url = Url::from_file_path(&path)?;
        let suggested_file_name = url.last_path_component();
        Some(MediaPickerItem::new(
            for_draft.id.clone(),
            kind,
            url,
            suggested_file_name,
            Some(for_draft.id.clone()),
            None,
            None,
            None,
            false,
        ))
    }

    pub fn discard(item: &MediaPickerItem) {
        let Some(path) = item.url.to_file_path() else {
            return;
        };
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::debug!(target: "riseonly", %error, "exported draft not removed");
        }
    }

    async fn write_image(draft: &MapGalleryDraft, to: &Path) -> bool {
        let spec = ImageSpec {
            width: u32::try_from(draft.width).unwrap_or(0).max(1),
            height: u32::try_from(draft.height).unwrap_or(0).max(1),
            mode: ImageContentMode::AspectFit,
            delivery: ImageDeliveryMode::HighQuality,
            allow_network: true,
        };
        let Some(image) = Self::request_final_image(&draft.id, spec).await else {
            return false;
        };
        let destination = to.to_path_buf();
        RiseAccountRuntime::current()
            .spawn_blocking(move || Self::write_jpeg(&image, &destination))
            .await
            .unwrap_or(false)
    }

    fn write_jpeg(image: &LibraryImage, to: &Path) -> bool {
        let mut rgb = Vec::with_capacity(image.bgra.len() / 4 * 3);
        for pixel in image.bgra.chunks_exact(4) {
            rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        let Some(rgb) = RgbImage::from_raw(image.width, image.height, rgb) else {
            return false;
        };
        let mut data = Vec::new();
        if JpegEncoder::new_with_quality(&mut data, 90).encode_image(&rgb).is_err() {
            return false;
        }
        Self::write_atomically(to, &data).is_ok()
    }

    fn write_atomically(to: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = to.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, to).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }

    async fn write_video(asset_id: &str, folder: &Path, to: &Path) -> bool {
        let (sender, receiver) = oneshot::channel::<Option<PathBuf>>();
        let mut sender = Some(sender);
        let started = photo_library::export_video(asset_id, folder, move |outcome| {
            let original = match outcome {
                photo_library::ExportOutcome::Exported(path) => Some(path),
                photo_library::ExportOutcome::Cancelled | photo_library::ExportOutcome::Failed(_) => None,
            };
            if let Some(sender) = sender.take() {
                let _ = sender.send(original);
            }
        });
        if let Err(reason) = started {
            tracing::debug!(target: "riseonly", %reason, "gallery clip not exported");
            return false;
        }
        let Ok(Some(original)) = receiver.await else {
            return false;
        };

        let (sender, receiver) = oneshot::channel::<bool>();
        let target = to.to_path_buf();
        let source = original.clone();
        let launched = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let probe = video_export::probe(&source)?;
                let job = ExportJob::transcode(&source, &probe, ExportPreset::Size1280x720, &target, None, None);
                video_export::start_export(
                    job,
                    Box::new(move |outcome| {
                        let _ = sender.send(matches!(outcome, video_export::ExportOutcome::Completed { .. }));
                    }),
                )
            })
            .await;
        let completed = match launched {
            Ok(Ok(_ticket)) => receiver.await.unwrap_or(false),
            Ok(Err(error)) => {
                tracing::debug!(target: "riseonly", %error, "gallery clip not compressed");
                false
            }
            Err(_) => false,
        };
        if let Err(error) = std::fs::remove_file(&original) {
            tracing::debug!(target: "riseonly", %error, "exported original not removed");
        }
        completed
    }

    async fn request_final_image(asset_id: &str, spec: ImageSpec) -> Option<LibraryImage> {
        let (sender, receiver) = oneshot::channel::<Option<LibraryImage>>();
        let mut sender = Some(sender);
        let requested = photo_library::request_image(asset_id, spec, move |delivery| {
            if !delivery.is_final() {
                return;
            }
            if let Some(sender) = sender.take() {
                let _ = sender.send(delivery.image);
            }
        });
        if let Err(reason) = requested {
            tracing::debug!(target: "riseonly", %reason, "gallery image not requested");
            return None;
        }
        receiver.await.ok().flatten()
    }
}

/// placement of loose drafts
/// a draft within the grid radius of one of the user's grids belongs to it
/// the rest cluster by the same radius and each cluster is its own pin until one is published
#[derive(Clone, PartialEq, Debug, Default)]
pub struct MapDraftLayout {
    pub by_grid: HashMap<String, Vec<MapGalleryDraft>>,
    pub loose: HashMap<String, Vec<MapGalleryDraft>>,
    pub pins: Vec<MapPhotoGrid>,
}

impl MapDraftLayout {
    pub const PIN_PREFIX: &'static str = "draft|";

    pub fn make(drafts: &[MapGalleryDraft], grids: &[MapPhotoGrid], radius_meters: f64) -> MapDraftLayout {
        let mut layout = MapDraftLayout::default();
        let mut groups: Vec<(LatLng, Vec<MapGalleryDraft>)> = Vec::new();
        let grid_spots: Vec<(&str, LatLng)> = grids
            .iter()
            .map(|grid| (grid.id.as_str(), LatLng::new(grid.latitude, grid.longitude)))
            .collect();

        for draft in drafts {
            let spot = LatLng::new(draft.latitude, draft.longitude);
            let home = grid_spots
                .iter()
                .map(|(id, grid_spot)| (*id, distance(*grid_spot, spot)))
                .filter(|(_, meters)| *meters <= radius_meters)
                .min_by(|left, right| left.1.total_cmp(&right.1));
            if let Some((home, _)) = home {
                layout.by_grid.entry(home.to_owned()).or_default().push(draft.clone());
            } else if let Some(index) = groups.iter().position(|(seed, _)| distance(*seed, spot) <= radius_meters) {
                groups[index].1.push(draft.clone());
            } else {
                groups.push((spot, vec![draft.clone()]));
            }
        }

        for (seed, members) in groups {
            let Some(cover) = members.first() else {
                continue;
            };
            let id = format!("{}{}", Self::PIN_PREFIX, cover.id);
            let pin = MapPhotoGrid {
                id: id.clone(),
                owner_id: String::new(),
                latitude: seed.latitude,
                longitude: seed.longitude,
                photo_count: i64::try_from(members.len()).unwrap_or(i64::MAX),
                cover_url: cover.display_source(),
                created_at: cover.taken_at,
                updated_at: cover.taken_at,
                is_pending: false,
                is_draft: true,
                title: String::new(),
                description: String::new(),
                cover_photo_id: None,
                sequence_number: None,
            };
            layout.loose.insert(id, members);
            layout.pins.push(pin);
        }
        layout
    }
}

fn distance(from: LatLng, to: LatLng) -> f64 {
    const EARTH_RADIUS_METERS: f64 = 6_371_008.8;
    let latitude_delta = (to.latitude - from.latitude).to_radians();
    let longitude_delta = (to.longitude - from.longitude).to_radians();
    let haversine = (latitude_delta / 2.0).sin().powi(2)
        + from.latitude.to_radians().cos() * to.latitude.to_radians().cos() * (longitude_delta / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_METERS * haversine.sqrt().min(1.0).asin()
}

/// small decode of a gallery asset for a pin or grid cell
/// the viewer loader asks for a screen sized image and a tiny thumbnail must not cost that
pub enum MapGalleryThumbnail {}

impl MapGalleryThumbnail {
    pub async fn load_local_identifier(local_identifier: &str, side: f32) -> Option<Arc<RenderImage>> {
        let pixels = (side * Self::screen_scale()).round().max(1.0) as u32;
        let spec = ImageSpec {
            width: pixels,
            height: pixels,
            mode: ImageContentMode::AspectFill,
            delivery: ImageDeliveryMode::HighQuality,
            allow_network: true,
        };
        let image = MapGalleryExport::request_final_image(local_identifier, spec).await?;
        let frame = RgbaImage::from_raw(image.width, image.height, image.bgra)?;
        Some(Self::render_image_bgra(frame))
    }

    pub async fn load_file_url(file_url: &Url, side: f32) -> Option<Arc<RenderImage>> {
        let pixels = (side * Self::screen_scale()).round().max(1.0) as u32;
        let path = file_url.to_file_path()?;
        RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let maximum = pixels.saturating_mul(2).max(1);
                if let Some(image) = Self::still_image(&path) {
                    let image = if image.width().max(image.height()) > maximum {
                        image.thumbnail(maximum, maximum)
                    } else {
                        image
                    };
                    let mut rgba = image.into_rgba8();
                    for pixel in rgba.pixels_mut() {
                        pixel.0.swap(0, 2);
                    }
                    return Some(Self::render_image_bgra(rgba));
                }
                let frame = video_export::poster(&path, 0.0, maximum).ok()?;
                let bgra = RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
                Some(Self::render_image_bgra(bgra))
            })
            .await
            .ok()
            .flatten()
    }

    fn screen_scale() -> f32 {
        1.0 / MessageSizes::device_pixel()
    }

    fn still_image(path: &Path) -> Option<DynamicImage> {
        let reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
        let mut decoder = reader.into_decoder().ok()?;
        let orientation = decoder.orientation().ok();
        let mut image = DynamicImage::from_decoder(decoder).ok()?;
        if let Some(orientation) = orientation {
            image.apply_orientation(orientation);
        }
        Some(image)
    }

    /// a bgra buffer carried in an rgba image, as a single frame gpui image
    fn render_image_bgra(bgra: RgbaImage) -> Arc<RenderImage> {
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(bgra)]);
        Arc::new(RenderImage::new(frames))
    }
}
