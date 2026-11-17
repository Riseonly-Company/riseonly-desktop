use std::cell::Cell;
use std::f32::consts::{FRAC_PI_2, PI};
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Context, ElementId, Entity, Global, Hsla, ImageSource, IntoElement, Path, PathBuilder,
    Pixels, Point, Render, RenderOnce, SharedString, Size, Subscription, TransformationMatrix, Window, black, canvas,
    img, point, white,
};
use rise_canon::RiseUploadFormat;
use rise_theme::AppTheme;
use rise_theme::theme::theme_environment::theme_store;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationContentInteraction, PresentationDetent,
};
use rise_ui::swiftui::prelude::*;
use rise_theme::ui_color_system::UIColor;
use rise_ui::swiftui::progress_view::{LINEAR_HEIGHT, linear_fraction, progress_view_value};

use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::media_picker::api::media_picker_configuration::{MediaPickerConfiguration, MediaPickerSourceTab};
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerResult};
use crate::modules::media_picker::editor::media_editor_screen::MediaEditorOutcome;
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorWindowPresenter;
use crate::modules::media_picker::library::media_asset::MediaAsset;
use crate::modules::media_picker::ui::media_picker_screen::{MediaPickerLayout, MediaPickerScreen};
use crate::modules::media_picker::ui::media_picker_session::MediaPickerSession;
use crate::modules::media_picker::ui::media_picker_sheet::{MediaPickerFloatingDoneButton, MediaPickerSheet};

const PNG_SIGNATURE: [u8; 4] = [0x89, 0x50, 0x4E, 0x47];
const JPEG_SIGNATURE: [u8; 3] = [0xFF, 0xD8, 0xFF];

type SelectMediaCustomLabel = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

const UPLOAD_FILL_CAP_SEGMENTS: usize = 8;

/// upload bar fill drawn as a swappable path
/// the progress owner swaps each new value into the frame, no page rerender
#[derive(Clone)]
pub struct SelectMediaUploadSwap {
    key: u64,
    painted: Rc<Cell<Option<Size<Pixels>>>>,
}

impl SelectMediaUploadSwap {
    pub fn new(key: u64) -> Self {
        Self { key, painted: Rc::new(Cell::new(None)) }
    }

    /// swaps the fill to cover_upload_progress, 0 to 100
    /// false if the frame holds no bar with this key, then render instead
    pub fn swap(&self, cover_upload_progress: f64, window: &mut Window) -> bool {
        let Some(size) = self.painted.get() else {
            return false;
        };
        let (path, color) =
            upload_fill_path(Bounds::new(Point::default(), size), linear_fraction(cover_upload_progress / 100.0));
        let swapped = path.is_some_and(|path| window.swap_paths(vec![(self.key, path, color.into())]));
        if !swapped {
            self.painted.set(None);
        }
        swapped
    }
}

/// white capsule of the linear bar up to fraction
/// empty bar keeps a clear sliver so the frame still has a path to swap
fn upload_fill_path(bounds: Bounds<Pixels>, fraction: f32) -> (Option<Path<Pixels>>, Hsla) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let radius = height / 2.0;
    let end = width * fraction;
    let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
    let mut builder = PathBuilder::fill();
    if end < 0.5 || radius <= 0.0 {
        builder.add_polygon(&[at(0.0, 0.0), at(0.5, 0.0), at(0.0, height.max(0.5))], true);
        return (builder.build().ok(), white().opacity(0.0));
    }
    let length = end.max(height);
    let mut points = Vec::with_capacity(2 * (UPLOAD_FILL_CAP_SEGMENTS + 1));
    for step in 0..=UPLOAD_FILL_CAP_SEGMENTS {
        let angle = -FRAC_PI_2 + PI * step as f32 / UPLOAD_FILL_CAP_SEGMENTS as f32;
        points.push(at(length - radius + radius * angle.cos(), radius + radius * angle.sin()));
    }
    for step in 0..=UPLOAD_FILL_CAP_SEGMENTS {
        let angle = FRAC_PI_2 + PI * step as f32 / UPLOAD_FILL_CAP_SEGMENTS as f32;
        points.push(at(radius + radius * angle.cos(), radius + radius * angle.sin()));
    }
    builder.add_polygon(&points, true);
    (builder.build().ok(), white())
}

