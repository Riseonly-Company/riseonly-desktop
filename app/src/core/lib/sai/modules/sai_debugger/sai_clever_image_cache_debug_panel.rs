#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext, Context, ElementId, Entity, FontWeight, Hsla, ImageSource, ListAlignment, ObjectFit, SharedString,
    Task, Window, img, rgb, white,
};
use rise_core::foundation::foundation_date::Date;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text_selection::selectable_text;

use super::sai_debug_storage_panel_views::SaiDebugBodyFormat;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageDiskCache, CleverImageDiskCacheListedEntry};

const DESTRUCTIVE_RED: u32 = 0xe11d48;
const LINK_BLUE: u32 = 0x29b6f6;
const IMAGE_BADGE: u32 = 0x00897b;
const SIZE_BADGE: u32 = 0x6a1b9a;

const LIST_MAX_HEIGHT: f32 = 500.;

pub type SaiCleverImageCacheRequestDelete = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub type SaiCleverImageCacheRequestClearAll = Rc<dyn Fn(&mut Window, &mut App)>;

/// disk read runs on a worker once per snapshot tick, dropping the task cancels a stale read
/// frames only commit what it returned
struct SaiCleverImageCacheDebugPanelState {
    entries: Rc<Vec<CleverImageDiskCacheListedEntry>>,
    folder_path: String,
    loaded_tick: Option<i64>,
    list: ListUi,
    preview_entry: Option<CleverImageDiskCacheListedEntry>,
    presented_preview: Option<CleverImageDiskCacheListedEntry>,
    _load_task: Option<Task<()>>,
}

impl SaiCleverImageCacheDebugPanelState {
    fn load_entries(&mut self, tick: i64, cx: &mut Context<Self>) {
        self.loaded_tick = Some(tick);
        let user_id = SaiAccountCacheScope::active_user_id();
        self._load_task = Some(cx.spawn(async move |this, cx| {
            let (entries, folder_path) = cx
                .background_spawn(async move {
                    (
                        CleverImageDiskCache::list_entries(user_id.as_deref()),
                        CleverImageDiskCache::folder_path_for_inspector(user_id.as_deref()),
                    )
                })
                .await;
            let _ = this.update(cx, |state, cx| {
                let old_count = state.list.item_count();
                if entries.len() > old_count {
                    state.list.appended(entries.len() - old_count);
                } else {
                    state.list.removed(entries.len()..old_count);
                }
                state.entries = Rc::new(entries);
                state.folder_path = folder_path;
                cx.notify();
            });
        }));
    }
}

#[derive(IntoElement)]
pub struct SaiCleverImageCacheDebugPanel {
    theme: AppTheme,
    snapshot_tick: Binding<i64>,
    on_request_delete: SaiCleverImageCacheRequestDelete,
    on_request_clear_all: SaiCleverImageCacheRequestClearAll,
}

impl SaiCleverImageCacheDebugPanel {
    pub fn new(
        theme: AppTheme,
        snapshot_tick: Binding<i64>,
        on_request_delete: impl Fn(String, &mut Window, &mut App) + 'static,
        on_request_clear_all: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            theme,
            snapshot_tick,
            on_request_delete: Rc::new(on_request_delete),
            on_request_clear_all: Rc::new(on_request_clear_all),
        }
    }

    fn header(&self, count: usize) -> Div {
        let theme = &self.theme;
        let on_request_clear_all = self.on_request_clear_all.clone();
        let snapshot_tick = self.snapshot_tick.clone();
        hstack(8.)
            .w_full()
            .child(
                div()
                    .child("CleverImage")
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.secondary),
            )
            .child(
                div()
                    .child(format!("({count})"))
                    .swiftui_font(Font::caption2().monospaced_digit())
                    .text_color(theme.text.secondary.opacity(0.75)),
            )
            .child(spacer_min_w(0.))
            .children((count > 0).then(|| {
                plain("sai-clever-image-clear-all")
                    .on_press(move |window, cx| on_request_clear_all(window, cx))
                    .child(
                        div()
                            .child("Clear all")
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(rgb(DESTRUCTIVE_RED)),
                    )
            }))
            .child(
                plain("sai-clever-image-refresh")
                    .on_press(move |window, cx| {
                        let tick = snapshot_tick.get(cx);
                        snapshot_tick.set(tick + 1, window, cx);
                    })
                    .child(
                        div()
                            .child("Refresh")
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(rgb(LINK_BLUE)),
                    ),
            )
            .px(px(14.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.35))
    }

    fn path_line(&self, folder_path: String) -> Div {
        div()
            .w_full()
            .px(px(12.))
            .pb(px(6.))
            .text_center()
            .child(folder_path)
            .swiftui_font(Font::caption2())
            .text_color(self.theme.text.secondary.opacity(0.78))
            .line_clamp(3)
    }
}

