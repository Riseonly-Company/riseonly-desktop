use std::cell::Cell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, AsyncApp, ElementId, Entity, FontWeight, Image, ImageFormat, ImageSource, MouseButton,
    ObjectFit, Pixels, RenderImage, SharedString, Size, Window, black, canvas, img, relative, size, white,
};
use image::{Delay, Frame, RgbaImage};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_media::video_export::poster as video_poster;
use rise_platform::paths::temporary_directory;
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::uniform_type::UTType;
use rise_theme::theme::theme_environment::theme_store;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::keyboard_dismiss::KeyboardDismissModifiers;
use rise_ui::media_spoiler_overlay_view::MediaSpoilerOverlay;
use rise_ui::bottom_sheet_ui::modal_sheet_surface::{
    ModalSheetFrame, ModalSheetMetrics, modal_sheet_centering, modal_sheet_chrome, modal_sheet_outline,
};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::scroll_edge_blur_ui::ScrollEdgeBlurUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::window_cover::WindowCoverFrame;
use rise_ui::{IconUi, MainText, TextAlignT};
use smallvec::SmallVec;

use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::modules::chat::components::chat::composer::chat_composer_paste::ChatComposerPaste;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatDraftCustomEmoji;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;
use crate::modules::media_picker::ui::media_picker_selection_bar::MediaPickerCaptionBar;
use crate::modules::media_picker::editor::media_editor_presentation_host::{
    MEDIA_SEND_BACKDROP_BLUR, media_edge_color, media_send_backdrop,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_backdrop::backdrop_blur_layer;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// one pasted image, video or file
/// already copied out of the pasteboard into a temp file
#[derive(Clone)]
pub struct ChatPasteMediaEntry {
    pub id: String,
    pub kind: MediaPickerItemKind,
    pub url: Url,
    pub file_name: String,
    pub preview: Option<CleverImage>,
}

impl ChatPasteMediaEntry {
    /// builds the picker item, spoiler defaults to false
    pub fn item(&self, as_file: bool, spoiler: bool) -> MediaPickerItem {
        let mut item = MediaPickerItem::new(
            self.id.clone(),
            if as_file { MediaPickerItemKind::File } else { self.kind },
            self.url.clone(),
            self.file_name.clone(),
            None,
            None,
            None,
            None,
            as_file || self.kind == MediaPickerItemKind::File,
        );
        item.has_spoiler = spoiler && !as_file && self.kind != MediaPickerItemKind::File;
        item
    }
}

#[derive(Clone)]
pub struct ChatPasteMediaProvider {
    pub registered_types: Vec<UTType>,
    pub suggested_name: Option<String>,
    source: ChatPasteMediaProviderSource,
}

#[derive(Clone)]
enum ChatPasteMediaProviderSource {
    File(PathBuf),
    Image(Image),
}

impl ChatPasteMediaProvider {
    pub fn file(path: PathBuf) -> Self {
        let registered = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(UTType::from_filename_extension)
            .unwrap_or_else(UTType::data);
        let suggested_name = path.file_name().and_then(|name| name.to_str()).map(str::to_owned);
        Self {
            registered_types: vec![registered],
            suggested_name,
            source: ChatPasteMediaProviderSource::File(path),
        }
    }

    pub fn image(image: Image) -> Self {
        Self {
            registered_types: vec![Self::image_type(image.format)],
            suggested_name: None,
            source: ChatPasteMediaProviderSource::Image(image),
        }
    }

    pub fn from_paste(paste: ChatComposerPaste) -> Vec<Self> {
        match paste {
            ChatComposerPaste::Files(paths) => paths.into_iter().map(Self::file).collect(),
            ChatComposerPaste::Image(image) => vec![Self::image(image)],
            ChatComposerPaste::Text => Vec::new(),
        }
    }

    fn image_type(format: ImageFormat) -> UTType {
        match format {
            ImageFormat::Png => UTType::png(),
            ImageFormat::Jpeg => UTType::jpeg(),
            ImageFormat::Gif => UTType::gif(),
            ImageFormat::Tiff => UTType::tiff(),
            _ => UTType::image(),
        }
    }

    fn load_file_representation(&self, target: &Path) -> io::Result<()> {
        match &self.source {
            ChatPasteMediaProviderSource::File(source) => fs::copy(source, target).map(|_| ()),
            ChatPasteMediaProviderSource::Image(image) => fs::write(target, &image.bytes),
        }
    }
}

pub enum ChatPasteMediaLoader {}

impl ChatPasteMediaLoader {
    pub const MAX_ITEMS: usize = 10;
    const PREVIEW_SIDE: u32 = 1400;

    pub fn pasteboard_holds_media(pasteboard: UIPasteboard, cx: &App) -> bool {
        if pasteboard.has_images(cx) {
            return true;
        }
        let paths = pasteboard.file_paths(cx);
        if !paths.is_empty() {
            return paths.iter().all(|path| path.is_file());
        }
        false
    }

    pub async fn load(providers: Vec<ChatPasteMediaProvider>, cx: &AsyncApp) -> Vec<ChatPasteMediaEntry> {
        let mut entries = Vec::new();
        for provider in providers.into_iter().take(Self::MAX_ITEMS) {
            if let Some(entry) = Self::entry(provider, cx).await {
                entries.push(entry);
            }
        }
        entries
    }

    pub fn discard(entries: &[ChatPasteMediaEntry]) {
        let paths: Vec<PathBuf> = entries.iter().filter_map(|entry| entry.url.to_file_path()).collect();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            for path in paths {
                let _ = fs::remove_file(path);
            }
        }));
    }

    async fn entry(provider: ChatPasteMediaProvider, cx: &AsyncApp) -> Option<ChatPasteMediaEntry> {
        cx.background_executor().spawn(async move { Self::entry_from(&provider) }).await
    }

    fn entry_from(provider: &ChatPasteMediaProvider) -> Option<ChatPasteMediaEntry> {
        let registered = &provider.registered_types;
        let (kind, ty) = if let Some(image) = registered.iter().find(|ty| ty.conforms_to(&UTType::image())) {
            (MediaPickerItemKind::Image, *image)
        } else if let Some(movie) = registered.iter().find(|ty| ty.conforms_to(&UTType::movie())) {
            (MediaPickerItemKind::Video, *movie)
        } else if let Some(file) = registered.iter().find(|ty| {
            !ty.conforms_to(&UTType::text()) && !ty.conforms_to(&UTType::url()) && ty.conforms_to(&UTType::data())
        }) {
            (MediaPickerItemKind::File, *file)
        } else {
            return None;
        };
        let path = Self::copy_file(provider, ty)?;
        let preview = match kind {
            MediaPickerItemKind::Image => Self::downsampled_image(&path),
            MediaPickerItemKind::Video => Self::video_poster(&path),
            MediaPickerItemKind::File => None,
        };
        let file_name = path.file_name().and_then(|name| name.to_str()).map(str::to_owned).unwrap_or_default();
        let url = Url::from_file_path(&path)?;
        Some(ChatPasteMediaEntry {
            id: format!("paste-{}", Uuid::new_v4().uuid_string()),
            kind,
            url,
            file_name,
            preview,
        })
    }

    fn copy_file(provider: &ChatPasteMediaProvider, ty: UTType) -> Option<PathBuf> {
        let folder = temporary_directory().join("chat-paste");
        let _ = fs::create_dir_all(&folder);
        let ext = ty.preferred_filename_extension().unwrap_or_else(|| "bin".to_owned());
        let base = provider.suggested_name.as_deref().map(trimming_whitespaces_and_newlines);
        let stem = match base {
            Some(base) if !base.is_empty() => base.to_owned(),
            _ => Uuid::new_v4().uuid_string().to_lowercase(),
        };
        let name = if Path::new(&stem).extension().is_none() { format!("{stem}.{ext}") } else { stem };
        let target = folder.join(Uuid::new_v4().uuid_string().to_lowercase()).join(name);
        fs::create_dir_all(target.parent()?).ok()?;
        provider.load_file_representation(&target).ok()?;
        Some(target)
    }

    fn downsampled_image(path: &Path) -> Option<CleverImage> {
        let data = fs::read(path).ok()?;
        CleverImageBitmapDecoder::decode(&data, Some(i64::from(Self::PREVIEW_SIDE)))
    }

    fn video_poster(path: &Path) -> Option<CleverImage> {
        let frame = video_poster(path, 0.0, Self::PREVIEW_SIDE).ok()?;
        let bitmap = RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
        let frames: SmallVec<[Frame; 1]> =
            SmallVec::from_buf([Frame::from_parts(bitmap, 0, 0, Delay::from_numer_denom_ms(0, 1))]);
        Some(Arc::new(RenderImage::new(frames)))
    }
}

