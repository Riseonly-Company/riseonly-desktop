use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, SMOOTH_SVG_SCALE_FACTOR, SvgRenderer, WeakEntity};
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use rise_canon::RiseIdentityCanon;
use rise_canon::rise_request::map::UpdatePhotoGrid;
use rise_canon::{RiseGuideSignal, RiseLimits};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_media::map_view::LatLng;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::CleverImageBitmapEncoder;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::guides::shared::guide_sandbox_map::GuideSandboxMapScene;
use crate::modules::map::engine::rise_map_engine_models::RiseMapLoadState;
use crate::modules::map::engine::rise_map_journeys_repository::{
    MapJourneySelection, MapJourneySnapshot, MapJourneySnapshotGrid, MapJourneySnapshotPhoto,
};
use crate::modules::map::engine::rise_map_photos_repository::{RiseMapOpenGridSnapshot, RiseMapPhotosSnapshot};
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::stores::map_actions::map_interactions_hooks::map_interactions_hooks;
use crate::modules::map::stores::map_actions::map_photos_types::{MapMediaKind, MapPhoto, MapPhotoGrid};
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};

const OPTIMISTIC_TEMPORARY_PREFIX: &str = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;

impl GuideSandbox {
    const PHOTO_FOLDER_NAME: &'static str = "guide_map_photos";

    pub fn practice_grid_id() -> String {
        format!("{OPTIMISTIC_TEMPORARY_PREFIX}guide_map_grid")
    }

    pub fn practice_journey_id() -> String {
        format!("{OPTIMISTIC_TEMPORARY_PREFIX}guide_journey")
    }

    fn photo_folder() -> PathBuf {
        temporary_directory().join(Self::PHOTO_FOLDER_NAME)
    }

    pub fn is_practice_map_file(source: &str) -> bool {
        if !Self::is_armed_for_transport() || !source.starts_with("file://") {
            return false;
        }
        let Some(url) = Url::from_string(source) else {
            return false;
        };
        let Some(folder) = Url::from_file_path(&Self::photo_folder()) else {
            return false;
        };
        let prefix = format!("{}/", folder.standardized().path());
        url.standardized().path().starts_with(&prefix)
    }

    pub fn owns_map_grid(&self, id: Option<&str>) -> bool {
        let Some(id) = id else {
            return false;
        };
        self.owns_map_scene()
            && self
                .map
                .as_ref()
                .is_some_and(|scene| scene.photo_grids.iter().any(|grid| grid.id == id))
    }

    pub fn map_photos(&self, real: RiseMapPhotosSnapshot, cx: &mut App) -> RiseMapPhotosSnapshot {
        let Some(scene) = self.map.as_ref() else {
            return real;
        };
        if !self.owns_map_scene() || real.owner_id.is_some() || scene.photo_grids.is_empty() {
            return real;
        }
        let mut open_grid = real.open_grid;
        let selected_grid_id = map_interactions_hooks().and_then(|hooks| (hooks.selected_grid_id)(cx));
        if let Some(id) = selected_grid_id.as_deref()
            && let Some(photos) = scene.photos_by_grid.get(id)
        {
            open_grid = Some(RiseMapOpenGridSnapshot {
                grid_id: id.to_owned(),
                photos: photos.clone(),
                has_more: false,
                load_state: RiseMapLoadState::Loaded,
            });
        }
        let mut grids = scene.photo_grids.clone();
        grids.extend(real.grids);
        RiseMapPhotosSnapshot {
            owner_id: None,
            grids,
            open_grid,
            load_state: if real.load_state == RiseMapLoadState::Idle {
                RiseMapLoadState::Loaded
            } else {
                real.load_state
            },
            revision: real.revision.wrapping_add(scene.revision),
        }
    }