impl RenderOnce for SaiCleverImageCacheDebugPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let list_theme = theme.clone();
        let state = window.use_keyed_state("sai-clever-image-cache-panel", cx, move |_, _| {
            SaiCleverImageCacheDebugPanelState {
                entries: Rc::new(Vec::new()),
                folder_path: String::new(),
                loaded_tick: None,
                list: ListUi::new(&list_theme, 0, ListAlignment::Top),
                preview_entry: None,
                presented_preview: None,
                _load_task: None,
            }
        });
        let tick = self.snapshot_tick.get(cx);
        state.update(cx, |state, cx| {
            if state.loaded_tick != Some(tick) {
                state.load_entries(tick, cx);
            }
        });
        let entries = state.read(cx).entries.clone();
        let folder_path = state.read(cx).folder_path.clone();

        let content: AnyElement = if entries.is_empty() {
            div()
                .w_full()
                .flex()
                .justify_center()
                .px(px(14.))
                .pt(px(14. + 28.))
                .child(
                    div()
                        .child("No CleverImage disk entries yet. Enable `persistToDisk` on chat message images.")
                        .swiftui_font(Font::caption())
                        .text_color(theme.text.secondary),
                )
                .into_any_element()
        } else {
            let row_entries = entries.clone();
            let row_theme = theme.clone();
            let row_state = state.clone();
            let on_request_delete = self.on_request_delete.clone();
            state
                .read(cx)
                .list
                .element(move |index, _window, _cx| {
                    let Some(entry) = row_entries.get(index) else {
                        return div().into_any_element();
                    };
                    let show_state = row_state.clone();
                    let show_entry = entry.clone();
                    let copy_key = entry.cache_key.clone();
                    let delete_digest = entry.digest.clone();
                    let on_request_delete = on_request_delete.clone();
                    div()
                        .w_full()
                        .px(px(14.))
                        .pt(px(14.))
                        .pb(px(if index + 1 == row_entries.len() { 14. } else { 0. }))
                        .child(SaiCleverImageCacheDebugCardView {
                            entry: entry.clone(),
                            theme: row_theme.clone(),
                            on_show: Rc::new(move |cx: &mut App| {
                                let entry = show_entry.clone();
                                show_state.update(cx, |state, cx| {
                                    state.presented_preview = Some(entry.clone());
                                    state.preview_entry = Some(entry);
                                    cx.notify();
                                });
                            }),
                            on_copy: Rc::new(move |cx: &mut App| UIPasteboard::general().set_string(&copy_key, cx)),
                            on_delete: Rc::new(move |window: &mut Window, cx: &mut App| {
                                on_request_delete(delete_digest.clone(), window, cx);
                            }),
                        })
                        .into_any_element()
                })
                .into_any_element()
        };

        let is_presented = Binding::entity(
            state.clone(),
            |state: &SaiCleverImageCacheDebugPanelState| state.preview_entry.is_some(),
            |state: &mut SaiCleverImageCacheDebugPanelState, value, _| {
                if !value {
                    state.preview_entry = None;
                }
            },
        );
        let sheet_state = state.clone();
        let sheet_theme = theme.clone();
        let sheet_presented = is_presented.clone();
        let preview_sheet = NativeBottomSheetPresentationHost::new(
            "sai-clever-image-preview-sheet",
            is_presented,
            move |_window, cx| match sheet_state.read(cx).presented_preview.clone() {
                Some(entry) => SaiCleverImageCachePreviewSheet {
                    entry,
                    theme: sheet_theme.clone(),
                    is_presented: sheet_presented.clone(),
                }
                .into_any_element(),
                None => div().into_any_element(),
            },
        )
        .presentation_detents([PresentationDetent::Large]);

        vstack(0.)
            .w_full()
            .child(self.header(entries.len()))
            .child(self.path_line(folder_path))
            .child(div().w_full().h(px(LIST_MAX_HEIGHT)).bg(theme.bg._100).child(content))
            .child(preview_sheet)
    }
}

