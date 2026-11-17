use std::cmp::Reverse;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, Empty, FontWeight, Hsla, ObjectFit, Rgba, SharedString, StyledImage, Window, black,
    canvas, fill, img, point, size, transparent_black, white,
};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::IconUi;
use rise_ui::main_text::TextAlignT;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::radial_gradient::RadialGradient;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::ui::bottom_sheet_ui::sheet_floating_header::{SheetFloatingHeader, SheetFloatingHeaderMetrics};
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapEncoder, CleverImageBitmapLoader,
};
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::modules::gifts::components::gift_card_view::{
    GiftBackdropColors, GiftRarityStyle, GiftSerialFormat, GiftSymbolImages,
};
use crate::modules::gifts::components::gift_cosmetic_preview::{
    GiftCosmeticThumb, GiftDropChanceText, GiftRarityTierLabel,
};
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAttributeDTO, RiseGiftAttributeExt, RiseGiftAttributeKind, RiseGiftRarityClass,
};
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::{
    GiftOddsRequest, GiftsInteractionsStore,
};
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// every variation of one kind a gift can roll, rarest last
/// name, rarity tier the admin named and the drop chance on its own line and for an optional
/// kind the chance of rolling none
#[derive(IntoElement)]
pub struct GiftOddsSheetContent;

impl GiftOddsSheetContent {
    pub fn new() -> Self {
        Self
    }

    fn list(request: &GiftOddsRequest, cx: &mut App) -> Vec<AnyElement> {
        let theme_store = ThemeStore::shared(cx);
        let theme = theme_store.read(cx).current_theme().clone();
        let services = GiftsServicesStore::shared(cx);
        let attributes =
            services.read(cx).catalog_details(cx).get(&request.gift_id).map(|details| details.attributes.clone());
        let Some(attributes) = attributes else {
            return vec![div().py(px(40.0)).child(progress_view()).into_any_element()];
        };

        let mut items: Vec<RiseGiftAttributeDTO> = attributes
            .into_iter()
            .filter(|attribute| attribute.attribute_kind() == Some(request.kind) && attribute.weight_permille > 0)
            .collect();
        items.sort_by_key(|attribute| Reverse(attribute.weight_permille));
        let taken: i64 = items.iter().map(|attribute| attribute.weight_permille).sum();
        let none = if request.kind.required() {
            0
        } else {
            (i64::from(rise_gifts::WEIGHT_TOTAL_PERMILLE) - taken).max(0)
        };

        let divider = Self::divider(theme.border._200);
        let mut rows = vstack(0.0).w_full().rounded(px(16.0)).bg(theme.bg._200);
        for (index, attribute) in items.iter().enumerate() {
            if index > 0 {
                rows = rows.child(divider());
            }
            rows = rows.child(GiftOddsRow {
                preview: GiftVariationPreview::new(
                    ElementId::NamedInteger("gift-odds-preview".into(), attribute.id as u64),
                    attribute.clone(),
                    request.kind,
                )
                .into_any_element(),
                name: attribute.display_name().into(),
                tier: attribute.tier(),
                permille: attribute.weight_permille,
            });
        }
        if none > 0 {
            if !items.is_empty() {
                rows = rows.child(divider());
            }
            rows = rows.child(GiftOddsRow {
                preview: IconUi::sized("circle.slash", px(18.0), theme.text.secondary, false)
                    .map_or_else(|| Empty.into_any_element(), IntoElement::into_any_element),
                name: tr(&format!("gift_odds_none_{}", request.kind.slug())).into(),
                tier: None,
                permille: none,
            });
        }

        vec![
            div().w_full().px(px(16.0)).child(rows).into_any_element(),
            div()
                .w_full()
                .px(px(24.0))
                .child(SecondaryText::new(tr("gift_odds_note")).px(12.0).tac(TextAlignT::Center))
                .into_any_element(),
        ]
    }

    fn divider(color: Hsla) -> impl Fn() -> AnyElement {
        move || div().w_full().pl(px(70.0)).child(div().w_full().h(px(0.5)).bg(color)).into_any_element()
    }
}