    pub fn add_practice_map_photos(
        &mut self,
        items: &[MediaPickerItem],
        place: Option<LatLng>,
        open_grid_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if !self.owns_map_scene() {
            return None;
        }
        let mut scene = self.map.clone()?;
        let owner = SaiAccountCacheScope::active_user_id().unwrap_or_default();
        let now = RiseClock::now().ms();
        let centre = place
            .or_else(|| scene.photo_grids.first().map(MapPhotoGrid::coordinate))
            .unwrap_or_else(|| self.map_panda_coordinate());
        let radius = RiseLimits::service_value("map_photo_grid_radius_m") as f64;
        let grid_id = if let Some(open) = open_grid_id.filter(|open| scene.photo_grids.iter().any(|grid| grid.id == *open)) {
            open.to_owned()
        } else if let Some(near) = scene.photo_grids.iter().find(|grid| {
            MapTrackGeometry::haversine_meters(centre.latitude, centre.longitude, grid.latitude, grid.longitude) <= radius
        }) {
            near.id.clone()
        } else {
            let id = if scene.photo_grids.is_empty() {
                Self::practice_grid_id()
            } else {
                format!("{}_{}", Self::practice_grid_id(), scene.photo_grids.len() + 1)
            };
            scene.photo_grids.insert(
                0,
                MapPhotoGrid::new(id.clone(), owner.clone(), centre.latitude, centre.longitude, 0, String::new(), now, now),
            );
            id
        };
        let added: Vec<MapPhoto> = items
            .iter()
            .filter_map(|item| {
                let is_video = item.kind == MediaPickerItemKind::Video;
                let source = if is_video {
                    item.video_payload
                        .as_ref()
                        .and_then(|payload| payload.poster_url.clone())
                        .unwrap_or_else(|| item.url.clone())
                } else {
                    item.url.clone()
                };
                let copied = Self::copy_into_practice_folder(&source)?;
                let (width, height) = Self::pixel_size(&copied);
                let duration = item
                    .video_payload
                    .as_ref()
                    .map(|payload| (payload.end_seconds - payload.start_seconds).max(0.0))
                    .unwrap_or(0.0);
                let address = Url::from_file_path(&copied)?.absolute_string().to_owned();
                let mut photo = MapPhoto::new(
                    format!("{grid_id}_photo_{}", Uuid::new_v4().uuid_string().to_lowercase()),
                    owner.clone(),
                    address.clone(),
                    address,
                    centre.latitude,
                    centre.longitude,
                    width,
                    height,
                    now,
                    now,
                    String::new(),
                );
                photo.grid_id = grid_id.clone();
                photo.media_kind = if is_video { MapMediaKind::Video } else { MapMediaKind::Image };
                photo.duration_ms = (duration * 1_000.0) as i64;
                Some(photo)
            })
            .collect();
        if added.is_empty() {
            return None;
        }
        let existing = scene.photos_by_grid.remove(&grid_id).unwrap_or_default();
        let mut photos = added;
        photos.extend(existing);
        scene.photos_by_grid.insert(grid_id.clone(), photos);
        scene.refresh_grid(&grid_id, now);
        scene.revision = scene.revision.wrapping_add(1);
        self.map = Some(scene);
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::MapPhotosAdded);
        Some(grid_id)
    }

    pub fn update_practice_grid(&mut self, body: &UpdatePhotoGrid, cx: &mut Context<Self>) -> bool {
        if !self.owns_map_grid(Some(&body.grid_id)) {
            return false;
        }
        let Some(scene) = self.map.as_mut() else {
            return false;
        };
        let Some(index) = scene.photo_grids.iter().position(|grid| grid.id == body.grid_id) else {
            return false;
        };
        let mut grid = scene.photo_grids[index].clone();
        grid.title = body.title.clone().unwrap_or_default();
        grid.description = body.description.clone().unwrap_or_default();
        if let Some(cover_id) = body.cover_photo_id.as_ref()
            && let Some(cover) = scene
                .photos_by_grid
                .get(&grid.id)
                .and_then(|photos| photos.iter().find(|photo| &photo.id == cover_id))
        {
            grid.cover_photo_id = Some(cover_id.clone());
            grid.cover_url = cover.display_url().to_owned();
        }
        grid.updated_at = RiseClock::now().ms();
        scene.photo_grids[index] = grid;
        scene.revision = scene.revision.wrapping_add(1);
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::MapGridSaved);
        true
    }

    pub fn delete_practice_map_photo(&mut self, photo_id: &str, cx: &mut Context<Self>) -> bool {
        if !self.owns_map_scene() {
            return false;
        }
        let Some(scene) = self.map.as_mut() else {
            return false;
        };
        let Some(grid_id) = scene
            .photos_by_grid
            .iter()
            .find(|(_, photos)| photos.iter().any(|photo| photo.id == photo_id))
            .map(|(grid_id, _)| grid_id.clone())
        else {
            return false;
        };
        if let Some(photos) = scene.photos_by_grid.get_mut(&grid_id) {
            photos.retain(|photo| photo.id != photo_id);
        }
        if scene.photos_by_grid.get(&grid_id).is_none_or(Vec::is_empty) {
            scene.photos_by_grid.remove(&grid_id);
            scene.photo_grids.retain(|grid| grid.id != grid_id);
        } else {
            scene.refresh_grid(&grid_id, RiseClock::now().ms());
        }
        scene.revision = scene.revision.wrapping_add(1);
        cx.notify();
        true
    }

    pub fn delete_practice_grid(&mut self, grid_id: &str, cx: &mut Context<Self>) -> bool {
        if !self.owns_map_grid(Some(grid_id)) {
            return false;
        }
        let Some(scene) = self.map.as_mut() else {
            return false;
        };
        scene.photos_by_grid.remove(grid_id);
        scene.photo_grids.retain(|grid| grid.id != grid_id);
        scene.revision = scene.revision.wrapping_add(1);
        cx.notify();
        true
    }

    pub fn prepare_practice_journey(&self, selection: &MapJourneySelection) -> Option<MapJourneySnapshot> {
        if !self.owns_map_scene() {
            return None;
        }
        let scene = self.map.as_ref()?;
        let window = selection.from_ms..=selection.until_ms;
        let grids = scene
            .photo_grids
            .iter()
            .filter(|grid| selection.grid_ids.contains(&grid.id))
            .map(|grid| MapJourneySnapshotGrid {
                id: grid.id.clone(),
                lat: grid.latitude,
                lng: grid.longitude,
                photos: scene
                    .photos_by_grid
                    .get(&grid.id)
                    .map(|photos| {
                        photos
                            .iter()
                            .filter(|photo| window.contains(&photo.taken_at))
                            .map(|photo| MapJourneySnapshotPhoto {
                                id: photo.id.clone(),
                                url: photo.url.clone(),
                                thumbnail_url: photo.thumbnail_url.clone(),
                                width: photo.width,
                                height: photo.height,
                                media_kind: photo.media_kind.raw_value().to_owned(),
                                duration_ms: photo.duration_ms,
                                taken_at: photo.taken_at,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        Some(MapJourneySnapshot {
            id: Self::practice_journey_id(),
            owner_id: SaiAccountCacheScope::active_user_id().unwrap_or_default(),
            from_ms: selection.from_ms,
            until_ms: selection.until_ms,
            polygon: selection.polygon.clone(),
            lines: Vec::new(),
            grids,
            show_heat: None,
            heat_walks: None,
        })
    }

    pub fn seed_practice_grid(&mut self, centre: LatLng, cx: &mut Context<Self>) {
        if !self.map.as_ref().is_some_and(|scene| scene.seeds_grid && scene.photo_grids.is_empty()) {
            return;
        }
        let renderer = SvgRenderer::new(Arc::clone(cx.asset_source()));
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let pictures = RiseAccountRuntime::current()
                .spawn_blocking(move || Self::render_practice_pictures(&renderer))
                .await
                .unwrap_or_default();
            this.update(cx, |sandbox, cx| sandbox.file_practice_grid(centre, pictures, cx)).ok();
        });
        self.map_tasks.push(task);
    }

    fn file_practice_grid(&mut self, centre: LatLng, pictures: Vec<Url>, cx: &mut Context<Self>) {
        let Some(scene) = self.map.as_mut() else {
            return;
        };
        if !scene.seeds_grid || !scene.photo_grids.is_empty() {
            return;
        }
        let owner = SaiAccountCacheScope::active_user_id().unwrap_or_default();
        let now = RiseClock::now().ms();
        let place = LatLng::new(centre.latitude + 0.0004, centre.longitude + 0.0003);
        let photos: Vec<MapPhoto> = pictures
            .iter()
            .enumerate()
            .map(|(index, url)| {
                let mut photo = MapPhoto::new(
                    format!("{}_photo_{}", Self::practice_grid_id(), index + 1),
                    owner.clone(),
                    url.absolute_string().to_owned(),
                    url.absolute_string().to_owned(),
                    place.latitude,
                    place.longitude,
                    720,
                    720,
                    now - index as i64 * 60_000,
                    now,
                    String::new(),
                );
                photo.grid_id = Self::practice_grid_id();
                photo
            })
            .collect();
        let Some(first) = photos.first() else {
            return;
        };
        let mut grid = MapPhotoGrid::new(
            Self::practice_grid_id(),
            owner,
            place.latitude,
            place.longitude,
            photos.len() as i64,
            first.display_url().to_owned(),
            now,
            now,
        );
        grid.title = tr("guide_scene_map_grid_title");
        scene.photo_grids = vec![grid];
        scene.photos_by_grid.insert(Self::practice_grid_id(), photos);
        scene.revision = scene.revision.wrapping_add(1);
        cx.notify();
    }

    pub fn withdraw_map_photos(&mut self, cx: &mut Context<Self>) {
        if let Some(hooks) = map_interactions_hooks() {
            let selected = self.owns_map_grid((hooks.selected_grid_id)(cx).as_deref());
            let editing = self.owns_map_grid((hooks.grid_to_edit_id)(cx).as_deref());
            (hooks.withdraw_guide_grids)(selected, editing, cx);
        }
        GuideFacts::set(RiseGuideSignal::MapPhotoPickerOpen, false, cx);
        GuideFacts::set(RiseGuideSignal::MapGridOpen, false, cx);
        GuideFacts::set(RiseGuideSignal::MapGridEditorOpen, false, cx);
        GuideFacts::set(RiseGuideSignal::MapJourneyOutlined, false, cx);
        GuideFacts::set(RiseGuideSignal::MapJourneyGridSelected, false, cx);
        let folder = Self::photo_folder();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            FileManager.remove_item(&folder).ok();
        }));
    }

    fn copy_into_practice_folder(source: &Url) -> Option<PathBuf> {
        let manager = FileManager;
        let folder = Self::photo_folder();
        manager.create_directory(&folder, true).ok();
        let extension = source.path_extension();
        let extension = if extension.is_empty() { "jpg".to_owned() } else { extension };
        let target = folder.join(format!("{}.{extension}", Uuid::new_v4().uuid_string().to_lowercase()));
        let origin = source.to_file_path()?;
        manager.copy_item(&origin, &target).ok()?;
        Some(target)
    }

    fn pixel_size(path: &Path) -> (i64, i64) {
        match image::image_dimensions(path) {
            Ok((width, height)) => (i64::from(width), i64::from(height)),
            Err(_) => (1080, 1080),
        }
    }

    fn render_practice_pictures(renderer: &SvgRenderer) -> Vec<Url> {
        let manager = FileManager;
        let folder = Self::photo_folder();
        manager.create_directory(&folder, true).ok();
        let scenes: [(&str, (f64, f64, f64), (f64, f64, f64)); 3] = [
            ("\u{1F332}", (0.55, 0.78, 0.62), (0.18, 0.45, 0.33)),
            ("\u{1F3D4}\u{FE0F}", (0.62, 0.76, 0.95), (0.28, 0.39, 0.66)),
            ("\u{1F38B}", (0.96, 0.86, 0.58), (0.62, 0.72, 0.32)),
        ];
        scenes
            .iter()
            .enumerate()
            .filter_map(|(index, (glyph, from, to))| {
                let mut document = String::new();
                write!(
                    document,
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="720" height="720" viewBox="0 0 720 720"><defs><linearGradient id="fill" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="720" y2="720"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/></linearGradient></defs><rect x="0" y="0" width="720" height="720" fill="url(#fill)"/><text x="360" y="360" text-anchor="middle" dominant-baseline="central" font-size="360">{glyph}</text></svg>"#,
                    Self::hex(*from),
                    Self::hex(*to),
                )
                .ok()?;
                let image = renderer
                    .render_single_frame(document.as_bytes(), 1.0 / SMOOTH_SVG_SCALE_FACTOR)
                    .ok()?;
                let rgb = DynamicImage::ImageRgba8(CleverImageBitmapEncoder::rgba(&image)?).into_rgb8();
                let mut data = Vec::new();
                JpegEncoder::new_with_quality(&mut data, 86).encode_image(&rgb).ok()?;
                let path = folder.join(format!("practice_{}.jpg", index + 1));
                std::fs::write(&path, &data).ok()?;
                Url::from_file_path(&path)
            })
            .collect()
    }

    fn hex((red, green, blue): (f64, f64, f64)) -> String {
        let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02X}{:02X}{:02X}", channel(red), channel(green), channel(blue))
    }
}

impl GuideSandboxMapScene {
    /// count, cover and date follow the photos like the server keeps a grid
    pub fn refresh_grid(&mut self, grid_id: &str, now: i64) {
        let Some(index) = self.photo_grids.iter().position(|grid| grid.id == grid_id) else {
            return;
        };
        let photos = self.photos_by_grid.get(grid_id).map(Vec::as_slice).unwrap_or_default();
        let mut grid = self.photo_grids[index].clone();
        grid.photo_count = photos.len() as i64;
        let cover = grid
            .cover_photo_id
            .as_ref()
            .and_then(|cover_id| photos.iter().find(|photo| &photo.id == cover_id));
        if let Some(cover) = cover {
            grid.cover_url = cover.display_url().to_owned();
        } else if !photos.iter().any(|photo| photo.display_url() == grid.cover_url) {
            grid.cover_photo_id = None;
            grid.cover_url = photos.first().map(|photo| photo.display_url().to_owned()).unwrap_or_default();
        }
        grid.updated_at = now;
        self.photo_grids[index] = grid;
    }
}