struct ChatPasteMediaSheetState {
    is_more_menu_presented: bool,
    spoiler_opacity: Animated<f32>,
    preview_box: Rc<Cell<Size<Pixels>>>,
}

/// paste of media into the composer
/// a sheet with no surface of its own
/// chat wallpaper behind the preview, caption under it and one send
#[derive(IntoElement)]
pub struct ChatPasteMediaSheet {
    id: ElementId,
    entries: Vec<ChatPasteMediaEntry>,
    backdrop: AnyElement,
    caption: Binding<String>,
    caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
    on_send: Rc<dyn Fn(bool, bool, &mut Window, &mut App)>,
    appearance: f32,
    presentable: bool,
}

impl ChatPasteMediaSheet {
    const SEND_UNCOMPRESSED_ACTION_ID: &'static str = "chat_paste_send_uncompressed";
    const SPOILER_ACTION_ID: &'static str = "chat_paste_spoiler";
    const HEIGHT_FRACTION: f32 = 0.74;
    const MIN_HEIGHT: f32 = 320.0;
    const TOP_BLUR_HEIGHT: f32 = 96.0;

    /// builds the sheet, caption custom emojis default to none
    pub fn new(
        id: impl Into<ElementId>,
        entries: Vec<ChatPasteMediaEntry>,
        backdrop: impl IntoElement,
        caption: Binding<String>,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        on_send: impl Fn(bool, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            entries,
            backdrop: backdrop.into_any_element(),
            caption,
            caption_custom_emojis: None,
            on_close: Rc::new(on_close),
            on_send: Rc::new(on_send),
            appearance: 1.0,
            presentable: true,
        }
    }

