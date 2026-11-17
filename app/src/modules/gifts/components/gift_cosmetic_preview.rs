use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Size, Styled, Window, div, px};
use rise_i18n::number_format::percent_number_formatter::PercentNumberFormatter;
use rise_i18n::{FormatArg, I18n, format_printf, tr};
use rise_theme::theme::app_theme::alpha;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::frame::{EdgeSet, SwiftUiFrame};
use rise_ui::swiftui::radial_gradient::RadialGradient;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, pass_through};
use rise_ui::swiftui::stacks::{Alignment, hstack, zstack};
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::components::gift_animation_view::GiftAnimationView;
use crate::modules::gifts::components::gift_card_view::{
    GiftBackdropColors, GiftPatternLayer, GiftRarityStyle, geometry_size, geometry_size_seeded,
};
use crate::modules::gifts::components::gift_cosmetic_playback::GiftCosmeticGeometry;
use crate::modules::gifts::components::gift_frame_view::GiftFrameView;
use crate::modules::gifts::components::gift_nameplate_view::{GiftNameplateRow, GiftNameplateView};
use crate::modules::gifts::components::gift_profile_effect_view::{GiftProfileEffectBanner, GiftProfileEffectView};
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAttributeDTO, RiseGiftInstanceDTO, RiseGiftItemKind, RiseGiftMediaDTO, RiseGiftMediaExt, RiseGiftMediaKind,
    RiseGiftRarityClass,
};
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftRecipient;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// what a cosmetic card draws instead of an item animation, the kind and its media and whom
/// its shown on
#[derive(Clone, Debug, PartialEq)]
pub struct GiftCosmeticContent {
    pub kind: RiseGiftItemKind,
    pub model: Option<RiseGiftMediaDTO>,
    pub effect: Option<RiseGiftMediaDTO>,
    pub person: Option<GiftRecipient>,
    pub bare_nameplate: bool,
}

/// a cosmetic worn by a person, fitted into any box
/// frame around the photo or member row over a nameplate or small banner with the effect
/// bare_nameplate shows the nameplate alone, everything plays once per appearance
#[derive(IntoElement)]
pub struct GiftCosmeticPreview {
    id: ElementId,
    kind: RiseGiftItemKind,
    person: Option<GiftRecipient>,
    model: Option<RiseGiftMediaDTO>,
    effect: Option<RiseGiftMediaDTO>,
    animated: bool,
    bare_nameplate: bool,
    size_hint: Option<Size<f32>>,
}

impl GiftCosmeticPreview {
    pub fn new(
        id: impl Into<ElementId>,
        kind: RiseGiftItemKind,
        person: Option<GiftRecipient>,
        model: Option<RiseGiftMediaDTO>,
    ) -> Self {
        Self { id: id.into(), kind, person, model, effect: None, animated: true, bare_nameplate: false, size_hint: None }
    }

    pub fn effect(mut self, effect: Option<RiseGiftMediaDTO>) -> Self {
        self.effect = effect;
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    pub fn bare_nameplate(mut self, bare_nameplate: bool) -> Self {
        self.bare_nameplate = bare_nameplate;
        self
    }

    /// size the parent gives the preview, its first render draws at it
    pub fn size_hint(mut self, size_hint: Option<Size<f32>>) -> Self {
        self.size_hint = size_hint;
        self
    }
}

impl RenderOnce for GiftCosmeticPreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (size, reader) = geometry_size_seeded(child_id(&self.id, "geometry"), self.size_hint, window, cx);
        let content = match self.kind {
            RiseGiftItemKind::AvatarFrame => {
                let side = (size.width.min(size.height) / GiftCosmeticGeometry::frame_canvas_scale()).max(1.0);
                GiftFrameView::new(
                    child_id(&self.id, "frame"),
                    self.model.clone(),
                    side,
                    GiftPersonAvatar::new(child_id(&self.id, "avatar"), self.person.clone(), side).into_any_element(),
                )
                .effect(self.effect.clone())
                .animated(self.animated)
                .into_any_element()
            }
            RiseGiftItemKind::Nameplate => {
                let height = GiftCosmeticGeometry::MEMBER_ROW_HEIGHT
                    .min((size.width * 0.38).max(28.0))
                    .min(size.height);
                GiftNameplateRow::new(child_id(&self.id, "nameplate"), self.person.clone(), self.model.clone())
                    .effect(self.effect.clone())
                    .height(height)
                    .animated(self.animated)
                    .shows_person(!self.bare_nameplate)
                    .into_any_element()
            }
            RiseGiftItemKind::ProfileEffect => div()
                .w_full()
                .max_w(px(size.width.min(size.height * GiftCosmeticGeometry::BANNER_ASPECT)))
                .child(
                    GiftProfileEffectBanner::new(child_id(&self.id, "banner"), self.person.clone(), self.model.clone())
                        .animated(self.animated),
                )
                .into_any_element(),
            RiseGiftItemKind::Item => GiftAnimationView::new(child_id(&self.id, "item"), self.model.clone())
                .large(size.width > 160.0)
                .animated(self.animated)
                .into_any_element(),
        };
        div()
            .relative()
            .size_full()
            .child(reader)
            .child(zstack(Alignment::Center).frame_size(size.width, size.height).child(content))
    }
}