fn swappable_upload_bar(fraction: f32, swap: SelectMediaUploadSwap, theme: &AppTheme) -> AnyElement {
    let track = UIColor::system_fill().resolved_color(theme.appearance);
    let key = swap.key;
    let painted = swap.painted;
    div()
        .relative()
        .w_full()
        .h(px(LINEAR_HEIGHT))
        .flex_shrink_0()
        .rounded_full()
        .overflow_hidden()
        .bg(track)
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let (path, color) = upload_fill_path(bounds, fraction);
                    let Some(path) = path else {
                        painted.set(None);
                        return;
                    };
                    let swappable = window.transformation() == TransformationMatrix::unit()
                        && !path.bounds.intersect(&window.content_mask().bounds).is_empty();
                    window.paint_swappable_path(path, color, key, bounds.origin);
                    painted.set(swappable.then_some(bounds.size));
                },
            )
            .absolute()
            .inset_0(),
        )
        .into_any_element()
}

#[derive(IntoElement)]
pub struct SelectMediaUi {
    pub id: ElementId,
    pub local_file: Binding<Option<SaiWsOptimisticLocalFile>>,
    pub preview: Binding<Option<CleverImage>>,
    pub size: f32,
    pub placeholder_system_image: SharedString,
    pub cover_upload_progress: f64,
    pub show_upload_overlay: bool,
    pub picker_configuration: MediaPickerConfiguration,
    pub placeholder_background: Option<Hsla>,
    pub placeholder_foreground: Option<Hsla>,
    pub remote_source: Option<String>,
    pub remote_name: String,
    pub remote_user_id: Option<String>,
    pub remote_cache_policy: CleverImageCachePolicy,
    pub custom_label: Option<SelectMediaCustomLabel>,
    pub uses_app_sheet_presentation: bool,
    pub caption: Option<String>,
    pub disabled: bool,
    pub upload_swap: Option<SelectMediaUploadSwap>,
}

impl SelectMediaUi {
    /// defaults: size 60, camera placeholder no upload overlay
    /// chat avatar picker and standard cache policy, no remote source
    pub fn new(
        id: impl Into<ElementId>,
        local_file: Binding<Option<SaiWsOptimisticLocalFile>>,
        preview: Binding<Option<CleverImage>>,
    ) -> Self {
        Self {
            id: id.into(),
            local_file,
            preview,
            size: 60.0,
            placeholder_system_image: "camera.fill".into(),
            cover_upload_progress: 0.0,
            show_upload_overlay: false,
            picker_configuration: MediaPickerConfiguration::chat_avatar().offering_file_chooser(),
            placeholder_background: None,
            placeholder_foreground: None,
            remote_source: None,
            remote_name: String::new(),
            remote_user_id: None,
            remote_cache_policy: CleverImageCachePolicy::Standard,
            custom_label: None,
            uses_app_sheet_presentation: false,
            caption: None,
            disabled: false,
            upload_swap: None,
        }
    }

    /// paints the upload bar fill through the given swap
    pub fn upload_swap(mut self, swap: SelectMediaUploadSwap) -> Self {
        self.upload_swap = Some(swap);
        self
    }

