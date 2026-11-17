use std::fs;
use std::path::PathBuf;

use gpui::{
    App, AsyncWindowContext, ElementId, Entity, ExternalPaths, IntoElement, ObjectFit, RenderOnce, SharedString, Window,
    black, div, img, px, white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::file_dialog::{self, FileFilter, FilterLabel, OpenRequest, Selection};
use rise_ui::IconUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::progress_view_value;

use super::music_cover_placeholder::MusicCoverPlaceholder;
use super::music_track_cover_image::MusicCoverImage;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;

const MUSIC_COVER_FILE_CONTEXT: &str = "music_cover";
const FALLBACK_IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];

/// one cover is one file so the open panel or a dropped file goes straight into the preview
/// no photo grid sheet on desktop
struct MusicCoverPickerUiState {
    is_picking: bool,
}

#[derive(IntoElement)]
pub struct MusicCoverPickerUi {
    id: ElementId,
    local_cover_file: Binding<Option<SaiWsOptimisticLocalFile>>,
    cover_preview: Binding<Option<CleverImage>>,
    existing_cover_url: Option<String>,
    placeholder_label: Option<String>,
    cover_upload_progress: f64,
    show_upload_overlay: bool,
    size: f32,
    corner_radius: f32,
    picker_configuration: MediaPickerConfiguration,
}

impl MusicCoverPickerUi {
    pub fn new(
        id: impl Into<ElementId>,
        local_cover_file: Binding<Option<SaiWsOptimisticLocalFile>>,
        cover_preview: Binding<Option<CleverImage>>,
    ) -> Self {
        Self {
            id: id.into(),
            local_cover_file,
            cover_preview,
            existing_cover_url: None,
            placeholder_label: Some(String::new()),
            cover_upload_progress: 0.0,
            show_upload_overlay: false,
            size: 120.0,
            corner_radius: 16.0,
            picker_configuration: MediaPickerConfiguration::track_cover(),
        }
    }

    pub fn existing_cover_url(mut self, existing_cover_url: impl Into<Option<String>>) -> Self {
        self.existing_cover_url = existing_cover_url.into();
        self
    }

    pub fn placeholder_label(mut self, placeholder_label: impl Into<Option<String>>) -> Self {
        self.placeholder_label = placeholder_label.into();
        self
    }

    pub fn cover_upload_progress(mut self, cover_upload_progress: f64) -> Self {
        self.cover_upload_progress = cover_upload_progress;
        self
    }

    pub fn show_upload_overlay(mut self, show_upload_overlay: bool) -> Self {
        self.show_upload_overlay = show_upload_overlay;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn picker_configuration(mut self, picker_configuration: MediaPickerConfiguration) -> Self {
        self.picker_configuration = picker_configuration;
        self
    }

    fn resolved_placeholder_label(&self) -> Option<String> {
        let placeholder_label = self.placeholder_label.as_deref()?;
        let trimmed = trimming_whitespaces(placeholder_label);
        Some(if trimmed.is_empty() { tr("music_upload_cover") } else { trimmed.to_owned() })
    }

    fn placeholder_label_width(&self) -> f32 {
        let corner_inset = self.corner_radius.min(self.size / 2.0) * 0.3;
        (self.size - 2.0 * (corner_inset + 4.0)).max(0.0)
    }

    fn placeholder_glyph_size(&self) -> f32 {
        if self.resolved_placeholder_label().is_none() { 34.0 } else { 24.0 }
    }

    fn cover_placeholder(&self) -> impl IntoElement + use<> {
        let label = self.resolved_placeholder_label();
        let corner_radius = self.corner_radius;

        zstack(Alignment::Center)
            .relative()
            .size(px(self.size))
            .clip_shape(Shape::continuous(corner_radius))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(MusicCoverPlaceholder::new().corner_radius(corner_radius).shows_note(false)),
            )
            .child(
                vstack(4.0)
                    .children(IconUi::sized("photo", px(self.placeholder_glyph_size()), white(), false))
                    .children(label.map(|label| {
                        div()
                            .max_w(px(self.placeholder_label_width()))
                            .text_center()
                            .swiftui_font(Font::system_size(11.0))
                            .text_color(white().opacity(0.85))
                            .child(label)
                    })),
            )
    }

    fn cover_filter() -> FileFilter {
        FileFilter::images(MUSIC_COVER_FILE_CONTEXT)
            .unwrap_or_else(|| FileFilter::new(FilterLabel::IMAGES, FALLBACK_IMAGE_EXTENSIONS))
    }