/// image disk cache read for a card or the sheet, done on a worker
/// decoding in render would stall every frame
struct SaiCleverImageLoadState {
    loaded_at: Option<Date>,
    image: Option<CleverImage>,
    finished: bool,
    _task: Option<Task<()>>,
}

fn use_cached_image(
    id: ElementId,
    entry: &CleverImageDiskCacheListedEntry,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SaiCleverImageLoadState> {
    let state = window.use_keyed_state(id, cx, |_, _| SaiCleverImageLoadState {
        loaded_at: None,
        image: None,
        finished: false,
        _task: None,
    });
    let modified_at = entry.modified_at;
    let cache_key = entry.cache_key.clone();
    state.update(cx, |state, cx| {
        if state.loaded_at == Some(modified_at) {
            return;
        }
        state.loaded_at = Some(modified_at);
        state.finished = false;
        let user_id = SaiAccountCacheScope::active_user_id();
        state._task = Some(cx.spawn(async move |this, cx| {
            let image = cx
                .background_spawn(async move { CleverImageDiskCache::ui_image(&cache_key, user_id.as_deref()) })
                .await;
            let _ = this.update(cx, |state, cx| {
                state.image = image;
                state.finished = true;
                cx.notify();
            });
        }));
    });
    state
}

fn sub_id(id: &ElementId, part: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), part.into())
}

fn capsule_badge(text: impl Into<SharedString>, fill: u32) -> Div {
    div()
        .child(text.into())
        .swiftui_font(Font::caption2().weight(FontWeight::EXTRA_BOLD))
        .text_color(white())
        .px(px(7.))
        .py(px(3.))
        .bg(rgb(fill))
        .capsule()
}

