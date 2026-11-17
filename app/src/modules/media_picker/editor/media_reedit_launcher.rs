use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{AnyWindowHandle, App, Context, ElementId, Entity, FocusHandle, Render, SharedString, Size, Window, size};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{DynamicImage, ImageEncoder};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::paths::temporary_directory;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::top_presenter::TopPresenter;

use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapEncoder};
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_presenter_resolver::MediaPickerPresenterResolver;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerEditedImage, MediaPickerItem, MediaPickerItemKind, MediaPickerTrackSelection,
};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_state::{EditorImageState, EditorPersonTagEntity};
use crate::modules::media_picker::editor::editor_state_codable::EditorStateCodec;
use crate::modules::media_picker::editor::media_editor_view_hooks::media_editor_view_hooks;
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorOutcome;
use crate::modules::media_picker::library::media_asset::{MediaAsset, MediaAssetMediaType};
use crate::modules::media_picker::selection::media_selection_state::{
    MediaSelectionEdit, MediaSelectionEditPayload, MediaSelectionState,
};

const CROSS_DISSOLVE_SECONDS: f64 = 0.3;
const ORIGINAL_JPEG_QUALITY: u8 = 95;

pub struct MediaReeditResult {
    pub item: MediaPickerItem,
    pub caption: String,
}

type MediaReeditCompletion = Box<dyn FnOnce(Option<MediaReeditResult>, &mut App)>;

/// re-edit layers on screen, each with the window it was presented from
pub struct MediaReeditLauncherHosts {
    hosts: Vec<(AnyWindowHandle, Entity<MediaReeditHostView>)>,
}

impl MediaReeditLauncherHosts {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self { hosts: Vec::new() }
    }

    pub(crate) fn hosts(&self, window: &Window) -> Vec<Entity<MediaReeditHostView>> {
        let window_id = window.window_handle().window_id();
        self.hosts
            .iter()
            .filter(|(presenter, _)| presenter.window_id() == window_id)
            .map(|(_, host)| host.clone())
            .collect()
    }
}

pub enum MediaReeditLauncher {}

impl MediaReeditLauncher {
    pub(crate) fn shared(cx: &mut App) -> Entity<MediaReeditLauncherHosts> {
        rise_ui::shared::shared(cx, MediaReeditLauncherHosts::new)
    }

    pub fn present(
        original: CleverImage,
        initial_state: Option<EditorImageState>,
        asset_id: String,
        caption: String,
        theme_store: Entity<ThemeStore>,
        completion: impl FnOnce(Option<MediaReeditResult>, &mut App) + 'static,
        cx: &mut App,
    ) {
        Self::present_with_configuration(
            original,
            initial_state,
            asset_id,
            caption,
            theme_store,
            MediaPickerConfiguration::chat_attachments(),
            None,
            completion,
            cx,
        );
    }

    /// present with a configuration and an optional attached track
    #[allow(clippy::too_many_arguments)]
    pub fn present_with_configuration(
        original: CleverImage,
        initial_state: Option<EditorImageState>,
        asset_id: String,
        caption: String,
        theme_store: Entity<ThemeStore>,
        configuration: MediaPickerConfiguration,
        attached_track: Option<MediaPickerTrackSelection>,
        completion: impl FnOnce(Option<MediaReeditResult>, &mut App) + 'static,
        cx: &mut App,
    ) {
        let Some(presenter) = MediaPickerPresenterResolver::top_presenter(cx) else {
            completion(None, cx);
            return;
        };
        let host = cx.new(|cx| {
            MediaReeditHostView::new(
                original,
                initial_state,
                asset_id,
                caption,
                theme_store,
                configuration,
                attached_track,
                Box::new(completion),
                cx,
            )
        });
        Self::shared(cx).update(cx, |hosts, cx| {
            hosts.hosts.push((presenter, host));
            cx.notify();
        });
    }
}

pub struct MediaReeditHostView {
    original: CleverImage,
    initial_state: Option<EditorImageState>,
    asset_id: String,
    initial_caption: String,
    theme_store: Entity<ThemeStore>,
    configuration: MediaPickerConfiguration,
    completion: Option<MediaReeditCompletion>,
    selection: Entity<MediaSelectionState>,
    caption: Entity<String>,
    opacity: Animated<f32>,
    did_appear: bool,
    did_finish: bool,
    is_baking: bool,
    focus_before_presentation: Option<(AnyWindowHandle, FocusHandle)>,
}

