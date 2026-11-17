use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AsyncApp, Context, RenderImage, SMOOTH_SVG_SCALE_FACTOR, Size, SvgRenderer, WeakEntity, size};
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerBitmapLoader;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder, CleverImageBitmapEncoder};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind,
};
use crate::modules::guides::engine::rise_guides_models::RiseGuide;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::media_picker::library::photo_library_fetcher::PhotoLibraryFetcher;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;

/// photo the reader sent to panda for the replace lesson
/// its the reader's own message so hold menu offers edit and composer offers replace
#[derive(Clone, Debug, PartialEq)]
pub struct GuideSandboxPhotoScene {
    pub file_url: Option<Url>,
    pub size: Size<f32>,
    pub edits: i64,
    pub edited_at: Option<TimeInterval>,
}

impl Default for GuideSandboxPhotoScene {
    fn default() -> Self {
        Self {
            file_url: None,
            size: size(1200.0, 900.0),
            edits: 0,
            edited_at: None,
        }
    }
}

impl GuideSandbox {
    pub fn arm_photo(&mut self, cx: &mut Context<Self>) {
        self.photo = Some(GuideSandboxPhotoScene::default());
        cx.notify();
        let renderer = SvgRenderer::new(Arc::clone(cx.asset_source()));
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let file = RiseAccountRuntime::current()
                .spawn_blocking(move || GuideSandboxPhotoArt::render_landscape(&renderer))
                .await
                .ok()
                .flatten();
            this.update(cx, |sandbox, cx| {
                if !sandbox.is_armed {
                    return;
                }
                let Some(photo) = sandbox.photo.as_mut() else {
                    return;
                };
                photo.file_url = file;
                cx.notify();
                sandbox.publish_conversation(cx);
            })
            .ok();
        });
        self.scene_tasks.push(task);
    }

    pub fn arm_gallery_photo(&mut self, guide: &RiseGuide, cx: &mut Context<Self>) {
        let lights_gallery = guide
            .steps
            .iter()
            .any(|step| step.anchor == Some(RiseGuideAnchor::MediaPickerFirstThumbnail));
        if !lights_gallery {
            return;
        }
        let renderer = SvgRenderer::new(Arc::clone(cx.asset_source()));
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let path = RiseAccountRuntime::current()
                .spawn_blocking(move || GuideSandboxPhotoArt::render_landscape(&renderer))
                .await
                .ok()
                .flatten()
                .and_then(|url| url.to_file_path());
            let still_armed = this.read_with(cx, |sandbox, _| sandbox.is_armed).unwrap_or(false);
            if still_armed {
                PhotoLibraryFetcher::set_guide_gallery_photo(path);
            }
        });
        self.scene_tasks.push(task);
    }

    pub fn withdraw_gallery_photo() {
        PhotoLibraryFetcher::set_guide_gallery_photo(None);
    }

    pub fn photo_message(&self) -> GetMessageMessage {
        let viewer = AuthServicesStore::viewer_mirror();
        let scene = self.photo.clone().unwrap_or_default();
        let path = scene.file_url.as_ref().map(|url| url.absolute_string().to_owned());
        let media = MessageMediaItem {
            thumbnail_url: path.clone(),
            width: scene.size.width,
            height: scene.size.height,
            mime_type: Some("image/jpeg".to_owned()),
            local_preview_url: path,
            ..MessageMediaItem::new(Self::photo_media_id(), MessageMediaKind::Image, String::new())
        };
        let mut message = GetMessageMessage {
            chat_id: Self::chat_id(),
            sender_id: viewer.effective_user_id_for_wire.unwrap_or_default(),
            sender_name: viewer.profile_name.unwrap_or_default(),
            content_type: MessageContentType::Image,
            text: String::new(),
            created_at: Self::scene_stamp(),
            media_items: vec![media],
            ..GetMessageMessage::new(Self::photo_message_id())
        };
        message.edited_at = scene.edited_at;
        message.is_read = true;
        message
    }

    pub fn replace_photo(&mut self, item: MediaPickerItem, cx: &mut Context<Self>) {
        let Some(photo) = self.photo.as_ref() else {
            return;
        };
        let index = photo.edits + 1;
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let image = Self::replacement_image(&item, cx).await;
            let still_armed = this
                .read_with(cx, |sandbox, _| sandbox.is_armed && sandbox.photo.is_some())
                .unwrap_or(false);
            if !still_armed {
                return;
            }
            let written = match image {
                Some(image) => RiseAccountRuntime::current()
                    .spawn_blocking(move || GuideSandboxPhotoArt::write_replacement(&image, index))
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            this.update(cx, |sandbox, cx| {
                if !sandbox.is_armed {
                    return;
                }
                let Some(photo) = sandbox.photo.as_mut() else {
                    return;
                };
                if let Some((file_url, pixels)) = written {
                    photo.file_url = Some(file_url);
                    photo.size = pixels;
                }
                photo.edits += 1;
                photo.edited_at = Some(Date::now().time_interval_since_1970());
                cx.notify();
                sandbox.publish_conversation(cx);
                GuideSignalBus::emit(RiseGuideSignal::MessageMediaReplaced);
            })
            .ok();
        });
        self.scene_tasks.push(task);
    }

    async fn replacement_image(item: &MediaPickerItem, cx: &AsyncApp) -> Option<CleverImage> {
        let payload = item.image_payload.as_ref();
        if let Some(payload) = payload
            && !payload.data.is_empty()
        {
            let data = payload.data.clone();
            let decoded = RiseAccountRuntime::current()
                .spawn_blocking(move || CleverImageBitmapDecoder::decode(&data, None))
                .await
                .ok()
                .flatten();
            if decoded.is_some() {
                return decoded;
            }
        }
        let mut base: Option<CleverImage> = None;
        let sources = [Some(&item.url), payload.and_then(|payload| payload.original_image_url.as_ref())];
        for source in sources.into_iter().flatten() {
            base = MediaViewerBitmapLoader::load_into_memory(
                source.absolute_string(),
                false,
                false,
                CleverImageCachePolicy::Standard,
                cx,
            )
            .await;
            if base.is_some() {
                break;
            }
        }
        let Some(base) = base else {
            let path = payload
                .and_then(|payload| payload.optimistic_preview_url.as_ref())
                .and_then(Url::to_file_path)?;
            return RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    let data = std::fs::read(path).ok()?;
                    CleverImageBitmapDecoder::decode(&data, None)
                })
                .await
                .ok()
                .flatten();
        };
        let Some(edits) = payload
            .and_then(|payload| payload.deferred_edit_state.clone())
            .filter(|state| state.has_meaningful_edits())
        else {
            return Some(base);
        };
        RiseAccountRuntime::current()
            .spawn_blocking(move || EditorImageBake::render_edited_image(&base, &edits))
            .await
            .ok()
    }

    pub fn withdraw_photo(&mut self) {
        if self.photo.is_none() {
            return;
        }
        GuideSandboxPhotoArt::discard();
    }
}