#[derive(IntoElement)]
pub struct GiftPersonAvatar {
    id: ElementId,
    person: Option<GiftRecipient>,
    size: f32,
}

impl GiftPersonAvatar {
    pub fn new(id: impl Into<ElementId>, person: Option<GiftRecipient>, size: f32) -> Self {
        Self { id: id.into(), person, size }
    }
}

impl RenderOnce for GiftPersonAvatar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if let Some(person) = self.person {
            let source = (!person.avatar.is_empty()).then(|| SharedString::from(person.avatar.clone()));
            return UserLogoUi::new(self.id)
                .source(source)
                .name(Some(SharedString::from(person.name.clone())))
                .user_id(Some(SharedString::from(person.id.clone())))
                .size(self.size)
                .show_story_ring(false)
                .into_any_element();
        }
        let fill = ThemeStore::shared(cx).read(cx).current_theme().bg._300;
        div().frame_size(self.size, self.size).circle().bg(fill).into_any_element()
    }
}

/// ground of a card, the rolled backdrop gradient with pattern rings or the plain shop card
/// when nothing was rolled
/// a cosmetic card passes the receding pattern strength
#[derive(IntoElement)]
pub struct GiftBackdropSurface {
    id: ElementId,
    backdrop: Option<RiseGiftAttributeDTO>,
    pattern: Option<RiseGiftAttributeDTO>,
    rarity: RiseGiftRarityClass,
    pattern_strength: f64,
    size_hint: Option<Size<f32>>,
}

impl GiftBackdropSurface {
    pub fn new(id: impl Into<ElementId>, backdrop: Option<RiseGiftAttributeDTO>) -> Self {
        Self {
            id: id.into(),
            backdrop,
            pattern: None,
            rarity: RiseGiftRarityClass::Common,
            pattern_strength: 1.0,
            size_hint: None,
        }
    }

    pub fn pattern(mut self, pattern: Option<RiseGiftAttributeDTO>) -> Self {
        self.pattern = pattern;
        self
    }

    pub fn rarity(mut self, rarity: RiseGiftRarityClass) -> Self {
        self.rarity = rarity;
        self
    }

    pub fn pattern_strength(mut self, pattern_strength: f64) -> Self {
        self.pattern_strength = pattern_strength;
        self
    }

    /// size the parent gives the ground, first render draws at it and the pattern too
    pub fn size_hint(mut self, size_hint: Option<Size<f32>>) -> Self {
        self.size_hint = size_hint;
        self
    }
}

impl RenderOnce for GiftBackdropSurface {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = GiftBackdropColors::resolve(self.backdrop.as_ref(), self.rarity);
        let (size, reader) = geometry_size_seeded(child_id(&self.id, "geometry"), self.size_hint, window, cx);
        let side = size.width.min(size.height);
        let layer = || div().absolute().inset_0();
        let mut stack = div().relative().size_full().child(reader);
        if colors.is_rolled {
            stack = stack.child(layer().child(
                RadialGradient::new(
                    [colors.center, colors.edge],
                    UnitPoint::new(0.5, 0.42),
                    0.0,
                    side.max(size.width * 0.6) * 0.75,
                )
                .view(),
            ));
        } else {
            let ground = ThemeStore::shared(cx).read(cx).current_theme().bg._200;
            stack = stack.child(layer().bg(ground));
        }
        if let Some(symbol) = self.pattern.as_ref().map(|pattern| pattern.image_url.clone())
            && !symbol.is_empty()
        {
            stack = stack.child(layer().child(
                GiftPatternLayer::new(child_id(&self.id, "pattern"), symbol, colors.pattern)
                    .strength(self.pattern_strength)
                    .size_hint(self.size_hint.map(|_| size)),
            ));
        }
        pass_through().size_full().child(stack)
    }
}