    /// disables the button for pointer and keyboard both
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn placeholder_system_image(mut self, placeholder_system_image: impl Into<SharedString>) -> Self {
        self.placeholder_system_image = placeholder_system_image.into();
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

    pub fn picker_configuration(mut self, picker_configuration: MediaPickerConfiguration) -> Self {
        self.picker_configuration = picker_configuration.offering_file_chooser();
        self
    }

    pub fn placeholder_background(mut self, placeholder_background: impl Into<Option<Hsla>>) -> Self {
        self.placeholder_background = placeholder_background.into();
        self
    }

    pub fn placeholder_foreground(mut self, placeholder_foreground: impl Into<Option<Hsla>>) -> Self {
        self.placeholder_foreground = placeholder_foreground.into();
        self
    }

    pub fn remote_source(mut self, remote_source: impl Into<Option<String>>) -> Self {
        self.remote_source = remote_source.into();
        self
    }

    pub fn remote_name(mut self, remote_name: impl Into<String>) -> Self {
        self.remote_name = remote_name.into();
        self
    }

    pub fn remote_user_id(mut self, remote_user_id: impl Into<Option<String>>) -> Self {
        self.remote_user_id = remote_user_id.into();
        self
    }

    pub fn remote_cache_policy(mut self, remote_cache_policy: CleverImageCachePolicy) -> Self {
        self.remote_cache_policy = remote_cache_policy;
        self
    }

    pub fn custom_label<E: IntoElement>(mut self, custom_label: impl Fn(&mut Window, &mut App) -> E + 'static) -> Self {
        self.custom_label = Some(Rc::new(move |window, cx| custom_label(window, cx).into_any_element()));
        self
    }

    pub fn uses_app_sheet_presentation(mut self, uses_app_sheet_presentation: bool) -> Self {
        self.uses_app_sheet_presentation = uses_app_sheet_presentation;
        self
    }

    pub fn caption(mut self, caption: impl Into<Option<String>>) -> Self {
        self.caption = caption.into();
        self
    }
}

/// state pair of the view
struct SelectMediaUiState {
    show_picker: bool,
    session: Option<Entity<MediaPickerSession>>,
}

/// what the picker closures need from the view, cloned out of it
#[derive(Clone)]
struct SelectMediaWiring {
    state: Entity<SelectMediaUiState>,
    show_picker: Binding<bool>,
    local_file: Binding<Option<SaiWsOptimisticLocalFile>>,
    preview: Binding<Option<CleverImage>>,
    picker_configuration: MediaPickerConfiguration,
    theme_store: Entity<ThemeStore>,
}

impl Drop for SelectMediaUiState {
    fn drop(&mut self) {
        tracing::warn!(target: "riseonly::X844", show_picker = self.show_picker, has_session = self.session.is_some(), "select media state dropped");
    }
}

impl SelectMediaWiring {
    fn session(&self, cx: &App) -> Option<Entity<MediaPickerSession>> {
        self.state.read(cx).session.clone()
    }

    fn hide_picker(&self, window: &mut Window, cx: &mut App) {
        tracing::warn!(target: "riseonly::X844", "select media hide picker");
        self.show_picker.set(false, window, cx);
    }

    fn hide_picker_if_finished(&self, session: &Entity<MediaPickerSession>, window: &mut Window, cx: &mut App) {
        if session.read(cx).did_finish() {
            self.hide_picker(window, cx);
        }
    }

    fn handle_sheet_dismiss(&self, cx: &mut App) {
        if let Some(session) = self.session(cx) {
            tracing::warn!(target: "riseonly::X844", finished = session.read(cx).did_finish(), "select media sheet dismissed");
            MediaEditorWindowPresenter::shared(cx).update(cx, |presenter, cx| presenter.teardown(&session, cx));
            if !session.read(cx).did_finish() {
                let editor_presentation = session.read(cx).editor_presentation.clone();
                if editor_presentation.read(cx).is_presented {
                    editor_presentation.update(cx, |presentation, cx| presentation.complete_flyback_handoff(cx));
                }
                session.update(cx, |session, cx| session.cancel(cx));
            }
        }
        self.state.update(cx, |state, cx| {
            state.session = None;
            cx.notify();
        });
    }