/// an evening over hills drawn once, sky gradient with a low sun and two ridges
pub enum GuideSandboxPhotoArt {}

impl GuideSandboxPhotoArt {
    const FOLDER_NAME: &'static str = "guide-sandbox-photo";
    const WIDTH: f64 = 1200.0;
    const HEIGHT: f64 = 900.0;

    pub fn folder() -> PathBuf {
        temporary_directory().join(Self::FOLDER_NAME)
    }

    /// off the main thread only
    /// draws with the gpui svg rasterizer so the renderer is built on main and passed in
    pub fn render_landscape(renderer: &SvgRenderer) -> Option<Url> {
        let manager = FileManager;
        let folder = Self::folder();
        manager.create_directory(&folder, true).ok();
        let path = folder.join("evening.jpg");
        if manager.file_exists(&path) {
            return Url::from_file_path(&path);
        }
        let mut document = String::new();
        write!(
            document,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><defs><linearGradient id="sky" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="0" y2="{h}"><stop offset="0" stop-color="{c0}"/><stop offset="0.62" stop-color="{c1}"/><stop offset="1" stop-color="{c2}"/></linearGradient></defs><rect x="0" y="0" width="{w}" height="{h}" fill="url(#sky)"/><ellipse cx="815" cy="525" rx="95" ry="95" fill="{sun}" fill-opacity="0.95"/>"#,
            w = Self::WIDTH,
            h = Self::HEIGHT,
            c0 = Self::hex(0.24, 0.30, 0.62),
            c1 = Self::hex(0.93, 0.52, 0.47),
            c2 = Self::hex(1.00, 0.80, 0.55),
            sun = Self::hex(1.0, 0.93, 0.78),
        )
        .ok()?;
        Self::ridge(&mut document, 0.70, 70.0, &Self::hex(0.36, 0.27, 0.45));
        Self::ridge(&mut document, 0.82, 50.0, &Self::hex(0.19, 0.15, 0.29));
        document.push_str("</svg>");
        let image = renderer
            .render_single_frame(document.as_bytes(), 1.0 / SMOOTH_SVG_SCALE_FACTOR)
            .ok()?;
        let data = Self::jpeg_data(&image, 0.88)?;
        Self::write_atomically(&path, &data)?;
        Url::from_file_path(&path)
    }