    /// modal appearance fraction (scale and opacity) and whether it takes input
    /// a closing modal lets clicks through to the chat
    pub fn appearance(mut self, appearance: f32, presentable: bool) -> Self {
        self.appearance = appearance;
        self.presentable = presentable;
        self
    }

    pub fn caption_custom_emojis(mut self, caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>) -> Self {
        self.caption_custom_emojis = caption_custom_emojis;
        self
    }

    fn more_menu_actions(has_media: bool, selection: &Entity<MediaSelectionState>) -> HoldContextMenuPresentationActions {
        let selection = selection.clone();
        Rc::new(move |cx: &App| {
            let mut actions = vec![
                HoldContextMenuAction::new(Self::SEND_UNCOMPRESSED_ACTION_ID, tr("media_picker_send_uncompressed"))
                    .system_image("doc"),
            ];
            if has_media {
                let hidden = selection.read(cx).hides_under_spoiler;
                actions.push(
                    HoldContextMenuAction::new(
                        Self::SPOILER_ACTION_ID,
                        tr(if hidden { "media_picker_show_without_spoiler" } else { "chat_paste_spoiler" }),
                    )
                    .system_image(if hidden { "eye" } else { "eye.slash" }),
                );
            }
            actions
        })
    }

    fn handle_menu(
        &self,
        state: &Entity<ChatPasteMediaSheetState>,
        selection: &Entity<MediaSelectionState>,
    ) -> HoldContextMenuPresentationOnAction {
        let on_send = self.on_send.clone();
        let state = state.clone();
        let selection = selection.clone();
        Rc::new(move |id: String, window: &mut Window, cx: &mut App| match id.as_str() {
            Self::SEND_UNCOMPRESSED_ACTION_ID => on_send(true, false, window, cx),
            Self::SPOILER_ACTION_ID => {
                let hidden = selection.update(cx, |selection, cx| {
                    selection.hides_under_spoiler = !selection.hides_under_spoiler;
                    cx.notify();
                    selection.hides_under_spoiler
                });
                state.update(cx, |state, cx| {
                    let target = if hidden { 1.0 } else { 0.0 };
                    state.spoiler_opacity.set_in(target, ease_in_out(0.25), RiseClock::monotonic(), cx);
                    cx.notify();
                });
            }
            _ => {}
        })
    }