    fn boot_session(&self, cx: &mut App) {
        let wiring = self.clone();
        let configuration = self.picker_configuration.clone();
        let theme_store = self.theme_store.clone();
        let session = cx.new(|cx| {
            MediaPickerSession::new(
                configuration,
                theme_store,
                None,
                None,
                None,
                Rc::new(move |result: MediaPickerResult, window: &mut Window, cx: &mut App| {
                    tracing::warn!(target: "riseonly::X844", cancelled = result.was_cancelled, items = result.items.len(), "select media session completed");
                    wiring.handle_result(result, window, cx);
                    wiring.hide_picker(window, cx);
                }),
                cx,
            )
        });
        self.state.update(cx, |state, cx| {
            state.session = Some(session);
            cx.notify();
        });
    }

    fn sync_editor_window(&self, session: &Entity<MediaPickerSession>, cx: &mut App) {
        let wiring = self.clone();
        let outcome_session = session.clone();
        let configuration = self.picker_configuration.clone();
        let theme_store = self.theme_store.clone();
        let session = session.clone();
        MediaEditorWindowPresenter::shared(cx).update(cx, |presenter, cx| {
            presenter.sync_presentation(
                session,
                configuration,
                theme_store,
                Rc::new(move |outcome: MediaEditorOutcome, asset: MediaAsset, window: &mut Window, cx: &mut App| {
                    outcome_session.update(cx, |session, cx| session.handle_editor_outcome(outcome, asset, cx));
                    wiring.hide_picker_if_finished(&outcome_session, window, cx);
                }),
                cx,
            );
        });
    }