    fn open_cover_panel(
        prompt: String,
        state: Entity<MusicCoverPickerUiState>,
        local_cover_file: Binding<Option<SaiWsOptimisticLocalFile>>,
        cover_preview: Binding<Option<CleverImage>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if state.read(cx).is_picking {
            return;
        }
        state.update(cx, |state, _| state.is_picking = true);
        let request = OpenRequest::files(Self::cover_filter()).allowing_multiple(false).with_prompt(prompt);
        let selection = file_dialog::open(cx, &request);
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                let picked = match selection.await {
                    Selection::Picked(picked) => picked.accepted.into_iter().next(),
                    Selection::Cancelled => None,
                    Selection::Failed(error) => {
                        tracing::warn!(target: "riseonly", "music cover open panel failed: {error}");
                        None
                    }
                };
                let _ = cx.update(|window, cx| {
                    state.update(cx, |state, _| state.is_picking = false);
                    if let Some(path) = picked {
                        Self::load_cover_file(path, local_cover_file, cover_preview, window, cx);
                    }
                });
            })
            .detach();
    }

    fn handle_dropped_paths(
        paths: &ExternalPaths,
        local_cover_file: Binding<Option<SaiWsOptimisticLocalFile>>,
        cover_preview: Binding<Option<CleverImage>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let filter = Self::cover_filter();
        let Some(path) = filter.partition(paths.paths().iter().cloned()).accepted.into_iter().next() else {
            return;
        };
        Self::load_cover_file(path, local_cover_file, cover_preview, window, cx);
    }

    /// decoding leaves the main thread, bindings are written together afterwards
    /// a file that doesnt decode leaves the cover as it was
    fn load_cover_file(
        path: PathBuf,
        local_cover_file: Binding<Option<SaiWsOptimisticLocalFile>>,
        cover_preview: Binding<Option<CleverImage>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(url) = Url::from_file_path(&path) else {
            return;
        };
        let filename = url.last_path_component();

        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move {
                        let bytes = fs::read(path).ok()?;
                        let image = CleverImageBitmapDecoder::decode(&bytes, None)?;
                        Some((image, bytes.len() as i64))
                    })
                    .await;
                let Some((image, byte_count)) = decoded else {
                    return;
                };

                let mut file = SaiWsOptimisticLocalFile::new(url, filename, "image/jpeg".to_owned());
                file.byte_count = Some(byte_count);
                let _ = cx.update(|window, cx| {
                    cover_preview.set(Some(image), window, cx);
                    local_cover_file.set(Some(file), window, cx);
                });
            })
            .detach();
    }
}

impl RenderOnce for MusicCoverPickerUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        let state = window.use_keyed_state(id.clone(), cx, |_, _| MusicCoverPickerUiState { is_picking: false });
        let size = self.size;
        let corner_radius = self.corner_radius;

        let cover: AnyElement = if let Some(cover_preview) = self.cover_preview.get(cx) {
            img(cover_preview)
                .size(px(size))
                .object_fit(ObjectFit::Cover)
                .clip_shape(Shape::continuous(corner_radius))
                .into_any_element()
        } else if let Some(remote) = RemoteMediaURL::optional_normalized(self.existing_cover_url.as_deref())
            .filter(|remote| !remote.is_empty())
        {
            MusicCoverImage::new(SharedString::from(format!("{id}-cover")), remote)
                .size(size)
                .corner_radius(corner_radius)
                .into_any_element()
        } else {
            self.cover_placeholder().into_any_element()
        };

        let show_upload_overlay = self.show_upload_overlay;
        let upload_progress = (self.cover_upload_progress / 100.0).clamp(0.0, 1.0);

        let prompt = self.picker_configuration.title.clone();
        let local_cover_file = self.local_cover_file;
        let cover_preview = self.cover_preview;
        let drop_cover_file = local_cover_file.clone();
        let drop_cover_preview = cover_preview.clone();

        div()
            .relative()
            .child(
                plain(SharedString::from(format!("{id}-button")))
                    .on_press(move |window, cx| {
                        Self::open_cover_panel(
                            prompt.clone(),
                            state.clone(),
                            local_cover_file.clone(),
                            cover_preview.clone(),
                            window,
                            cx,
                        );
                    })
                    .child(
                        zstack(Alignment::Center)
                            .relative()
                            .size(px(size))
                            .child(cover)
                            .when(show_upload_overlay, |stack| {
                                stack
                                    .child(
                                        div()
                                            .absolute()
                                            .inset_0()
                                            .clip_shape(Shape::continuous(corner_radius))
                                            .bg(black().opacity(0.45)),
                                    )
                                    .child(progress_view_value(upload_progress).tint(white()).w(px(72.0)))
                            }),
                    ),
            )
            .on_drop(move |paths: &ExternalPaths, window, cx| {
                Self::handle_dropped_paths(paths, drop_cover_file.clone(), drop_cover_preview.clone(), window, cx);
            })
    }
}