impl MediaReeditHostView {
    #[allow(clippy::too_many_arguments)]
    fn new(
        original: CleverImage,
        initial_state: Option<EditorImageState>,
        asset_id: String,
        initial_caption: String,
        theme_store: Entity<ThemeStore>,
        configuration: MediaPickerConfiguration,
        attached_track: Option<MediaPickerTrackSelection>,
        completion: MediaReeditCompletion,
        cx: &mut Context<Self>,
    ) -> Self {
        let selection = cx.new(|cx| {
            let mut selection = MediaSelectionState::new(cx);
            selection.attached_track = attached_track;
            selection
        });
        let caption = {
            let initial_caption = initial_caption.clone();
            cx.new(|_| initial_caption)
        };
        Self {
            original,
            initial_state,
            asset_id,
            initial_caption,
            theme_store,
            configuration,
            completion: Some(completion),
            selection,
            caption,
            opacity: Animated::new(0.0),
            did_appear: false,
            did_finish: false,
            is_baking: false,
            focus_before_presentation: None,
        }
    }

    /// fades the layer out, then answers once
    fn finish(&mut self, result: Option<MediaReeditResult>, cx: &mut Context<Self>) {
        if self.did_finish {
            return;
        }
        self.did_finish = true;
        self.opacity.set_in(0.0, ease_in_out(CROSS_DISSOLVE_SECONDS), RiseClock::monotonic(), cx);
        cx.notify();
        let this_entity = cx.entity();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(CROSS_DISSOLVE_SECONDS)).await;
            let _ = this.update(cx, |this, cx| {
                MediaReeditLauncher::shared(cx).update(cx, |hosts, cx| {
                    hosts.hosts.retain(|(_, host)| *host != this_entity);
                    cx.notify();
                });
                if let Some((window_handle, focus)) = this.focus_before_presentation.take() {
                    let _ = window_handle.update(cx, |_, window, cx| window.focus(&focus, cx));
                }
                if let Some(completion) = this.completion.take() {
                    let app: &mut App = cx;
                    completion(result, app);
                }
            });
        })
        .detach();
    }

    fn handle_done(&mut self, outcome: MediaEditorOutcome, cx: &mut Context<Self>) {
        if self.is_baking {
            return;
        }
        let MediaSelectionEditPayload::Image(mut state) = outcome.edit.payload else {
            self.finish(None, cx);
            return;
        };
        let caption = self.caption.read(cx).clone();
        let caption_unchanged = caption == self.initial_caption;
        let nothing_changed = match &self.initial_state {
            Some(initial_state) => state == *initial_state && caption_unchanged,
            None => !state.was_edited && caption_unchanged,
        };
        if nothing_changed {
            self.finish(None, cx);
            return;
        }
        state.caption = caption.clone();

        self.is_baking = true;
        let original = self.original.clone();
        let bake = cx.background_spawn(async move { Self::bake(&original, state, caption) });
        cx.spawn(async move |this, cx| {
            let result = bake.await.and_then(Self::result);
            let _ = this.update(cx, |this, cx| {
                this.is_baking = false;
                this.finish(result, cx);
            });
        })
        .detach();
    }

    fn bake(original: &CleverImage, state: EditorImageState, caption: String) -> Option<MediaReeditBaked> {
        let rendered = EditorImageBake::render_edited_image(original, &state);
        let rgba = CleverImageBitmapEncoder::rgba(&rendered)?;
        let mut png_data = Vec::new();
        PngEncoder::new(&mut png_data)
            .write_image(rgba.as_raw(), rgba.width(), rgba.height(), image::ExtendedColorType::Rgba8)
            .ok()?;
        let pixel_size = size(rgba.width() as f32, rgba.height() as f32);
        let png_url = temporary_directory().join(format!("ro-reedit-{}.png", Uuid::new_v4().uuid_string()));
        let png_file_name = Self::readable_file_name();
        Self::write_atomically(&png_url, &png_data).ok()?;
        let original_url = Self::write_original(original);
        let editor_state_data = EditorStateCodec::encode_image(&state);
        let person_tags = state.person_tags.unwrap_or_default();
        Some(MediaReeditBaked {
            png_data,
            pixel_size,
            png_url,
            png_file_name,
            original_url,
            editor_state_data,
            person_tags,
            caption,
        })
    }

    fn result(baked: MediaReeditBaked) -> Option<MediaReeditResult> {
        let url = Url::from_file_path(&baked.png_url)?;
        let mut payload =
            MediaPickerEditedImage::new(baked.png_data, true, baked.pixel_size, baked.caption.clone());
        payload.editor_state_data = baked.editor_state_data;
        payload.original_image_url = baked.original_url.as_deref().and_then(Url::from_file_path);
        payload.person_tags = baked.person_tags;
        let item = MediaPickerItem::new(
            Uuid::new_v4().uuid_string(),
            MediaPickerItemKind::Image,
            url,
            baked.png_file_name,
            None,
            Some(payload),
            None,
            None,
            false,
        );
        Some(MediaReeditResult { item, caption: baked.caption })
    }

    fn readable_file_name() -> String {
        let calendar = Calendar::current();
        let now = RiseClock::now();
        let part = |component| calendar.component(component, &now);
        format!(
            "photo_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.png",
            part(CalendarComponent::Year),
            part(CalendarComponent::Month),
            part(CalendarComponent::Day),
            part(CalendarComponent::Hour),
            part(CalendarComponent::Minute),
            part(CalendarComponent::Second),
        )
    }

    fn write_original(image: &CleverImage) -> Option<PathBuf> {
        let rgba = CleverImageBitmapEncoder::rgba(image)?;
        let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, ORIGINAL_JPEG_QUALITY).encode_image(&rgb).ok()?;
        let url = temporary_directory().join(format!("ro-orig-{}.jpg", Uuid::new_v4().uuid_string()));
        let _ = Self::write_atomically(&url, &data);
        Some(url)
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = url.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, url).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }
}