    fn handle_result(&self, result: MediaPickerResult, window: &mut Window, cx: &mut App) {
        tracing::warn!(target: "riseonly::X844", cancelled = result.was_cancelled, items = result.items.len(), "select media handle result");
        if result.was_cancelled {
            return;
        }
        let Some(item) = result.items.into_iter().next() else {
            return;
        };
        let payload = item.image_payload.as_ref().map(|payload| payload.data.clone());
        let path = item.url.to_file_path();
        let preview = self.preview.clone();
        let executor = cx.background_executor().clone();
        window
            .spawn(cx, async move |cx| {
                let decoded = executor
                    .spawn(async move {
                        payload
                            .and_then(|data| CleverImageBitmapDecoder::decode(&data, None))
                            .or_else(|| {
                                path.and_then(|path| std::fs::read(path).ok())
                                    .and_then(|data| CleverImageBitmapDecoder::decode(&data, None))
                            })
                    })
                    .await;
                if let Some(image) = decoded {
                    let _ = cx.update(|window, cx| preview.set(Some(image), window, cx));
                }
            })
            .detach();
        let content_type = image_content_type(&item);
        tracing::warn!(target: "riseonly::X844", url = %item.url.path(), has_payload = item.image_payload.is_some(), "select media local file set");
        self.local_file.set(
            Some(SaiWsOptimisticLocalFile::new(item.url.clone(), item.suggested_file_name.clone(), content_type)),
            window,
            cx,
        );
    }
}

fn image_content_type(item: &MediaPickerItem) -> String {
    if let Some(payload) = &item.image_payload {
        if payload.data.starts_with(&PNG_SIGNATURE) {
            return RiseUploadFormat::IMAGE_PNG.mime.to_owned();
        }
        if payload.data.starts_with(&JPEG_SIGNATURE) {
            return RiseUploadFormat::IMAGE_JPEG.mime.to_owned();
        }
    }
    let extension = item.url.path_extension().to_lowercase();
    let format = [RiseUploadFormat::IMAGE_PNG, RiseUploadFormat::IMAGE_HEIC, RiseUploadFormat::IMAGE_WEBP]
        .into_iter()
        .find(|format| format.disk_extensions.iter().any(|disk_extension| *disk_extension == extension))
        .unwrap_or(RiseUploadFormat::IMAGE_JPEG);
    format.mime.to_owned()
}

impl SelectMediaUi {
    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), suffix.into())
    }

    fn avatar_label(&self, theme: &AppTheme, cx: &App) -> AnyElement {
        let size = self.size;
        let mut label = zstack(Alignment::Center);
        if let Some(preview) = self.preview.get(cx) {
            label = label.child(
                img(ImageSource::Render(preview))
                    .size(px(size))
                    .object_fit(gpui::ObjectFit::Cover)
                    .rounded_full(),
            );
        } else if let Some(remote_source) = self.remote_source.clone().filter(|source| !source.is_empty()) {
            label = label.child(
                UserLogoUi::new(self.child_id("remote-logo"))
                    .source(SharedString::from(remote_source))
                    .name(SharedString::from(self.remote_name.clone()))
                    .user_id(self.remote_user_id.clone().map(SharedString::from))
                    .size(size)
                    .image_cache_policy(self.remote_cache_policy),
            );
        } else {
            let foreground = self.placeholder_foreground.unwrap_or(theme.text.secondary);
            let glyph = IconUi::sized(
                &self.placeholder_system_image,
                px(size * 0.34),
                foreground,
                self.placeholder_system_image.ends_with(".fill"),
            )
            .map(IntoElement::into_any_element);
            label = label.child(
                zstack(Alignment::Center)
                    .size(px(size))
                    .rounded_full()
                    .bg(self.placeholder_background.unwrap_or(theme.bg._300))
                    .children(glyph),
            );
        }

        if self.show_upload_overlay {
            label = label
                .child(div().size(px(size)).rounded_full().bg(black().opacity(0.45)))
                .child(div().w(px(size * 0.55)).child(match self.upload_swap.clone() {
                    Some(swap) => swappable_upload_bar(linear_fraction(self.cover_upload_progress / 100.0), swap, theme),
                    None => progress_view_value((self.cover_upload_progress / 100.0).clamp(0.0, 1.0))
                        .tint(white())
                        .into_any_element(),
                }));
        }
        label.into_any_element()
    }

    fn app_sheet_picker(&self, wiring: &SelectMediaWiring) -> AnyElement {
        Self::app_sheet_picker_element(self.child_id("app-sheet"), wiring)
    }

    fn app_sheet_picker_element(id: ElementId, wiring: &SelectMediaWiring) -> AnyElement {
        let on_complete = wiring.clone();
        MediaPickerSheet::new(
            id,
            wiring.show_picker.clone(),
            wiring.picker_configuration.clone(),
            wiring.theme_store.clone(),
            move |result, window, cx| {
                on_complete.hide_picker(window, cx);
                on_complete.handle_result(result, window, cx);
            },
        )
        .experimental_bottom_sheet(true)
        .into_any_element()
    }

    fn media_picker_content(
        picker_configuration: &MediaPickerConfiguration,
        wiring: &SelectMediaWiring,
        session: &Entity<MediaPickerSession>,
        cx: &App,
    ) -> AnyElement {
        let theme_store = wiring.theme_store.clone();
        let read = session.read(cx);
        let (on_cancel, on_cancel_session) = (wiring.clone(), session.clone());
        let (on_outcome, on_outcome_session) = (wiring.clone(), session.clone());
        let (on_send, on_send_session) = (wiring.clone(), session.clone());
        let (on_document, on_document_session) = (wiring.clone(), session.clone());
        let picker = MediaPickerScreen::new(
            picker_configuration.clone(),
            theme_store.clone(),
            read.selection.clone(),
            read.caption.clone(),
            read.fetcher.clone(),
            read.editor_presentation.clone(),
            move |window, cx| {
                on_cancel_session.update(cx, |session, cx| session.cancel(cx));
                on_cancel.hide_picker(window, cx);
            },
            move |outcome, asset, window, cx| {
                on_outcome_session.update(cx, |session, cx| session.handle_editor_outcome(outcome, asset, cx));
                on_outcome.hide_picker_if_finished(&on_outcome_session, window, cx);
            },
            move |send_as_file, window, cx| {
                on_send_session.update(cx, |session, cx| session.export_selection(send_as_file, cx));
                on_send.hide_picker_if_finished(&on_send_session, window, cx);
            },
            move |window, cx| {
                on_document_session.update(cx, |session, cx| {
                    session.present_document_picker(MediaPickerSourceTab::File, cx);
                });
                on_document.hide_picker_if_finished(&on_document_session, window, cx);
            },
        )
        .layout(MediaPickerLayout::BottomSheet)
        .import_session(session.downgrade());

        let mut content = div().relative().size_full().child(picker);
        if picker_configuration.shows_bottom_bar {
            let (on_done, on_done_session) = (wiring.clone(), session.clone());
            content = content.child(
                div().absolute().left_0().right_0().bottom(px(5.0)).px(px(16.0)).child(
                    MediaPickerFloatingDoneButton::new(
                        read.selection.clone(),
                        theme_store,
                        picker_configuration.confirm_button_title.clone(),
                        move |window, cx| {
                            on_done_session.update(cx, |session, cx| session.export_selection(false, cx));
                            on_done.hide_picker_if_finished(&on_done_session, window, cx);
                        },
                    ),
                ),
            );
        }
        content.into_any_element()
    }
}