    fn header(
        &self,
        theme: &AppTheme,
        state: &Entity<ChatPasteMediaSheetState>,
        selection: &Entity<MediaSelectionState>,
    ) -> impl IntoElement + use<> {
        let on_close = self.on_close.clone();
        let opener = state.clone();
        let is_more_menu_presented = Binding::entity(
            state.clone(),
            |state: &ChatPasteMediaSheetState| state.is_more_menu_presented,
            |state: &mut ChatPasteMediaSheetState, value: bool, cx| {
                state.is_more_menu_presented = value;
                cx.notify();
            },
        );
        let has_media = self.entries.iter().any(|entry| entry.kind != MediaPickerItemKind::File);
        let source_id = child_id(&self.id, "more-source");
        let source: HoldContextMenuTapSource = Rc::new(move |_window: &mut Window, _cx: &mut App| {
            PageHeaderIconButton::new(source_id.clone(), "ellipsis", |_, _| {}).font_size(17.0).into_any_element()
        });
        div()
            .relative()
            .w_full()
            .px(px(12.0))
            .flex()
            .items_center()
            .child(
                div().absolute().inset_0().flex().items_center().justify_center().child(
                    MainText::new(tr("chat_paste_sheet_title"))
                        .px(17.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary)
                        .number_of_lines(1),
                ),
            )
            .child(
                hstack(None)
                    .w_full()
                    .child(
                        PageHeaderIconButton::new(child_id(&self.id, "close"), "xmark", move |window, cx| {
                            on_close(window, cx)
                        })
                        .font_size(17.0),
                    )
                    .child(spacer())
                    .child(
                        div().id(child_id(&self.id, "more-hint")).aria_label(tr("share_action_more")).tooltip_label(tr("share_action_more")).child(
                        PageHeaderIconButton::new(child_id(&self.id, "more"), "ellipsis", move |_, cx| {
                            opener.update(cx, |state, cx| {
                                state.is_more_menu_presented = true;
                                cx.notify();
                            });
                        })
                        .font_size(17.0)
                        .tap_context_menu(
                            child_id(&self.id, "more-menu"),
                            is_more_menu_presented,
                            HoldContextMenuAlignment::Trailing,
                            false,
                            false,
                            false,
                            None,
                            Self::more_menu_actions(has_media, selection),
                            self.handle_menu(state, selection),
                            source,
                        ),
                        ),
                    ),
            )
    }