impl RenderOnce for GiftOddsSheetContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let interactions = GiftsInteractionsStore::shared(cx);
        let request = interactions.read(cx).odds_request.clone();

        let mut column = vstack(14.0)
            .w_full()
            .pb(px(24.0))
            .child(div().w_full().h(px(SheetFloatingHeaderMetrics::HEIGHT - 14.0)));
        if let Some(request) = request.as_ref() {
            column = column.children(Self::list(request, cx));
        }
        let title = request
            .as_ref()
            .map(|request| tr(&format!("gift_odds_title_{}", request.kind.slug())))
            .unwrap_or_default();

        div()
            .relative()
            .size_full()
            .child(scroll_view("gift-odds-scroll").shows_indicators(false).child(column))
            .child(div().absolute().top_0().left_0().right_0().child(SheetFloatingHeader::new(
                "gift-odds-header",
                title,
                move |_, cx| interactions.update(cx, |interactions, cx| interactions.close_odds(cx)),
            )))
    }
}

#[derive(IntoElement)]
struct GiftOddsRow {
    preview: AnyElement,
    name: SharedString,
    tier: Option<RiseGiftRarityClass>,
    permille: i64,
}

impl RenderOnce for GiftOddsRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let text_primary = theme_store.read(cx).current_theme().text.primary;
        let mut title = hstack(6.0).w_full().child(
            div()
                .min_w_0()
                .swiftui_font(Font::system_weight(16.0, FontWeight::NORMAL))
                .text_color(text_primary)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(self.name),
        );
        if let Some(tier) = self.tier {
            title = title.child(div().flex_none().child(GiftRarityTierLabel::new(tier)));
        }
        hstack(12.0)
            .w_full()
            .px(px(14.0))
            .py(px(9.0))
            .child(div().flex().flex_none().items_center().justify_center().size(px(42.0)).child(self.preview))
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(title)
                    .child(GiftDropChanceText::new(self.permille).tier(self.tier)),
            )
    }
}

#[derive(IntoElement)]
pub struct GiftChancePill {
    pub share: f64,
}

impl GiftChancePill {
    pub fn new(share: f64) -> Self {
        Self { share }
    }
}

impl RenderOnce for GiftChancePill {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tint = GiftRarityStyle::tint(RiseGiftRarityClass::Rare);
        div()
            .swiftui_font(Font::system_weight(13.0, FontWeight::SEMIBOLD).monospaced_digit())
            .text_color(tint)
            .px(px(8.0))
            .py(px(3.0))
            .capsule()
            .bg(tint.opacity(0.15))
            .child(SharedString::from(GiftSerialFormat::percent_share(self.share)))
    }
}

#[derive(IntoElement)]
pub struct GiftVariationPreview {
    id: ElementId,
    pub attribute: RiseGiftAttributeDTO,
    pub kind: RiseGiftAttributeKind,
}

impl GiftVariationPreview {
    pub fn new(id: impl Into<ElementId>, attribute: RiseGiftAttributeDTO, kind: RiseGiftAttributeKind) -> Self {
        Self { id: id.into(), attribute, kind }
    }
}

