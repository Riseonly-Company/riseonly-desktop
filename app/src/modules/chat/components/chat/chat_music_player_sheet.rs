use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, Context, ElementId, Entity, Global, IntoElement, Render, RenderOnce, Window, div, px};
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::swiftui::binding::Binding;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::music::widgets::bottomsheets::now_playing::music_queue_sheet::MusicQueueSheet;

#[derive(IntoElement)]
pub struct ChatMusicPlayerSheet {
    id: ElementId,
    on_minimum_height: Option<Rc<dyn Fn(f32, &mut Window, &mut App)>>,
}

impl ChatMusicPlayerSheet {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), on_minimum_height: None }
    }

    pub fn on_minimum_height(mut self, on_minimum_height: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_minimum_height = Some(Rc::new(on_minimum_height));
        self
    }
}

impl RenderOnce for ChatMusicPlayerSheet {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let sheet = MusicQueueSheet::new(self.id);
        match self.on_minimum_height {
            Some(on_minimum_height) => sheet.on_minimum_height(move |height, window, cx| on_minimum_height(height, window, cx)),
            None => sheet,
        }
    }
}

struct ChatMusicPlayerSheetLayerGlobal(Entity<ChatMusicPlayerSheetLayer>);

impl Global for ChatMusicPlayerSheetLayerGlobal {}

/// desktop only, the chat player sheet as its own root layer
/// inside the chat screen its opening and seek bar notified the screen every frame
pub struct ChatMusicPlayerSheetLayer {
    is_presented: bool,
    lowest_fraction: f32,
    screen_height: f32,
}

impl ChatMusicPlayerSheetLayer {
    pub const FRACTIONS: [f32; 2] = [0.72, 0.92];

    pub fn install(cx: &mut App) -> Entity<Self> {
        if let Some(layer) = cx.try_global::<ChatMusicPlayerSheetLayerGlobal>() {
            return layer.0.clone();
        }
        let layer = cx.new(|_| Self { is_presented: false, lowest_fraction: Self::FRACTIONS[0], screen_height: 0.0 });
        cx.set_global(ChatMusicPlayerSheetLayerGlobal(layer.clone()));
        layer
    }

    fn shared(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<ChatMusicPlayerSheetLayerGlobal>().map(|layer| layer.0.clone())
    }

    /// the now playing bar opens the sheet, screen height sizes its lowest detent
    pub fn present(screen_height: f32, cx: &mut App) {
        let Some(layer) = Self::shared(cx) else { return };
        layer.update(cx, |layer, cx| {
            layer.screen_height = screen_height;
            layer.is_presented = true;
            cx.notify();
        });
    }

    pub fn dismiss(cx: &mut App) {
        let Some(layer) = Self::shared(cx) else { return };
        layer.update(cx, |layer, cx| {
            if layer.is_presented {
                layer.is_presented = false;
                cx.notify();
            }
        });
    }

    fn snap_fractions(&self) -> Vec<f32> {
        let top = Self::FRACTIONS[1];
        if self.lowest_fraction < top { vec![self.lowest_fraction, top] } else { vec![self.lowest_fraction] }
    }

    fn update_lowest_fraction(&mut self, content_height: f32, cx: &mut Context<Self>) {
        if self.screen_height <= 0.0 {
            return;
        }
        let chrome = BottomSheetMetrics::HANDLE_TOP_PADDING
            + BottomSheetMetrics::HANDLE_HEIGHT
            + BottomSheetMetrics::HANDLE_BOTTOM_PADDING;
        let needed = ((content_height + chrome) / self.screen_height).min(1.0);
        let fraction = Self::FRACTIONS[0].max(needed);
        if (fraction - self.lowest_fraction).abs() <= 0.005 {
            return;
        }
        self.lowest_fraction = fraction;
        cx.notify();
    }
}

impl Render for ChatMusicPlayerSheetLayer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layer = cx.entity();
        let measured = layer.downgrade();
        let sheet = BottomSheetUi::new(
            "chat-music-sheet",
            Binding::entity(layer, |layer| layer.is_presented, |layer, value, cx| {
                layer.is_presented = value;
                cx.notify();
            }),
            move |_, _| {
                let measured = measured.clone();
                ChatMusicPlayerSheet::new("chat-music-player-sheet").on_minimum_height(move |height, _, cx| {
                    let _ = measured.update(cx, |layer, cx| layer.update_lowest_fraction(height, cx));
                })
            },
            |_, _| div().h(px(0.0)),
        )
        .snap_fractions(self.snap_fractions())
        .content_is_edge_to_edge(true)
        .footer_shows_top_border(false)
        .content_scrollable(false)
        .sheet_background(Some(gpui::black()))
        .footer_background(Some(gpui::black()));
        div().size_full().child(div().absolute().inset_0().child(sheet))
    }
}