    fn preview(
        &self,
        entry: &ChatPasteMediaEntry,
        theme: &AppTheme,
        spoiler_opacity: f32,
        preview_box: &Rc<Cell<Size<Pixels>>>,
    ) -> AnyElement {
        let radius = px(16.0);
        if let Some(image) = entry.preview.clone() {
            let pixels = image.size(0);
            let fitted = Self::aspect_fit(pixels.width.0 as f32, pixels.height.0 as f32, preview_box.get());
            let probe_box = preview_box.clone();
            let probe = canvas(
                move |bounds, window, _| {
                    if probe_box.get() != bounds.size {
                        probe_box.set(bounds.size);
                        let host = window.current_view();
                        window.on_next_frame(move |_, cx| cx.notify(host));
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0();
            let mut frame = div()
                .relative()
                .flex_none()
                .w(fitted.width)
                .h(fitted.height)
                .rounded(radius)
                .overflow_hidden()
                .child(
                    img(ImageSource::Render(image.clone()))
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .rounded(radius),
                )
                .child(div().absolute().inset_0().rounded(radius).border_1().border_color(media_edge_color(theme)));
            if spoiler_opacity > 0.0 && entry.kind != MediaPickerItemKind::File {
                frame = frame.child(
                    pass_through()
                        .absolute()
                        .inset_0()
                        .rounded(radius)
                        .overflow_hidden()
                        .opacity(spoiler_opacity)
                        .child(MediaSpoilerOverlay::new(Some(image), format!("paste:{}", entry.id))),
                );
            }
            if entry.kind == MediaPickerItemKind::Video {
                frame = frame.child(
                    div().absolute().inset_0().flex().items_center().justify_center().child(
                        div()
                            .size(px(58.0))
                            .rounded_full()
                            .bg(black().opacity(0.42))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("play.fill", px(26.0), white(), true)),
                    ),
                );
            }
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(probe)
                .child(frame)
                .into_any_element()
        } else {
            let fill = theme.primary._100;
            div()
                .size_full()
                .flex()
                .items_center()
                .child(
                    hstack(12.0)
                        .w_full()
                        .items_center()
                        .p(px(14.0))
                        .rounded(radius)
                        .bg(black().opacity(0.36))
                        .child(
                            div()
                                .size(px(46.0))
                                .flex_none()
                                .rounded_full()
                                .bg(fill)
                                .flex()
                                .items_center()
                                .justify_center()
                                .children(IconUi::sized("doc.fill", px(22.0), theme.content_on(fill), true)),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                MainText::new(entry.file_name.clone())
                                    .px(15.0)
                                    .font_weight(FontWeight::MEDIUM)
                                    .color(white())
                                    .number_of_lines(2),
                            ),
                        ),
                )
                .into_any_element()
        }
    }

    fn aspect_fit(width: f32, height: f32, fitting: Size<Pixels>) -> Size<Pixels> {
        let (box_w, box_h) = (f32::from(fitting.width), f32::from(fitting.height));
        if !(width > 0.0 && height > 0.0 && box_w > 0.0 && box_h > 0.0) {
            return Size::default();
        }
        let scale = (box_w / width).min(box_h / height);
        size(px(width * scale), px(height * scale))
    }

    fn previews(&self, theme: &AppTheme, spoiler_opacity: f32, preview_box: &Rc<Cell<Size<Pixels>>>) -> AnyElement {
        if self.entries.len() == 1
            && let Some(entry) = self.entries.first()
        {
            return div()
                .size_full()
                .px(px(24.0))
                .child(self.preview(entry, theme, spoiler_opacity, preview_box))
                .into_any_element();
        }
        div()
            .id(child_id(&self.id, "previews"))
            .size_full()
            .overflow_x_scroll()
            .px(px(24.0))
            .flex()
            .gap(px(12.0))
            .children(self.entries.iter().map(|entry| {
                div().h_full().w(relative(1.0)).flex_none().child(self.preview(entry, theme, spoiler_opacity, preview_box))
            }))
            .into_any_element()
    }

    fn panel(
        &self,
        theme: &AppTheme,
        theme_store: Entity<ThemeStore>,
        state: &Entity<ChatPasteMediaSheetState>,
        selection: &Entity<MediaSelectionState>,
        spoiler_opacity: f32,
        preview_box: &Rc<Cell<Size<Pixels>>>,
    ) -> impl IntoElement + use<> {
        let on_send = self.on_send.clone();
        let sent_selection = selection.clone();
        vstack(0.0)
            .size_full()
            .child(
                div().w_full().pt(px(8.0)).child(self.header(theme, state, selection)),
            )
            .child(
                div().w_full().flex().justify_center().pt(px(10.0)).pb(px(10.0)).child(
                    div().px(px(12.0)).py(px(5.0)).rounded_full().bg(black().opacity(0.36)).child(
                        MainText::new(tr("chat_paste_preview_badge"))
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::MEDIUM)
                            .color(white())
                            .number_of_lines(1),
                    ),
                ),
            )
            .child(div().w_full().flex_1().min_h_0().child(self.previews(theme, spoiler_opacity, preview_box)))
            .child(
                MediaPickerCaptionBar::new(
                    child_id(&self.id, "caption-bar"),
                    selection.clone(),
                    theme_store,
                    self.caption.clone(),
                    move |window, cx| {
                        let hidden = sent_selection.read(cx).hides_under_spoiler;
                        on_send(false, hidden, window, cx);
                    },
                )
                .caption_custom_emojis(self.caption_custom_emojis.clone())
                .bottom_safe_inset(0.0)
                .send_enabled(!self.entries.is_empty()),
            )
    }
}