/// a cosmetic on its card in a focused place like the send form or result or offer
/// backdrop and pattern when known, preview on the person animated and the edition ribbon
#[derive(IntoElement)]
pub struct GiftCosmeticStage {
    id: ElementId,
    kind: RiseGiftItemKind,
    person: Option<GiftRecipient>,
    model: Option<RiseGiftMediaDTO>,
    effect: Option<RiseGiftMediaDTO>,
    backdrop: Option<RiseGiftAttributeDTO>,
    pattern: Option<RiseGiftAttributeDTO>,
    rarity: RiseGiftRarityClass,
    ribbon: Option<String>,
    animated: bool,
}

impl GiftCosmeticStage {
    pub fn new(
        id: impl Into<ElementId>,
        kind: RiseGiftItemKind,
        person: Option<GiftRecipient>,
        model: Option<RiseGiftMediaDTO>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            person,
            model,
            effect: None,
            backdrop: None,
            pattern: None,
            rarity: RiseGiftRarityClass::Common,
            ribbon: None,
            animated: true,
        }
    }

    pub fn effect(mut self, effect: Option<RiseGiftMediaDTO>) -> Self {
        self.effect = effect;
        self
    }

    pub fn backdrop(mut self, backdrop: Option<RiseGiftAttributeDTO>) -> Self {
        self.backdrop = backdrop;
        self
    }

    pub fn pattern(mut self, pattern: Option<RiseGiftAttributeDTO>) -> Self {
        self.pattern = pattern;
        self
    }

    pub fn rarity(mut self, rarity: RiseGiftRarityClass) -> Self {
        self.rarity = rarity;
        self
    }

    pub fn ribbon(mut self, ribbon: Option<String>) -> Self {
        self.ribbon = ribbon;
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    fn stage(&self) -> gpui::AnyElement {
        match self.kind {
            RiseGiftItemKind::AvatarFrame => div()
                .w_full()
                .h(px(240.0))
                .padding_edges(EdgeSet::VERTICAL, 22.0)
                .child(
                    GiftCosmeticPreview::new(child_id(&self.id, "preview"), self.kind, self.person.clone(), self.model.clone())
                        .effect(self.effect.clone())
                        .animated(self.animated),
                )
                .into_any_element(),
            RiseGiftItemKind::Nameplate => div()
                .w_full()
                .h(px(150.0))
                .flex()
                .items_center()
                .padding_edges(EdgeSet::HORIZONTAL, 16.0)
                .child(
                    GiftNameplateRow::new(child_id(&self.id, "nameplate"), self.person.clone(), self.model.clone())
                        .effect(self.effect.clone())
                        .animated(self.animated),
                )
                .into_any_element(),
            RiseGiftItemKind::ProfileEffect => div()
                .w_full()
                .p(px(14.0))
                .child(
                    GiftProfileEffectBanner::new(child_id(&self.id, "banner"), self.person.clone(), self.model.clone())
                        .animated(self.animated),
                )
                .into_any_element(),
            RiseGiftItemKind::Item => div()
                .w_full()
                .h(px(240.0))
                .p(px(12.0))
                .child(
                    GiftAnimationView::new(child_id(&self.id, "item"), self.model.clone())
                        .large(true)
                        .animated(self.animated),
                )
                .into_any_element(),
        }
    }
}

impl RenderOnce for GiftCosmeticStage {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let colors = GiftBackdropColors::resolve(self.backdrop.as_ref(), self.rarity);
        let pattern_strength = if self.kind.equippable() { GiftPatternLayer::COSMETIC_STRENGTH } else { 1.0 };
        let stage = self.stage();
        let mut card = zstack(Alignment::TopTrailing)
            .w_full()
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(
                        div().absolute().inset_0().child(
                            GiftBackdropSurface::new(child_id(&self.id, "surface"), self.backdrop.clone())
                                .pattern(self.pattern.clone())
                                .rarity(self.rarity)
                                .pattern_strength(pattern_strength),
                        ),
                    )
                    .child(stage),
            );
        if let Some(ribbon) = self.ribbon {
            card = card.child(
                div().p(px(12.0)).child(
                    div()
                        .swiftui_font(Font::system_weight(13.0, FontWeight::SEMIBOLD).monospaced_digit())
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(colors.ribbon_ink)
                        .px(px(9.0))
                        .py(px(4.0))
                        .capsule()
                        .bg(colors.ribbon)
                        .child(SharedString::from(ribbon)),
                ),
            );
        }
        card.clip_shape(Shape::continuous(22.0))
    }
}

