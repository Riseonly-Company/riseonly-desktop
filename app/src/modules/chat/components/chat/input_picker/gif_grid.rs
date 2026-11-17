use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, ImageSource, IntoElement, ObjectFit, Pixels, RenderImage, RenderOnce, SharedString, Size,
    Window, img, px, uniform_list,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::animated_gif_view::{AnimatedGifContentMode, AnimatedGifView};
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::ChatInputPickerMetrics;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::{
    StickerHoldEntry, StickerHoldGridModifier, StickerHoldPreviewModifiers, StickerPreviewAction, StickerPreviewAsset,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::GifItem;

type GifTap = Rc<dyn Fn(&GifItem, &mut Window, &mut App)>;
type GifReachEnd = Rc<dyn Fn(&mut Window, &mut App)>;
type GifMenu = Rc<dyn Fn(&GifItem, &App) -> Vec<StickerPreviewAction>>;

/// square tiles, two to a row on a narrow panel and more on wider ones
/// gifs are fitted not cropped since most are landscape
#[derive(IntoElement)]
pub struct GifGrid {
    id: ElementId,
    items: Vec<GifItem>,
    columns: usize,
    spacing: f32,
    is_playing: bool,
    prefetch_threshold: usize,
    uses_flat_chrome: bool,
    on_tap: GifTap,
    on_reach_end: Option<GifReachEnd>,
    menu_for: Option<GifMenu>,
}

impl GifGrid {
    pub fn new(
        id: impl Into<ElementId>,
        items: Vec<GifItem>,
        on_tap: impl Fn(&GifItem, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            items,
            columns: ChatInputPickerMetrics::GIF_COLUMNS,
            spacing: 2.0,
            is_playing: true,
            prefetch_threshold: 8,
            uses_flat_chrome: false,
            on_tap: Rc::new(on_tap),
            on_reach_end: None,
            menu_for: None,
        }
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns;
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn prefetch_threshold(mut self, prefetch_threshold: usize) -> Self {
        self.prefetch_threshold = prefetch_threshold;
        self
    }

    pub fn uses_flat_chrome(mut self, uses_flat_chrome: bool) -> Self {
        self.uses_flat_chrome = uses_flat_chrome;
        self
    }

    pub fn on_reach_end(mut self, on_reach_end: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_reach_end = Some(Rc::new(on_reach_end));
        self
    }

    pub fn menu_for(mut self, menu_for: impl Fn(&GifItem, &App) -> Vec<StickerPreviewAction> + 'static) -> Self {
        self.menu_for = Some(Rc::new(menu_for));
        self
    }

    fn child_id(&self, part: impl Into<SharedString>) -> ElementId {
        Self::named(&self.id, part)
    }

    fn named(id: &ElementId, part: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(id.clone()), part.into())
    }

    fn source(gif: &GifItem) -> String {
        if trimming_whitespaces_and_newlines(&gif.preview_url).is_empty() {
            gif.file_url.clone()
        } else {
            gif.preview_url.clone()
        }
    }

    /// unknown aspect reads as square, a provider with no dimensions would collapse the hold preview
    fn aspect(gif: &GifItem) -> f32 {
        if gif.width <= 0 || gif.height <= 0 {
            return 1.0;
        }
        let raw = gif.width as f32 / gif.height as f32;
        if raw.is_finite() && raw > 0.0 { raw } else { 1.0 }
    }

    /// pagination armed on two fixed cells so render builds no index array
    /// the last cell arms it too since a fast fling can skip the mid tail one
    fn prefetch_trigger_ids(&self) -> Option<(String, String)> {
        self.on_reach_end.as_ref()?;
        let last = self.items.last()?;
        let offset = self.prefetch_threshold.min(self.items.len()).max(1);
        Some((self.items[self.items.len() - offset].id.clone(), last.id.clone()))
    }

    /// one tile, the prefetch cells arm the next page when they appear
    fn cell(
        grid_id: &ElementId,
        gif: &GifItem,
        is_playing: bool,
        triggers: Option<&(String, String)>,
        on_reach_end: Option<&GifReachEnd>,
    ) -> AnyElement {
        let source = Self::source(gif);
        let cell = GifGridCell::new(Self::named(grid_id, format!("cell/{}", gif.id)), source.clone())
            .content_mode(AnimatedGifContentMode::ScaleAspectFit)
            .is_playing(is_playing)
            .fills_letterbox_with_media(true)
            .releases_hidden_frames(true)
            .sticker_hold_cell_frame(gif.id.clone(), StickerPreviewAsset::Gif(source, Self::aspect(gif)), None)
            .into_any_element();
        let arms_prefetch = triggers.is_some_and(|(early, last)| gif.id == *early || gif.id == *last);
        match (arms_prefetch, on_reach_end.cloned()) {
            (true, Some(on_reach_end)) => div()
                .child(cell)
                .on_appear(Self::named(grid_id, format!("appear/{}", gif.id)), move |window, cx| on_reach_end(window, cx))
                .into_any_element(),
            _ => cell,
        }
    }

    fn hold_modifier(&self) -> StickerHoldGridModifier {
        let items_for_primary = self.items.clone();
        let on_tap = self.on_tap.clone();
        let items_for_menu = self.items.clone();
        let menu_for = self.menu_for.clone();
        StickerHoldGridModifier::new(
            self.child_id("hold"),
            Rc::new(move |entry: &StickerHoldEntry, window: &mut Window, cx: &mut App| {
                if let Some(gif) = items_for_primary.iter().find(|gif| gif.id == entry.key) {
                    on_tap(gif, window, cx);
                }
            }),
        )
        .menu_for(Rc::new(move |entry: &StickerHoldEntry, cx: &App| {
            let (Some(menu_for), Some(gif)) = (menu_for.as_ref(), items_for_menu.iter().find(|gif| gif.id == entry.key))
            else {
                return Vec::new();
            };
            menu_for(gif, cx)
        }))
        .uses_flat_chrome(self.uses_flat_chrome)
    }

    pub fn virtual_list(self, size: Size<Pixels>, inset: f32, bottom_inset: f32) -> AnyElement {
        let width = f32::from(size.width) - inset * 2.0;
        if width <= 0.0 || f32::from(size.height) <= 0.0 {
            return div().into_any_element();
        }
        let columns = ChatInputPickerMetrics::gif_columns(width, self.columns.max(1)).max(1);
        let spacing = self.spacing;
        let cell = ((width - spacing * (columns as f32 - 1.0)) / columns as f32).max(1.0);
        let triggers = self.prefetch_trigger_ids();
        let rows: Rc<Vec<Vec<GifItem>>> = Rc::new(self.items.chunks(columns).map(<[GifItem]>::to_vec).collect());
        let row_count = rows.len();
        let grid_id = self.id.clone();
        let is_playing = self.is_playing;
        let on_reach_end = self.on_reach_end.clone();
        let hold = self.hold_modifier();
        uniform_list(self.child_id("rows"), row_count, move |range, _window, _cx| {
            range
                .filter_map(|row| rows.get(row))
                .map(|row| {
                    hstack(spacing)
                        .h(px(cell + spacing))
                        .items_start()
                        .children(row.iter().map(|gif| {
                            div().flex_none().w(px(cell)).child(Self::cell(
                                &grid_id,
                                gif,
                                is_playing,
                                triggers.as_ref(),
                                on_reach_end.as_ref(),
                            ))
                        }))
                        .into_any_element()
                })
                .collect::<Vec<AnyElement>>()
        })
        .w(size.width)
        .h(size.height)
        .px(px(inset))
        .pb(px(bottom_inset))
        .sticker_hold_grid(hold)
        .into_any_element()
    }
}

impl RenderOnce for GifGrid {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let measured_width = window.use_keyed_state(self.child_id("width"), cx, |_, _| 0.0_f32);
        let columns = ChatInputPickerMetrics::gif_columns(*measured_width.read(cx), self.columns.max(1));
        let triggers = self.prefetch_trigger_ids();

        let cells = self
            .items
            .iter()
            .map(|gif| Self::cell(&self.id, gif, self.is_playing, triggers.as_ref(), self.on_reach_end.as_ref()));

        let grid = div()
            .w_full()
            .grid()
            .grid_cols(columns as u16)
            .gap(px(self.spacing))
            .children(cells.collect::<Vec<_>>());

        let width_state = measured_width.clone();
        div()
            .relative()
            .w_full()
            .child(grid.sticker_hold_grid(self.hold_modifier()))
            .child(measure(self.child_id("measure"), move |bounds, _window, cx| {
                let width = f32::from(bounds.size.width);
                width_state.update(cx, |measured, cx| {
                    if *measured != width {
                        *measured = width;
                        cx.notify();
                    }
                });
            }))
    }
}