/// what the bake hands back to the main thread
struct MediaReeditBaked {
    png_data: Vec<u8>,
    pixel_size: Size<f32>,
    png_url: PathBuf,
    png_file_name: String,
    original_url: Option<PathBuf>,
    editor_state_data: Option<Vec<u8>>,
    person_tags: Vec<EditorPersonTagEntity>,
    caption: String,
}

impl Render for MediaReeditHostView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        TopPresenter::note_presented("media-reedit-top-presenter", window, cx);
        let now = RiseClock::monotonic();
        if !self.did_appear {
            self.did_appear = true;
            self.focus_before_presentation = window.focused(cx).map(|focus| (window.window_handle(), focus));
            self.opacity.set_in(1.0, ease_in_out(CROSS_DISSOLVE_SECONDS), now, cx);
        }
        let opacity = self.opacity.frame_value(now, window);

        let original_size = self.original.size(0);
        let media_asset = MediaAsset::new(
            self.asset_id.clone(),
            MediaAssetMediaType::Image,
            i64::from(original_size.width.0),
            i64::from(original_size.height.0),
            0.0,
            None,
        );
        let initial_edit = self.initial_state.clone().map(|state| MediaSelectionEdit {
            payload: MediaSelectionEditPayload::Image(state),
        });
        let caption = Binding::entity(
            self.caption.clone(),
            |caption: &String| caption.clone(),
            |caption: &mut String, value, _| *caption = value,
        );
        let cancel = cx.weak_entity();
        let done = cx.weak_entity();
        let screen = media_editor_view_hooks().map(|hooks| {
            (hooks.reedit_screen)(
                ElementId::Name(SharedString::from(format!("media-reedit-screen-{}", self.asset_id))),
                self.selection.clone(),
                caption,
                media_asset,
                self.theme_store.clone(),
                self.configuration.clone(),
                initial_edit,
                self.original.clone(),
                Box::new(move |_: &mut Window, cx: &mut App| {
                    let _ = cancel.update(cx, |this, cx| this.finish(None, cx));
                }),
                Box::new(move |outcome: MediaEditorOutcome, _: &mut Window, cx: &mut App| {
                    let _ = done.update(cx, |this, cx| this.handle_done(outcome, cx));
                }),
            )
        });

        div().id("media-reedit-host").absolute().inset_0().occlude().opacity(opacity).children(screen)
    }
}