impl RenderOnce for SelectMediaUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = theme_store(cx);
        let theme = theme_store.read(cx).current_theme().clone();
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _: &mut Context<SelectMediaUiState>| {
            tracing::warn!(target: "riseonly::X844", "select media state created");
            SelectMediaUiState { show_picker: false, session: None }
        });
        let show_picker = Binding::entity(
            state.clone(),
            |state: &SelectMediaUiState| state.show_picker,
            |state: &mut SelectMediaUiState, value, _| state.show_picker = value,
        );
        let wiring = SelectMediaWiring {
            state: state.clone(),
            show_picker: show_picker.clone(),
            local_file: self.local_file.clone(),
            preview: self.preview.clone(),
            picker_configuration: self.picker_configuration.clone(),
            theme_store: theme_store.clone(),
        };

        let label = if let Some(custom_label) = &self.custom_label {
            custom_label(window, cx)
        } else if let Some(caption) = &self.caption {
            vstack(10.0)
                .child(self.avatar_label(&theme, cx))
                .child(
                    MainText::new(caption.clone())
                        .px(14.0)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .color(theme.primary._100),
                )
                .into_any_element()
        } else {
            self.avatar_label(&theme, cx)
        };

        let open = show_picker.clone();
        let in_root_layer = self.uses_app_sheet_presentation && SelectMediaSheetLayer::shared(cx).is_some();
        let layer_sheet_id = self.child_id("app-sheet");
        if in_root_layer {
            SelectMediaSheetLayer::refresh(&layer_sheet_id, &wiring, cx);
        }
        let press_wiring = wiring.clone();
        let button = plain(self.child_id("button"))
            .disabled(self.disabled)
            .on_press(move |window, cx| {
                if in_root_layer {
                    SelectMediaSheetLayer::present(&layer_sheet_id, &press_wiring, cx);
                }
                open.set(true, window, cx);
            })
            .child(label);

        let overlay = if in_root_layer {
            div().into_any_element()
        } else if self.uses_app_sheet_presentation {
            self.app_sheet_picker(&wiring)
        } else {
            let session = state.read(cx).session.clone();
            let editor_is_presented = session
                .as_ref()
                .is_some_and(|session| session.read(cx).editor_presentation.read(cx).is_presented);
            if let Some(session) = &session {
                MediaPickerSession::error_alert(self.child_id("error-alert"), session, window, cx);
                let sync = wiring.clone();
                let sync_session = session.clone();
                on_change(
                    ElementId::NamedChild(
                        std::sync::Arc::new(self.child_id("editor-presented")),
                        SharedString::from(session.entity_id().as_u64().to_string()),
                    ),
                    editor_is_presented,
                    true,
                    window,
                    cx,
                    move |_, _, _, cx| sync.sync_editor_window(&sync_session, cx),
                );
            }
            let (on_prepare, on_dismiss, content_wiring) = (wiring.clone(), wiring.clone(), wiring.clone());
            let picker_configuration = self.picker_configuration.clone();
            NativeBottomSheetPresentationHost::new(self.child_id("native-sheet"), show_picker, move |_, cx| {
                match content_wiring.session(cx) {
                    Some(session) => {
                        Self::media_picker_content(&picker_configuration, &content_wiring, &session, cx)
                    }
                    None => div().into_any_element(),
                }
            })
            .is_content_ready(session.is_some())
            .on_prepare(move |_, cx| on_prepare.boot_session(cx))
            .on_dismiss(move |_, cx| on_dismiss.handle_sheet_dismiss(cx))
            .presentation_detents([PresentationDetent::Fraction(0.56), PresentationDetent::Fraction(0.92)])
            .presentation_drag_indicator(true)
            .presentation_background(theme.bg._100)
            .presentation_corner_radius(16.0)
            .presentation_content_interaction(PresentationContentInteraction::Scrolls)
            .interactive_dismiss_disabled(editor_is_presented)
            .into_any_element()
        };

        div().relative().child(button).child(div().absolute().inset_0().child(overlay))
    }
}