fn card_icon_button(
    id: ElementId,
    symbol: &str,
    color: Hsla,
    theme: &AppTheme,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> PlainButton {
    plain(id).on_press(action).child(
        div()
            .flex()
            .items_center()
            .justify_center()
            .frame_size(36., 32.)
            .bg(theme.bg._300.opacity(0.9))
            .clip_shape(Shape::continuous(8.))
            .children(IconUi::sized(symbol, px(15.), color, false)),
    )
}

#[derive(IntoElement)]
struct SaiCleverImageCacheDebugCardView {
    entry: CleverImageDiskCacheListedEntry,
    theme: AppTheme,
    on_show: Rc<dyn Fn(&mut App)>,
    on_copy: Rc<dyn Fn(&mut App)>,
    on_delete: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl RenderOnce for SaiCleverImageCacheDebugCardView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let id = ElementId::Name(SharedString::from(format!("sai-clever-image-{}", self.entry.digest)));
        let thumb = use_cached_image(sub_id(&id, "thumb"), &self.entry, window, cx).read(cx).image.clone();
        let on_delete = self.on_delete.clone();
        let on_copy = self.on_copy.clone();
        let on_show = self.on_show.clone();

        let thumb_view: AnyElement = match thumb {
            Some(image) => img(ImageSource::Render(image))
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(px(8.))
                .into_any_element(),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .bg(theme.bg._300)
                .children(IconUi::sized("photo", px(17.), theme.text.secondary.opacity(0.5), false))
                .into_any_element(),
        };

        let body = vstack(0.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .bg(theme.bg._200.opacity(0.92))
            .clip_shape(Shape::continuous(12.))
            .child(
                hstack(6.)
                    .w_full()
                    .px(px(10.))
                    .py(px(8.))
                    .bg(theme.bg._300.opacity(0.45))
                    .child(capsule_badge("IMAGE", IMAGE_BADGE))
                    .child(capsule_badge(SaiDebugBodyFormat::size_label_byte_count(self.entry.file_bytes), SIZE_BADGE))
                    .child(spacer_min_w(0.))
                    .child(card_icon_button(
                        sub_id(&id, "delete"),
                        "trash",
                        rgb(DESTRUCTIVE_RED).into(),
                        &theme,
                        move |window, cx| on_delete(window, cx),
                    ))
                    .child(card_icon_button(
                        sub_id(&id, "copy"),
                        "doc.on.doc",
                        theme.text.primary,
                        &theme,
                        move |_window, cx| on_copy(cx),
                    )),
            )
            .child(div().w_full().h(px(1.)).bg(theme.border._200))
            .child(
                hstack(12.)
                    .align(VerticalAlignment::Top)
                    .w_full()
                    .p(px(10.))
                    .bg(theme.bg._100)
                    .child(
                        div().flex_none().frame_size(52., 52.).clip_shape(Shape::continuous(8.)).child(thumb_view),
                    )
                    .child(
                        vstack(6.)
                            .align(HorizontalAlignment::Leading)
                            .min_w(px(0.))
                            .child(
                                div()
                                    .child("CACHE KEY")
                                    .swiftui_font(Font::system(10., FontWeight::EXTRA_BOLD, FontDesign::Default))
                                    .text_color(theme.text.secondary.opacity(0.85)),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                                    .text_color(theme.text.primary)
                                    .line_clamp(4)
                                    .child(selectable_text(sub_id(&id, "key"), self.entry.cache_key.clone())),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .child(format!("digest: {}", self.entry.digest))
                                    .swiftui_font(Font::caption2())
                                    .text_color(theme.text.secondary.opacity(0.7))
                                    .line_clamp(1)
                                    .text_ellipsis_middle(),
                            ),
                    )
                    .child(spacer_min_w(0.))
                    .child(
                        plain(sub_id(&id, "show"))
                            .on_press(move |_window, cx| on_show(cx))
                            .child(
                                div()
                                    .child("Show")
                                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                                    .text_color(rgb(LINK_BLUE))
                                    .px(px(12.))
                                    .py(px(8.))
                                    .bg(theme.bg._300.opacity(0.9))
                                    .clip_shape(Shape::continuous(8.)),
                            ),
                    ),
            );

        div()
            .relative()
            .w_full()
            .child(body)
            .child(centred_stroke(theme.border._200.opacity(0.65), 1., Shape::continuous(12.)))
    }
}

#[derive(IntoElement)]
struct SaiCleverImageCachePreviewSheet {
    entry: CleverImageDiskCacheListedEntry,
    theme: AppTheme,
    is_presented: Binding<bool>,
}

impl RenderOnce for SaiCleverImageCachePreviewSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = self.theme.clone();
        let id = ElementId::Name(SharedString::from(format!("sai-clever-image-preview-{}", self.entry.digest)));
        let loaded = use_cached_image(sub_id(&id, "image"), &self.entry, window, cx);
        let (image, finished) = {
            let loaded = loaded.read(cx);
            (loaded.image.clone(), loaded.finished)
        };
        let is_presented = self.is_presented.clone();
        let copy_key = self.entry.cache_key.clone();

        let image_view: Option<AnyElement> = match image {
            Some(image) => Some(
                img(ImageSource::Render(image))
                    .w_full()
                    .object_fit(ObjectFit::Contain)
                    .rounded(px(12.))
                    .into_any_element(),
            ),
            None if finished => Some(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w_full()
                    .min_h(px(120.))
                    .child("File missing on disk")
                    .swiftui_font(Font::caption())
                    .text_color(theme.text.secondary)
                    .into_any_element(),
            ),
            None => None,
        };

        vstack(0.)
            .size_full()
            .bg(theme.bg._100)
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w_full()
                    .h(px(44.))
                    .child(
                        div()
                            .child("CleverImage preview")
                            .swiftui_font(Font::headline())
                            .text_color(theme.text.primary),
                    )
                    .child(
                        div().absolute().left(px(16.)).child(
                            plain(sub_id(&id, "close"))
                                .on_press(move |window, cx| is_presented.set(false, window, cx))
                                .child(div().child("Close").text_color(theme.primary._100)),
                        ),
                    )
                    .child(
                        div().absolute().right(px(16.)).child(
                            plain(sub_id(&id, "copy"))
                                .on_press(move |_window, cx| UIPasteboard::general().set_string(&copy_key, cx))
                                .children(IconUi::sized("doc.on.doc", px(17.), theme.primary._100, false)),
                        ),
                    ),
            )
            .child(
                div().id(sub_id(&id, "scroll")).flex_1().min_h(px(0.)).w_full().overflow_y_scroll().child(
                    vstack(16.)
                        .align(HorizontalAlignment::Leading)
                        .w_full()
                        .p(px(16.))
                        .children(image_view)
                        .child(
                            div()
                                .w_full()
                                .swiftui_font(Font::system(12., FontWeight::NORMAL, FontDesign::Monospaced))
                                .text_color(theme.text.primary)
                                .child(selectable_text(sub_id(&id, "key"), self.entry.cache_key.clone())),
                        ),
                ),
            )
    }
}