    fn write_replacement(image: &RenderImage, index: i64) -> Option<(Url, Size<f32>)> {
        let manager = FileManager;
        let folder = Self::folder();
        let target = folder.join(format!("replaced-{index}.jpg"));
        let data = Self::jpeg_data(image, 0.9)?;
        manager.create_directory(&folder, true).ok();
        manager.remove_item(&target).ok();
        Self::write_atomically(&target, &data)?;
        let pixels = image.size(0);
        let file_url = Url::from_file_path(&target)?;
        Some((file_url, size(pixels.width.0 as f32, pixels.height.0 as f32)))
    }

    pub fn discard() {
        let folder = Self::folder();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            FileManager.remove_item(&folder).ok();
        }));
    }

    fn ridge(document: &mut String, base: f64, amplitude: f64, color: &str) {
        let baseline = Self::HEIGHT * base;
        let steps: i32 = 6;
        let mut path = format!("M0 {} L0 {}", Self::HEIGHT, baseline);
        for step in 1..=steps {
            let x = Self::WIDTH * f64::from(step) / f64::from(steps);
            let control_x = x - Self::WIDTH / f64::from(steps * 2);
            let lift = if step % 2 == 0 { 1.1 } else { -0.4 };
            let control_y = baseline - amplitude * lift;
            write!(path, " Q{control_x} {control_y} {x} {baseline}").ok();
        }
        write!(path, " L{} {} Z", Self::WIDTH, Self::HEIGHT).ok();
        write!(document, r#"<path d="{path}" fill="{color}"/>"#).ok();
    }

    fn hex(red: f64, green: f64, blue: f64) -> String {
        let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02X}{:02X}{:02X}", channel(red), channel(green), channel(blue))
    }

    fn jpeg_data(image: &RenderImage, compression_quality: f64) -> Option<Vec<u8>> {
        let rgb = DynamicImage::ImageRgba8(CleverImageBitmapEncoder::rgba(image)?).into_rgb8();
        let quality = (compression_quality * 100.0).round() as u8;
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, quality).encode_image(&rgb).ok()?;
        Some(data)
    }

    fn write_atomically(path: &std::path::Path, data: &[u8]) -> Option<()> {
        let staging = path.with_extension("partial");
        std::fs::write(&staging, data).ok()?;
        std::fs::rename(&staging, path).ok()
    }
}
