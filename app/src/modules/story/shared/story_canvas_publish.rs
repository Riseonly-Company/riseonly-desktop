use std::sync::LazyLock;

use gpui::{App, Hsla, Task};
use rise_canon::{RiseLimits, RiseStickerAssetType, RiseStoryCanvasItemKind};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::SaiWsChunkedFileUpload;
use crate::modules::auth::services::auth_token_refresh::AuthTokenRefresh;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerEditedVideo, MediaPickerItem, MediaPickerItemKind,
};
use crate::modules::media_picker::editor::editor_state::EditorImageState;
use crate::modules::media_picker::editor::editor_state_codable::EditorStateCodec;
use crate::modules::media_picker::editor::story_canvas_model::{
    StoryCanvasGeometry, StoryCanvasPhotoStore,
};
use crate::modules::media_picker::editor::story_canvas_renderer::{
    StoryCanvasRenderer, StoryCanvasVideoRenderError, StoryCanvasVideoRenderer,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryUploadWire;

pub type WireCanvas = rise_canon::rise_request::story::create_story::Canvas;
pub type WireItem = rise_canon::rise_request::story::create_story::canvas::ItemsElement;
pub type WireLinkArea = rise_canon::rise_request::story::create_story::LinkAreasElement;

static ITEMS_MAX: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("story_canvas_items_max")).unwrap_or(0));

/// what a canvas story and its links add to a publish,
/// the 15 s video when a sticker moves,
/// photo originals with the canvas document and link zones
pub enum StoryCanvasPublish {}

impl StoryCanvasPublish {
    /// editor state exported with the photo when it is a canvas
    pub fn canvas_state(item: &MediaPickerItem) -> Option<EditorImageState> {
        if item.kind != MediaPickerItemKind::Image {
            return None;
        }
        let payload = item.image_payload.as_ref()?;
        let state = payload.deferred_edit_state.clone().or_else(|| {
            payload
                .editor_state_data
                .as_deref()
                .and_then(EditorStateCodec::decode_image)
        })?;
        state.story_canvas.as_ref()?;
        Some(state)
    }

    /// a pack sticker that moves turns the canvas into a video,
    /// decided from the asset kind before any art is decoded
    pub fn animates(state: &EditorImageState) -> bool {
        state.stickers.as_deref().unwrap_or_default().iter().any(|entity| {
            entity.loops
                && entity.emoji.is_none()
                && entity.asset_type != RiseStickerAssetType::StaticImage.wire()
        })
    }

    /// canvas as an mp4 of the story length, the still render is its poster
    pub fn animated_video_item(
        item: MediaPickerItem,
        state: EditorImageState,
        cx: &mut App,
    ) -> Task<anyhow::Result<MediaPickerItem>> {
        let Some(canvas) = state.story_canvas.clone() else {
            return Task::ready(Err(StoryCanvasVideoRenderError::NoFrame.into()));
        };
        cx.spawn(async move |cx| {
            let path = StoryCanvasVideoRenderer::render(state, canvas, cx).await?;
            let url = Url::from_file_path(&path).ok_or(StoryCanvasVideoRenderError::WriterUnavailable)?;
            let duration = StoryCanvasGeometry::image_story_duration();
            let caption = item
                .image_payload
                .as_ref()
                .map(|payload| payload.caption.clone())
                .unwrap_or_default();
            let mut payload =
                MediaPickerEditedVideo::new(url.clone(), 0.0, duration, duration, false, caption);
            payload.pixel_size = StoryCanvasGeometry::RENDER_SIZE;
            payload.poster_url = Some(item.url.clone());
            payload.spatial_edit = None;
            Ok(MediaPickerItem::new(
                format!("story-canvas:{}", Uuid::new_v4().uuid_string().to_lowercase()),
                MediaPickerItemKind::Video,
                url.clone(),
                url.last_path_component(),
                None,
                None,
                Some(payload),
                None,
                false,
            ))
        })
    }