/// square tile, the profile gif tab fills it to match the photo grid
/// the picker fits into it so nothing is cropped
#[derive(IntoElement)]
pub struct GifGridCell {
    id: ElementId,
    url: String,
    content_mode: AnimatedGifContentMode,
    is_playing: bool,
    fills_letterbox_with_media: bool,
    releases_hidden_frames: bool,
}

impl GifGridCell {
    pub fn new(id: impl Into<ElementId>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            content_mode: AnimatedGifContentMode::ScaleAspectFill,
            is_playing: true,
            fills_letterbox_with_media: false,
            releases_hidden_frames: false,
        }
    }

    pub fn content_mode(mut self, content_mode: AnimatedGifContentMode) -> Self {
        self.content_mode = content_mode;
        self
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn fills_letterbox_with_media(mut self, fills_letterbox_with_media: bool) -> Self {
        self.fills_letterbox_with_media = fills_letterbox_with_media;
        self
    }

    pub fn releases_hidden_frames(mut self, releases_hidden_frames: bool) -> Self {
        self.releases_hidden_frames = releases_hidden_frames;
        self
    }

    fn child_id(&self, part: &str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), part.into())
    }
}

impl RenderOnce for GifGridCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let backdrop = window.use_keyed_state(self.child_id("backdrop"), cx, |_, _| None::<Arc<RenderImage>>);
        let reset = backdrop.clone();
        on_change(self.child_id("url"), self.url.clone(), false, window, cx, move |_, _, _, cx| {
            reset.update(cx, |backdrop, cx| {
                *backdrop = None;
                cx.notify();
            });
        });

        let mut gif = AnimatedGifView::new(self.child_id("gif"), self.url.clone())
            .content_mode(self.content_mode)
            .is_playing(self.is_playing)
            .releases_hidden_frames(self.releases_hidden_frames);
        if self.fills_letterbox_with_media {
            let sink = backdrop.clone();
            gif = gif.on_backdrop_image(move |image, cx| {
                sink.update(cx, |backdrop, cx| {
                    *backdrop = Some(image);
                    cx.notify();
                });
            });
        }

        let mut tile = div().relative().w_full().aspect_ratio(1.0).overflow_hidden();
        if let Some(image) = backdrop.read(cx).clone() {
            tile = tile.child(
                div()
                    .absolute()
                    .inset_0()
                    .overflow_hidden()
                    .opacity(0.5)
                    .child(img(ImageSource::Render(image)).object_fit(ObjectFit::Cover).size_full()),
            );
        }
        tile.child(div().absolute().inset_0().child(gif))
    }
}