/// a variation media in a small square moving like everywhere else
/// frame around an empty circle, right end of a nameplate an effect or an item animation
#[derive(IntoElement)]
pub struct GiftCosmeticThumb {
    id: ElementId,
    media: Option<RiseGiftMediaDTO>,
}

impl GiftCosmeticThumb {
    pub fn new(id: impl Into<ElementId>, media: Option<RiseGiftMediaDTO>) -> Self {
        Self { id: id.into(), media }
    }
}

impl RenderOnce for GiftCosmeticThumb {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (size, reader) = geometry_size(child_id(&self.id, "geometry"), window, cx);
        let side = size.width.min(size.height);
        let content = match self.media.as_ref().and_then(|media| media.media_kind()) {
            Some(RiseGiftMediaKind::FrameVideo) => {
                let avatar_size = side / GiftCosmeticGeometry::frame_canvas_scale();
                let fill = ThemeStore::shared(cx).read(cx).current_theme().bg._300;
                GiftFrameView::new(
                    child_id(&self.id, "frame"),
                    self.media.clone(),
                    avatar_size,
                    div().size_full().circle().bg(fill).into_any_element(),
                )
                .into_any_element()
            }
            Some(RiseGiftMediaKind::PlateVideo) => {
                GiftNameplateView::new(child_id(&self.id, "plate"), self.media.clone()).into_any_element()
            }
            Some(RiseGiftMediaKind::BannerVideo) => {
                GiftProfileEffectView::new(child_id(&self.id, "banner"), self.media.clone()).into_any_element()
            }
            _ => GiftAnimationView::new(child_id(&self.id, "item"), self.media.clone())
                .large(false)
                .into_any_element(),
        };
        div()
            .relative()
            .size_full()
            .child(reader)
            .child(zstack(Alignment::Center).frame_size(size.width, size.height).child(content))
    }
}

/// worn badge shown to the owner only, a check in the theme own ink like a selection mark
/// ringed in the page colour so it reads on any backdrop
#[derive(IntoElement, Default)]
pub struct GiftEquippedBadge {
    compact: bool,
}

impl GiftEquippedBadge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }
}

impl RenderOnce for GiftEquippedBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let ink = theme.text.primary;
        let paper = theme.bg._100;
        let compact = self.compact;
        let mut badge = hstack(if compact { 3.0 } else { 4.0 }).text_color(paper);
        if let Some(check) = IconUi::sized("checkmark", px(if compact { 10.0 } else { 11.0 }), paper, false) {
            badge = badge.child(check);
        }
        if !compact {
            badge = badge.child(
                div()
                    .swiftui_font(Font::system_weight(13.0, FontWeight::SEMIBOLD))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(SharedString::from(tr("gift_equip_worn"))),
            );
        }
        if compact {
            badge
                .justify_center()
                .frame_size(22.0, 22.0)
                .circle()
                .bg(ink)
                .stroke_border(paper, 1.5)
        } else {
            badge.px(px(10.0)).py(px(4.0)).capsule().bg(ink)
        }
    }
}

/// rarity tier the admin named for a variation, tinted by it
#[derive(IntoElement)]
pub struct GiftRarityTierLabel {
    tier: RiseGiftRarityClass,
}

impl GiftRarityTierLabel {
    pub fn new(tier: RiseGiftRarityClass) -> Self {
        Self { tier }
    }
}

impl RenderOnce for GiftRarityTierLabel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tint = GiftRarityStyle::tint(self.tier);
        div()
            .swiftui_font(Font::system_weight(12.0, FontWeight::SEMIBOLD))
            .text_color(tint)
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .px(px(7.0))
            .py(px(2.0))
            .capsule()
            .bg(alpha(tint, 0.15))
            .child(SharedString::from(tr(&format!("gift_rarity_tier_{}", self.tier.slug()))))
    }
}

thread_local! {
    static FORMATTERS: RefCell<HashMap<String, PercentNumberFormatter>> = RefCell::new(HashMap::new());
    static FINE_FORMATTERS: RefCell<HashMap<String, PercentNumberFormatter>> = RefCell::new(HashMap::new());
}