    /// uploads every photo original and returns the document
    /// none when the canvas fails server checks and then the story goes out flat as one picture
    pub fn upload_canvas(
        state: EditorImageState,
        account_user_id: String,
        account_epoch: String,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<WireCanvas>>> {
        cx.spawn(async move |cx| {
            let Some(mut canvas) = state.story_canvas.clone() else {
                return Ok(None);
            };
            for index in 0..canvas.photos.len() {
                if canvas.photos[index].file_id.is_some() {
                    continue;
                }
                let key = canvas.photos[index].photo_key.clone();
                let file = cx
                    .background_executor()
                    .spawn(async move { StoryCanvasPhotoStore::shared().upload_file(&key) })
                    .await;
                let Some(file) = file else {
                    return Ok(None);
                };
                let filename = file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                cx.update(AuthTokenRefresh::ensure_valid_access_token).await?;
                let upload = cx.update(|cx| {
                    SaiWsChunkedFileUpload::upload(
                        file,
                        filename,
                        "image/jpeg".to_owned(),
                        "story".to_owned(),
                        None,
                        Some(account_user_id.clone()),
                        Some(account_epoch.clone()),
                        None,
                        None,
                        cx,
                    )
                });
                let done = upload.await?;
                let Some(file_id) = StoryUploadWire::remote_file_id(&done) else {
                    return Ok(None);
                };
                canvas.photos[index].file_id = Some(file_id);
            }
            let mut uploaded = state;
            uploaded.story_canvas = Some(canvas);
            Ok(Self::wire_canvas(&uploaded))
        })
    }

    pub fn wire_canvas(state: &EditorImageState) -> Option<WireCanvas> {
        let canvas = state.story_canvas.as_ref()?;
        let mut items: Vec<WireItem> = Vec::new();
        for photo in &canvas.photos {
            let file_id = photo.file_id.clone()?;
            items.push(WireItem {
                center_x: Some(Self::finite(photo.center.x)),
                center_y: Some(Self::finite(photo.center.y)),
                file_id: Some(file_id),
                height: Some(photo.pixel_height),
                kind: Some(RiseStoryCanvasItemKind::Photo.wire().to_owned()),
                rotation: Some(Self::finite(photo.rotation)),
                scale: Some(Self::positive(photo.scale)),
                shape: Some(photo.shape_kind().wire().to_owned()),
                width: Some(photo.pixel_width),
                ..WireItem::default()
            });
        }
        for sticker in state.stickers.as_deref().unwrap_or_default() {
            items.push(WireItem {
                aspect_ratio: Some(Self::positive(sticker.aspect_ratio)),
                center_x: Some(Self::finite(sticker.center.x)),
                center_y: Some(Self::finite(sticker.center.y)),
                kind: Some(RiseStoryCanvasItemKind::Sticker.wire().to_owned()),
                pack_id: Some(sticker.pack_id.clone()),
                rotation: Some(Self::finite(sticker.rotation)),
                scale: Some(Self::positive(sticker.scale)),
                sticker_id: Some(sticker.sticker_id.clone()),
                ..WireItem::default()
            });
        }
        for text in &state.texts {
            items.push(WireItem {
                alignment: Some(i64::from(text.alignment)),
                center_x: Some(Self::finite(text.center.x)),
                center_y: Some(Self::finite(text.center.y)),
                color: Some(Self::hex(text.color)),
                font_size: Some(Self::positive(text.font_size)),
                kind: Some(RiseStoryCanvasItemKind::Text.wire().to_owned()),
                rotation: Some(Self::finite(text.rotation)),
                scale: Some(Self::positive(text.scale)),
                style: Some(i64::from(text.style_mode)),
                text: Some(text.text.clone()),
                ..WireItem::default()
            });
        }
        for link in state.links.as_deref().unwrap_or_default() {
            items.push(WireItem {
                center_x: Some(Self::finite(link.center.x)),
                center_y: Some(Self::finite(link.center.y)),
                kind: Some(RiseStoryCanvasItemKind::Link.wire().to_owned()),
                link_description: Some(link.summary.clone()),
                link_image_url: Some(link.image_url.clone()),
                link_preview: Some(link.preview_kind().wire().to_owned()),
                link_style: Some(link.style_kind().wire().to_owned()),
                link_title: Some(link.title.clone()),
                link_url: Some(link.url.clone()),
                rotation: Some(Self::finite(link.rotation)),
                scale: Some(Self::positive(link.scale)),
                ..WireItem::default()
            });
        }
        if items.len() > *ITEMS_MAX {
            return None;
        }
        Some(WireCanvas {
            background_bottom: Some(canvas.background_bottom.clone()),
            background_top: Some(canvas.background_top.clone()),
            items: Some(items),
            version: Some(1),
            ..WireCanvas::default()
        })
    }

    /// link zones in fractions of the published frame
    pub fn link_areas(state: &EditorImageState) -> Vec<WireLinkArea> {
        StoryCanvasRenderer::link_areas(state, StoryCanvasGeometry::RENDER_SIZE)
            .into_iter()
            .map(|(url, center_x, center_y, width, height, rotation)| WireLinkArea {
                center_x: Some(center_x),
                center_y: Some(center_y),
                height: Some(height),
                rotation: Some(rotation),
                url: Some(url),
                width: Some(width),
            })
            .collect()
    }

    /// the update request has its own nested types with the same wire shape
    pub fn reshaped<Source: Serialize, Target: DeserializeOwned>(value: &Source) -> Option<Target> {
        let data = serde_json::to_vec(value).ok()?;
        serde_json::from_slice(&data).ok()
    }

    fn finite(value: f32) -> f64 {
        let double = f64::from(value);
        if double.is_finite() { double } else { 0.0 }
    }

    fn positive(value: f32) -> f64 {
        let double = f64::from(value);
        if double.is_finite() && double > 0.0 { double } else { 1.0 }
    }

    fn hex(color: Hsla) -> String {
        let rgba = color.to_rgb();
        let channel = |value: f32| -> u8 {
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        };
        format!("#{:02X}{:02X}{:02X}", channel(rgba.r), channel(rgba.g), channel(rgba.b))
    }
}
