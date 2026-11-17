use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{App, ElementId, Empty, Entity, FontWeight, ScrollHandle, SharedString, Window, white};
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFraction;
use rise_ui::button_ui::ButtonUi;
use rise_ui::download_selection_badge::DownloadSelectionBadge;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::MainText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::modules::post::components::download::post_download_store::PostDownloadSheetStore;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;

#[derive(IntoElement)]
pub struct PostDownloadSheetHost {
    id: ElementId,
}

impl PostDownloadSheetHost {
    const MINIMUM_FRACTION: f32 = 0.3;

    pub fn new() -> Self {
        Self { id: ElementId::Name(SharedString::new_static("post-download-sheet")) }
    }

    fn child_id(id: &ElementId, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
    }
}

impl Default for PostDownloadSheetHost {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for PostDownloadSheetHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = PostDownloadSheetStore::shared(cx);
        let sheet_fraction =
            window.use_keyed_state(Self::child_id(&self.id, "fraction"), cx, |_, _| Self::MINIMUM_FRACTION);
        let fraction = *sheet_fraction.read(cx);
        let visible = Binding::entity(
            store.clone(),
            |store: &PostDownloadSheetStore| store.is_presented(),
            |store, value, cx| {
                if !value {
                    store.dismiss(cx);
                }
            },
        );
        let content = PostDownloadSheetContent {
            id: Self::child_id(&self.id, "content"),
            store: store.clone(),
            sheet_fraction,
        };
        let dismissed = store;
        BottomSheetUi::new(
            self.id.clone(),
            visible,
            move |window, cx| content.render_content(window, cx),
            |_, _| Empty,
        )
        .snap_fractions(vec![fraction])
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .on_did_dismiss(move |_, cx| dismissed.update(cx, |store, cx| store.sheet_did_dismiss(cx)))
    }
}

struct PostDownloadSheetContent {
    id: ElementId,
    store: Entity<PostDownloadSheetStore>,
    sheet_fraction: Entity<f32>,
}