/// drop chance like 0.3 %, weight in permille as a percent with one decimal in the app language
/// chance of a whole instance keeps two significant digits below one percent so rare rolls
/// read 0.0045 % not zero
pub enum GiftChanceFormat {}

impl GiftChanceFormat {
    pub fn drop_chance(permille: i64) -> String {
        format_printf(&tr("gift_market_drop_chance"), &[FormatArg::Text(Self::percent_permille(permille).into())])
    }

    /// chance of exactly this roll from 0 to 1
    /// market service records it in whole parts per million, rounded down and never below one
    /// when the product of listed weights is exactly that record the product is used, else the record
    pub fn share(instance: &RiseGiftInstanceDTO) -> Option<f64> {
        let ppm = instance.chance_ppm;
        if ppm <= 0 {
            return None;
        }
        let total = f64::from(rise_gifts::WEIGHT_TOTAL_PERMILLE);
        let product = instance
            .attributes
            .iter()
            .fold(1.0_f64, |product, attribute| product * attribute.weight_permille as f64 / total);
        let recorded = ((product * 1_000_000.0 + 1e-6).floor() as i64).max(1);
        Some(if recorded == ppm { product } else { ppm as f64 / 1_000_000.0 })
    }

    /// a share as a percentage, one decimal from one percent up and two significant digits below
    pub fn percent_share(share: f64) -> String {
        if share >= 0.01 {
            return Self::percent_permille((share * f64::from(rise_gifts::WEIGHT_TOTAL_PERMILLE)).round() as i64);
        }
        let code = I18n::shared().code().to_owned();
        let formatted = FINE_FORMATTERS.with(|formatters| {
            let mut formatters = formatters.borrow_mut();
            let formatter = formatters.entry(code.clone()).or_insert_with(|| {
                let mut formatter = PercentNumberFormatter::new(&code);
                formatter.uses_significant_digits = true;
                formatter.minimum_significant_digits = 1;
                formatter.maximum_significant_digits = 2;
                formatter
            });
            formatter.string(share)
        });
        formatted.unwrap_or_else(|| format_printf("%.2g%%", &[FormatArg::Float(share * 100.0)]))
    }

    pub fn percent_permille(permille: i64) -> String {
        let code = I18n::shared().code().to_owned();
        let share = permille as f64 / f64::from(rise_gifts::WEIGHT_TOTAL_PERMILLE);
        let formatted = FORMATTERS.with(|formatters| {
            let mut formatters = formatters.borrow_mut();
            let formatter = formatters.entry(code.clone()).or_insert_with(|| {
                let mut formatter = PercentNumberFormatter::new(&code);
                formatter.minimum_fraction_digits = 1;
                formatter.maximum_fraction_digits = 1;
                formatter
            });
            formatter.string(share)
        });
        formatted.unwrap_or_else(|| format_printf("%.1f%%", &[FormatArg::Float(share * 100.0)]))
    }

    /// rare and rarer tiers colour their chance so the scarce ones stand out
    pub fn highlights(tier: Option<RiseGiftRarityClass>) -> bool {
        matches!(
            tier,
            Some(RiseGiftRarityClass::Rare | RiseGiftRarityClass::Epic | RiseGiftRarityClass::Legendary)
        )
    }
}

/// one variation chance line under its name, tinted for the scarce tiers
#[derive(IntoElement)]
pub struct GiftDropChanceText {
    permille: i64,
    tier: Option<RiseGiftRarityClass>,
    size: f32,
}

impl GiftDropChanceText {
    pub fn new(permille: i64) -> Self {
        Self { permille, tier: None, size: 13.0 }
    }

    pub fn tier(mut self, tier: Option<RiseGiftRarityClass>) -> Self {
        self.tier = tier;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for GiftDropChanceText {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let secondary = ThemeStore::shared(cx).read(cx).current_theme().text.secondary;
        let highlighted = GiftChanceFormat::highlights(self.tier);
        let weight = if highlighted { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
        let tint = if highlighted {
            GiftRarityStyle::tint(self.tier.unwrap_or(RiseGiftRarityClass::Rare))
        } else {
            secondary
        };
        div()
            .swiftui_font(Font::system_weight(self.size, weight).monospaced_digit())
            .text_color(tint)
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .child(SharedString::from(GiftChanceFormat::drop_chance(self.permille)))
    }
}