struct SelectMediaSheetLayerGlobal(Entity<SelectMediaSheetLayer>);

impl Global for SelectMediaSheetLayerGlobal {}

struct SelectMediaSheetEntry {
    id: ElementId,
    wiring: SelectMediaWiring,
    _show_picker: Subscription,
}

/// the sheet picker of a select media button as a root layer
/// drawn in the page it rerendered the page and side panels every frame, so its state lives up here
pub struct SelectMediaSheetLayer {
    entry: Option<SelectMediaSheetEntry>,
}

impl SelectMediaSheetLayer {
    pub fn install(cx: &mut App) -> Entity<Self> {
        if let Some(layer) = cx.try_global::<SelectMediaSheetLayerGlobal>() {
            return layer.0.clone();
        }
        let layer = cx.new(|_| Self { entry: None });
        cx.set_global(SelectMediaSheetLayerGlobal(layer.clone()));
        layer
    }

    fn shared(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<SelectMediaSheetLayerGlobal>().map(|layer| layer.0.clone())
    }

    fn holds(&self, id: &ElementId, wiring: &SelectMediaWiring) -> bool {
        self.entry.as_ref().is_some_and(|entry| entry.id == *id && entry.wiring.state == wiring.state)
    }

    /// keeps the closures of the sheet current from the button render
    /// a notify from render is dropped and the sheet doesnt need one till the button is pressed
    fn refresh(id: &ElementId, wiring: &SelectMediaWiring, cx: &mut App) {
        let Some(layer) = Self::shared(cx) else { return };
        layer.update(cx, |layer, _| {
            if layer.holds(id, wiring)
                && let Some(entry) = layer.entry.as_mut()
            {
                entry.wiring = wiring.clone();
            }
        });
    }

    /// pressed button hands over to the layer, layer follows its state to draw open and dismiss
    fn present(id: &ElementId, wiring: &SelectMediaWiring, cx: &mut App) {
        let Some(layer) = Self::shared(cx) else { return };
        layer.update(cx, |layer, cx| {
            if layer.holds(id, wiring)
                && let Some(entry) = layer.entry.as_mut()
            {
                entry.wiring = wiring.clone();
            } else {
                let show_picker = cx.observe(&wiring.state, |_, _, cx| cx.notify());
                layer.entry =
                    Some(SelectMediaSheetEntry { id: id.clone(), wiring: wiring.clone(), _show_picker: show_picker });
            }
            cx.notify();
        });
    }
}

impl Render for SelectMediaSheetLayer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let sheet = self
            .entry
            .as_ref()
            .map(|entry| SelectMediaUi::app_sheet_picker_element(entry.id.clone(), &entry.wiring));
        div().size_full().child(div().absolute().inset_0().children(sheet))
    }
}