impl RenderOnce for ChatPasteMediaSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = theme_store(cx);
        let theme = theme_store.read(cx).current_theme().clone();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| ChatPasteMediaSheetState {
            is_more_menu_presented: false,
            spoiler_opacity: Animated::new(0.0),
            preview_box: Rc::new(Cell::new(Size::default())),
        });
        let selection = window.use_keyed_state(child_id(&self.id, "selection"), cx, |_, cx| MediaSelectionState::new(cx));
        let now = RiseClock::monotonic();
        let (spoiler_opacity, preview_box) = {
            let state = state.read(cx);
            (
                state.spoiler_opacity.frame_value(now, window),
                state.preview_box.clone(),
            )
        };
        let panel = self.panel(&theme, theme_store, &state, &selection, spoiler_opacity, &preview_box);
        let on_close = self.on_close.clone();
        let ChatPasteMediaSheet { backdrop, appearance, presentable, .. } = self;
        let frame = ModalSheetFrame::for_window(window, cx);
        let container_height = f32::from(WindowCoverFrame::resolve(window, cx).size.height);
        let height = frame.filled_height((container_height * Self::HEIGHT_FRACTION).max(Self::MIN_HEIGHT));
        let surface = div()
            .relative()
            .w(px(frame.width))
            .max_w_full()
            .h(px(height))
            .max_h_full()
            .when(presentable, |surface| surface.occlude())
            .child(
                pass_through()
                    .absolute()
                    .inset_0()
                    .child(div().absolute().inset_0().child(backdrop))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .w_full()
                            .h(px(Self::TOP_BLUR_HEIGHT))
                            .child(ScrollEdgeBlurUi::new()),
                    ),
            )
            .child(div().relative().size_full().child(panel));
        let surface = modal_sheet_chrome(surface, theme.bg._100, modal_sheet_outline(&theme), appearance);
        let appearance_share = appearance.clamp(0.0, 1.0);
        div()
            .relative()
            .size_full()
            .children(backdrop_blur_layer(MEDIA_SEND_BACKDROP_BLUR * appearance_share, cx))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(media_send_backdrop(cx).opacity(appearance_share))
                    .when(presentable, |scrim| {
                        scrim.occlude().on_mouse_down(MouseButton::Left, move |_, window, cx| on_close(window, cx))
                    }),
            )
            .child(modal_sheet_centering().child(surface))
            .keyboard_dismiss_excluded()
    }
}
