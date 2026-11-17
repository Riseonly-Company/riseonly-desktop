use std::rc::Rc;

use gpui::{App, FontWeight, SharedString, Window, white};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_theme::theme::app_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFraction;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::ui::bottom_sheet_ui::sheet_floating_header::{SheetFloatingHeader, SheetFloatingHeaderMetrics};
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftsInteractionsStore;
use crate::modules::sparks::components::sparks_icon_view::SparksHeroArt;

fn close_sparks_info(cx: &mut App) {
    GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.close_sparks_info(cx));
}

struct Point {
    icon: &'static str,
    title_key: &'static str,
    text_key: &'static str,
}

const POINTS: [Point; 3] = [
    Point { icon: "cart.fill", title_key: "sparks_about_buy_title", text_key: "sparks_about_buy_text" },
    Point { icon: "gift.fill", title_key: "sparks_about_items_title", text_key: "sparks_about_items_text" },
    Point { icon: "heart.fill", title_key: "sparks_about_authors_title", text_key: "sparks_about_authors_text" },
];

/// about sparks, opened from more about sparks and laid over it
/// header has close button and title, body has the moving spark and three short points
/// ok at the bottom, as tall as its content and huge text sizes scroll it
#[derive(IntoElement)]
pub struct GiftSparksInfoSheetContent {
    on_height: Rc<dyn Fn(f32, &mut Window, &mut App)>,
}

impl GiftSparksInfoSheetContent {
    pub fn new(on_height: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        Self { on_height: Rc::new(on_height) }
    }

    fn row(point: &Point, theme: &AppTheme) -> Div {
        let primary = theme.primary._100;
        hstack(14.0)
            .align(VerticalAlignment::Top)
            .w_full()
            .child(
                zstack(Alignment::Center)
                    .flex_none()
                    .size(px(36.0))
                    .bg(primary.opacity(0.12))
                    .clip_shape(Shape::continuous(11.0))
                    .children(IconUi::sized(point.icon, px(16.0), primary, true)),
            )
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(16.0, FontWeight::SEMIBOLD))
                            .text_color(theme.text.primary)
                            .child(SharedString::from(tr(point.title_key))),
                    )
                    .child(
                        div()
                            .swiftui_font(Font::system_size(14.0))
                            .text_color(theme.text.secondary)
                            .child(SharedString::from(tr(point.text_key))),
                    ),
            )
    }
}

impl RenderOnce for GiftSparksInfoSheetContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = ThemeStore::shared(cx);
        let theme = store.read(cx).current_theme().clone();
        let on_height = self.on_height;
        let header = SheetFloatingHeader::new("gift-sparks-info-header", tr("sparks_about_title"), |_, cx| {
            close_sparks_info(cx);
        });
        div()
            .relative()
            .size_full()
            .child(
                scroll_view("gift-sparks-info-scroll").shows_indicators(false).child(
                    div()
                        .relative()
                        .w_full()
                        .child(
                            vstack(0.0)
                                .w_full()
                                .pt(px(SheetFloatingHeaderMetrics::HEIGHT))
                                .pb(px(20.0))
                                .child(SparksHeroArt::new().size(112.0))
                                .child(
                                    vstack(18.0)
                                        .align(HorizontalAlignment::Leading)
                                        .w_full()
                                        .px(px(24.0))
                                        .pt(px(14.0))
                                        .children(POINTS.iter().map(|point| Self::row(point, &theme))),
                                ),
                        )
                        .child(measure("gift-sparks-info-height", move |bounds, window, cx| {
                            on_height(f32::from(bounds.size.height), window, cx);
                        })),
                ),
            )
            .child(div().absolute().top_0().left_0().right_0().child(header))
    }
}

/// ok button under the explanation
#[derive(IntoElement)]
pub struct GiftSparksInfoSheetFooter;

impl GiftSparksInfoSheetFooter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GiftSparksInfoSheetFooter {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for GiftSparksInfoSheetFooter {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().w_full().px(px(16.0)).py(px(GiftSparksInfoLayout::FOOTER_VERTICAL_PADDING)).child(
            ButtonUi::new("gift-sparks-info-ok").on_press(|_, cx| close_sparks_info(cx)).child(
                MainText::new(tr("gift_info_ok")).px(16.0).font_weight(FontWeight::SEMIBOLD).color(white()),
            ),
        )
    }
}

/// sheet height as a fraction of the window, content plus footer button under the shell ceiling
/// a desktop window has no bottom safe area
pub enum GiftSparksInfoLayout {}

impl GiftSparksInfoLayout {
    pub const FOOTER_VERTICAL_PADDING: f32 = 10.0;
    pub const INITIAL_FRACTION: f32 = 0.6;
    const MINIMUM_FRACTION: f32 = 0.3;

    pub fn fraction(content_height: f32, button_height: f32, window: &Window) -> f32 {
        BottomSheetMeasuredFraction::value(
            content_height + button_height + 2.0 * Self::FOOTER_VERTICAL_PADDING,
            Self::MINIMUM_FRACTION,
            window,
        )
    }
}