impl RenderOnce for GiftVariationPreview {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let theme = theme_store.read(cx).current_theme().clone();
        let chip = |fill: Hsla| div().relative().size_full().rounded(px(10.0)).bg(fill);
        let attribute = self.attribute;
        match self.kind {
            RiseGiftAttributeKind::Model | RiseGiftAttributeKind::Effect => {
                let fill = if self.kind == RiseGiftAttributeKind::Effect { black().opacity(0.55) } else { theme.bg._100 };
                chip(fill)
                    .overflow_hidden()
                    .child(
                        div()
                            .size_full()
                            .p(px(2.0))
                            .child(GiftCosmeticThumb::new(child_id(&self.id, "thumb"), attribute.media.clone())),
                    )
                    .into_any_element()
            }
            RiseGiftAttributeKind::Backdrop => {
                let colors = GiftBackdropColors::resolve(Some(&attribute), RiseGiftRarityClass::Common);
                let gradient =
                    RadialGradient::new([colors.center, colors.edge], UnitPoint::CENTER, 0.0, 24.0);
                div()
                    .relative()
                    .size_full()
                    .child(gift_circle_gradient(gradient).absolute().inset_0())
                    .child(centred_stroke(white().opacity(0.18), 1.0, Shape::Circle))
                    .into_any_element()
            }
            RiseGiftAttributeKind::Pattern => {
                let mut layer = chip(theme.bg._100);
                if !attribute.image_url.is_empty() {
                    layer = layer.child(div().size_full().p(px(9.0)).child(GiftSymbolView::new(
                        child_id(&self.id, "symbol"),
                        attribute.image_url.clone(),
                        theme.text.primary,
                    )));
                }
                layer.into_any_element()
            }
            RiseGiftAttributeKind::Frame => {
                let mut layer = chip(theme.bg._100);
                if !attribute.image_url.is_empty() {
                    layer = layer.child(
                        CleverImageUi::new(attribute.image_url.clone())
                            .id(child_id(&self.id, "frame"))
                            .content_mode(ContentMode::Fit)
                            .corner_radius(0.0)
                            .persist_to_disk(true)
                            .shows_progress_indicator(false)
                            .maximum_decode_pixel_dimension(128_i64)
                            .shows_failure_icon(false)
                            .placeholder_color(transparent_black()),
                    );
                }
                layer.into_any_element()
            }
        }
    }
}

fn gift_circle_gradient(gradient: RadialGradient) -> gpui::Canvas<RadialGradient> {
    canvas(
        move |_, _, _| gradient,
        |bounds, gradient, window, _| {
            let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0;
            let center = bounds.center();
            let mut disc = radius.ceil();
            while disc > 0.0 {
                let reach = disc.min(radius);
                let side = px(reach * 2.0);
                let origin = point(center.x - px(reach), center.y - px(reach));
                window.paint_quad(
                    fill(gpui::Bounds { origin, size: size(side, side) }, gradient.color_at_distance(reach))
                        .corner_radii(px(reach)),
                );
                disc -= 1.0;
            }
        },
    )
}

#[derive(Default)]
struct GiftSymbolViewState {
    image: Option<CleverImage>,
}

#[derive(IntoElement)]
pub struct GiftSymbolView {
    id: ElementId,
    pub url: String,
    pub color: Hsla,
}

impl GiftSymbolView {
    pub fn new(id: impl Into<ElementId>, url: impl Into<String>, color: Hsla) -> Self {
        Self { id: id.into(), url: url.into(), color }
    }

    async fn template_image(url: &str, color: Hsla, cx: &gpui::AsyncApp) -> Option<CleverImage> {
        let symbol = GiftSymbolImages::shared().image(url, cx).await?;
        CleverImageBitmapLoader::prepare_bitmap(
            move || {
                let mut bitmap = CleverImageBitmapEncoder::rgba(&symbol)?;
                let tint = Rgba::from(color);
                let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                let (red, green, blue) = (channel(tint.r), channel(tint.g), channel(tint.b));
                for pixel in bitmap.pixels_mut() {
                    let alpha = f32::from(pixel.0[3]) * tint.a;
                    pixel.0 = [red, green, blue, alpha.round() as u8];
                }
                Some(CleverImageBitmapDecoder::render_image(bitmap))
            },
            cx,
        )
        .await
    }
}

impl RenderOnce for GiftSymbolView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| GiftSymbolViewState::default());
        let (url, color) = (self.url, self.color);
        let task_state = state.clone();
        let task_url = url.clone();
        use_task(child_id(&self.id, "task"), (url, color), window, cx, async move |cx| {
            let image = Self::template_image(&task_url, color, cx).await;
            let _ = cx.update(|window, cx| {
                let previous = task_state.update(cx, |state, cx| {
                    cx.notify();
                    std::mem::replace(&mut state.image, image)
                });
                if let Some(previous) = previous {
                    let _ = window.drop_image(previous);
                }
            });
        });
        let mut layer = div().size_full();
        if let Some(image) = state.read(cx).image.clone() {
            layer = layer.child(img(image).size_full().object_fit(ObjectFit::Contain));
        }
        layer
    }
}