impl PostDownloadSheetContent {
    const THUMB_HEIGHT: f32 = 168.0;
    const BOTTOM_ALLOWANCE: f32 = 28.0;

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn render_content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if self.store.read(cx).post().is_none() {
            return Empty.into_any_element();
        }
        let chooses_watermark = self.store.read(cx).policy().is_some_and(|policy| policy.chooses_watermark);
        let measuring = self.sheet_fraction.clone();
        let body = vstack(14.)
            .w_full()
            .pt(px(8.))
            .child(self.header(cx))
            .child(self.strip(window, cx))
            .when(chooses_watermark, |body| body.child(self.watermark_row(cx)))
            .child(div().w_full().px(px(16.)).child(self.action(cx)))
            .child(div().h(px(Self::BOTTOM_ALLOWANCE)))
            .child(measure(self.child_id("height"), move |bounds, window, cx| {
                let target = BottomSheetMeasuredFraction::value(
                    f32::from(bounds.size.height),
                    PostDownloadSheetHost::MINIMUM_FRACTION,
                    window,
                );
                measuring.update(cx, |fraction, cx| {
                    if (target - *fraction).abs() > 0.005 {
                        *fraction = target;
                        cx.notify();
                    }
                });
            }));
        scroll_view(self.child_id("scroll")).shows_indicators(false).size_full().child(body).into_any_element()
    }

    fn header(&self, cx: &App) -> AnyElement {
        let store = self.store.read(cx);
        let subtitle = tr("post_download_selected")
            .replace("{count}", &store.selected().len().to_string())
            .replace("{total}", &store.slides().len().to_string());
        vstack(4.)
            .w_full()
            .px(px(16.))
            .items_center()
            .child(MainText::new(tr("post_download_sheet_title")).px(17.).font_weight(FontWeight::SEMIBOLD))
            .child(MainText::new(subtitle).px(13.).secondary(true))
            .into_any_element()
    }

    fn strip(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let handle = window.use_keyed_state(self.child_id("strip-handle"), cx, |_, _| ScrollHandle::new()).read(cx).clone();
        let is_presented = self.store.read(cx).is_presented();
        let focus = self.store.read(cx).focus_index();
        let scrolling = handle.clone();
        on_change(self.child_id("strip-appear"), is_presented, true, window, cx, move |_, presented, _, _| {
            if *presented {
                scrolling.scroll_to_item(focus);
            }
        });
        let slides: Vec<PostMediaSlide> = self.store.read(cx).slides().to_vec();
        let thumbnails: Vec<AnyElement> =
            slides.iter().enumerate().map(|(index, slide)| self.thumbnail(index, slide, cx)).collect();
        div()
            .id(self.child_id("strip"))
            .w_full()
            .h(px(Self::THUMB_HEIGHT))
            .flex()
            .flex_row()
            .gap(px(8.))
            .px(px(16.))
            .overflow_x_scroll()
            .track_scroll(&handle)
            .children(thumbnails)
            .into_any_element()
    }

    fn thumbnail_width(slide: &PostMediaSlide) -> f32 {
        let aspect = PostMediaBox::aspect(slide.size()).unwrap_or(0.75);
        Self::THUMB_HEIGHT * aspect.clamp(0.5, 1.6)
    }

    fn thumbnail(&self, index: usize, slide: &PostMediaSlide, cx: &App) -> AnyElement {
        let store = self.store.read(cx);
        let position = store.position(index);
        let dimmed = store.is_running() && position.is_none();
        let width = Self::thumbnail_width(slide);
        let toggling = self.store.clone();
        let item_id = ElementId::NamedChild(Arc::new(self.child_id("thumb")), SharedString::from(slide.id()));
        let video_marks = slide.is_video().then(|| {
            let play = div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("play.fill", px(22.), white(), true));
            let duration = Self::duration_label(slide).map(|label| {
                div()
                    .absolute()
                    .left(px(8.))
                    .bottom(px(8.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(white())
                    .child(label)
            });
            div().absolute().inset_0().child(play).children(duration)
        });
        div()
            .id(item_id.clone())
            .relative()
            .flex_none()
            .w(px(width))
            .h(px(Self::THUMB_HEIGHT))
            .rounded(px(12.))
            .overflow_hidden()
            .opacity(if dimmed { 0.5 } else { 1.0 })
            .child(
                div().absolute().inset_0().child(
                    CleverImageUi::new(slide.poster_url().unwrap_or_default().to_owned())
                        .id(ElementId::NamedChild(Arc::new(item_id.clone()), SharedString::new_static("image")))
                        .content_mode(ContentMode::Fill)
                        .maximum_decode_pixel_dimension(512),
                ),
            )
            .children(video_marks)
            .child(
                div().absolute().top(px(8.)).right(px(8.)).child(
                    DownloadSelectionBadge::new(
                        ElementId::NamedChild(Arc::new(item_id), SharedString::new_static("badge")),
                        position.is_some(),
                    )
                    .number(position),
                ),
            )
            .on_click(move |_, _, cx| toggling.update(cx, |store, cx| store.toggle(index, cx)))
            .into_any_element()
    }

    fn duration_label(slide: &PostMediaSlide) -> Option<String> {
        let ms = slide.size().map(|size| size.duration_ms).filter(|ms| *ms > 0)?;
        let seconds = (ms as f64 / 1000.0).round() as i64;
        Some(format!("{}:{:02}", seconds / 60, seconds % 60))
    }

    fn watermark_row(&self, cx: &App) -> AnyElement {
        let store = self.store.read(cx);
        let running = store.is_running();
        let with_watermark = store.with_watermark;
        let toggling = self.store.clone();
        GroupedBtns::new(
            self.child_id("watermark"),
            vec![
                GroupedBtn::new("postDownloadWithWatermark", tr("post_download_with_watermark"))
                    .icon("drop.fill")
                    .icon_background(SettingsIconPalette::memory_images())
                    .disabled(running)
                    .show_chevron(false)
                    .trailing_slot(DownloadSelectionBadge::new(self.child_id("watermark-badge"), with_watermark))
                    .callback(move |_, cx| toggling.update(cx, |store, cx| store.toggle_watermark(cx))),
            ],
        )
        .into_any_element()
    }

    /// the run works under a bottom banner, the sheet closes on this tap
    fn action(&self, cx: &App) -> AnyElement {
        let count = self.store.read(cx).selected().len();
        let theme = rise_ui::theme(cx);
        let on_primary = theme.content_on(theme.primary._100);
        let confirming = self.store.clone();
        ButtonUi::new(self.child_id("action"))
            .disabled(count == 0)
            .on_press(move |_, cx| confirming.update(cx, |store, cx| store.confirm(cx)))
            .child(
                MainText::new(format!("{} ({count})", tr("post_download_action")))
                    .px(15.)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(on_primary),
            )
            .into_any_element()
    }
}
