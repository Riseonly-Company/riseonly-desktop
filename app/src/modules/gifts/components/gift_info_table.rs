use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, ElementId, Font, FontFeatures, FontWeight, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, font, px,
};
use rise_i18n::tr;
use rise_theme::FONT_FAMILY;
use rise_ui::swiftui::environment::{EnvKey, env, env_scope};
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes};
use rise_ui::skeleton_ui::SkeletonLine;
use rise_ui::{IconUi, MainText, SecondaryText};
use smallvec::SmallVec;

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAttributeDTO, RiseGiftAttributeExt, RiseGiftAttributeKind,
};
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftsInteractionsStore;

/// a gift facts laid out as a table, label column on a darker ground and the value beside it
/// hairlines between rows, rounded and outlined as a whole rows say whether a rule goes above them
#[derive(IntoElement, Default)]
pub struct GiftInfoTable {
    content: SmallVec<[AnyElement; 4]>,
}

impl GiftInfoTable {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ParentElement for GiftInfoTable {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.content.extend(elements);
    }
}

const TABLE_CORNER_RADIUS: f32 = 16.0;
const TABLE_BORDER_WIDTH: f32 = 1.0;

struct GiftInfoRowEdgesKey;

impl EnvKey for GiftInfoRowEdgesKey {
    type Value = (bool, bool);

    fn default_value() -> Self::Value {
        (false, false)
    }

    fn fingerprint(value: &Self::Value) -> Option<u64> {
        Some(u64::from(value.0) | (u64::from(value.1) << 1))
    }
}

impl RenderOnce for GiftInfoTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let count = self.content.len();
        div()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .clip_shape(Shape::continuous(TABLE_CORNER_RADIUS))
            .stroke_border(theme.border._200, TABLE_BORDER_WIDTH)
            .children(self.content.into_iter().enumerate().map(|(index, row)| {
                env_scope::<GiftInfoRowEdgesKey>((index == 0, index + 1 == count)).child(row)
            }))
    }
}

#[derive(IntoElement)]
pub struct GiftInfoRow {
    pub label: Option<SharedString>,
    pub rule_above: bool,
    value: AnyElement,
}

impl GiftInfoRow {
    pub const LABEL_WIDTH: f32 = 112.0;

    pub fn new(label: Option<impl Into<SharedString>>, value: impl IntoElement) -> Self {
        Self {
            label: label.map(Into::into),
            rule_above: true,
            value: value.into_any_element(),
        }
    }

    pub fn rule_above(mut self, rule_above: bool) -> Self {
        self.rule_above = rule_above;
        self
    }

    pub fn label_width() -> f32 {
        Self::LABEL_WIDTH
    }
}

impl RenderOnce for GiftInfoRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let (is_first, is_last) = env::<GiftInfoRowEdgesKey>(cx);
        let label_corner = px(TABLE_CORNER_RADIUS - TABLE_BORDER_WIDTH);
        let mut row = div().flex().flex_row().items_stretch().w_full().min_h(px(48.0));
        if let Some(label) = self.label {
            row = row
                .child(
                    div()
                        .when(is_first, |cell| cell.rounded_tl(label_corner))
                        .when(is_last, |cell| cell.rounded_bl(label_corner))
                        .flex()
                        .flex_col()
                        .flex_none()
                        .justify_center()
                        .w(px(Self::LABEL_WIDTH))
                        .min_w_0()
                        .overflow_hidden()
                        .px(px(14.0))
                        .py(px(6.0))
                        .bg(theme.bg._200)
                        .text_size(px(16.0))
                        .text_color(theme.text.primary)
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .child(MinimumScaleText::new(label, 0.65).line_limit(2)),
                        ),
                )
                .child(div().flex_none().w(px(1.0)).bg(theme.border._200));
        }
        row = row.child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .px(px(14.0))
                .py(px(11.0))
                .text_size(px(16.0))
                .text_color(theme.text.primary)
                .child(self.value),
        );
        let mut column = div().flex().flex_col().w_full();
        if self.rule_above {
            column = column.child(div().w_full().h(px(1.0)).bg(theme.border._200));
        }
        column.child(row)
    }
}

/// a person in a row, small photo and name tappable to their profile
#[derive(IntoElement)]
pub struct GiftInfoPerson {
    pub id: SharedString,
    pub name: SharedString,
    pub avatar: Option<SharedString>,
    pub on_open: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    pub fills_width: bool,
}

impl GiftInfoPerson {
    pub const AVATAR_SIDE: f32 = 26.0;
    pub const SPACING: f32 = 8.0;
    pub const NAME_FONT_SIZE: f32 = 16.0;
    pub const NAME_FONT_WEIGHT: FontWeight = FontWeight::MEDIUM;

    pub fn name_ui_font() -> Font {
        Font {
            weight: Self::NAME_FONT_WEIGHT,
            ..font(FONT_FAMILY)
        }
    }

    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        avatar: Option<impl Into<SharedString>>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            avatar: avatar.map(Into::into),
            on_open: None,
            fills_width: false,
        }
    }

    /// the name takes the whole width the person is given, cut only when that is too narrow
    pub fn fills_width(mut self) -> Self {
        self.fills_width = true;
        self
    }

    pub fn on_open(mut self, on_open: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open = Some(Rc::new(on_open));
        self
    }
}

impl RenderOnce for GiftInfoPerson {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let fills_width = self.fills_width;
        let mut name = div()
            .min_w_0()
            .text_size(px(Self::NAME_FONT_SIZE))
            .font_weight(Self::NAME_FONT_WEIGHT)
            .text_color(theme.primary._100)
            .line_clamp(1)
            .text_ellipsis()
            .child(self.name.clone());
        if fills_width {
            name = name.flex_1();
        }
        let mut content = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(Self::SPACING))
            .child(
                UserLogoUi::new(ElementId::Name(format!("gift-info-person-logo-{}", self.id).into()))
                    .source(self.avatar.clone())
                    .name(Some(self.name.clone()))
                    .user_id(Some(self.id.clone()))
                    .size(Self::AVATAR_SIDE)
                    .show_story_ring(false),
            )
            .child(name);
        let mut button = div().id(ElementId::Name(format!("gift-info-person-{}", self.id).into()));
        if fills_width {
            content = content.w_full();
            button = button.w_full();
        }
        let button = button.child(content);
        match self.on_open {
            Some(on_open) => button
                .cursor_pointer()
                .on_click(move |_, window, cx| on_open(window, cx))
                .into_any_element(),
            None => button.into_any_element(),
        }
    }
}

/// chevron at the trailing end of a row that opens something
fn row_chevron(cx: &App) -> AnyElement {
    let theme = rise_ui::theme(cx);
    match IconUi::sized("chevron.right", px(13.0), theme.text.secondary, false) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(13.0)).into_any_element(),
    }
}

fn kind_label(kind: RiseGiftAttributeKind) -> String {
    tr(&format!("gift_kind_{}", kind.slug()))
}

/// the kinds a gift can roll with how many variations each has, a row opens their odds
#[derive(IntoElement)]
pub struct GiftOddsSummary {
    pub gift_id: i64,
    pub attributes: Vec<RiseGiftAttributeDTO>,
}

impl GiftOddsSummary {
    pub fn new(gift_id: i64, attributes: Vec<RiseGiftAttributeDTO>) -> Self {
        Self { gift_id, attributes }
    }
}

impl RenderOnce for GiftOddsSummary {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let kinds: Vec<(RiseGiftAttributeKind, usize)> = RiseGiftAttributeKind::ALL
            .iter()
            .filter_map(|kind| {
                let count = self
                    .attributes
                    .iter()
                    .filter(|attribute| attribute.attribute_kind() == Some(*kind) && attribute.weight_permille > 0)
                    .count();
                (count > 0).then_some((*kind, count))
            })
            .collect();
        let gift_id = self.gift_id;
        let rows = kinds.into_iter().enumerate().map(|(index, (kind, count))| {
            let value = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .w_full()
                .child(
                    div()
                        .font_features(FontFeatures(Arc::new(vec![("tnum".to_string(), 1)])))
                        .child(count.to_string()),
                )
                .child(div().flex_1())
                .child(row_chevron(cx));
            div()
                .id(ElementId::NamedInteger("gift-odds-kind".into(), index as u64))
                .w_full()
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    let store = GiftsInteractionsStore::shared(cx);
                    store.update(cx, |store, cx| store.open_odds(gift_id, kind, cx));
                })
                .child(GiftInfoRow::new(Some(kind_label(kind)), value).rule_above(index > 0))
        }).collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(MainText::new(tr("gift_odds_title")).px(15.0).font_weight(FontWeight::SEMIBOLD))
            .child(GiftInfoTable::new().children(rows))
            .child(SecondaryText::new(tr("gift_odds_note")).px(12.0))
    }
}

/// the odds summary while odds load, same title rows and note with counts as skeletons
/// so the caption below doesnt jump when they arrive
#[derive(IntoElement)]
pub struct GiftOddsSummaryPlaceholder {
    pub kinds: Vec<RiseGiftAttributeKind>,
}

impl GiftOddsSummaryPlaceholder {
    pub fn new(kinds: Vec<RiseGiftAttributeKind>) -> Self {
        Self { kinds }
    }
}

impl RenderOnce for GiftOddsSummaryPlaceholder {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let rows = self.kinds.into_iter().enumerate().map(|(index, kind)| {
            let value = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .w_full()
                .child(SkeletonLine::new(18.0, 12.0))
                .child(div().flex_1())
                .child(row_chevron(cx));
            GiftInfoRow::new(Some(kind_label(kind)), value).rule_above(index > 0)
        }).collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(MainText::new(tr("gift_odds_title")).px(15.0).font_weight(FontWeight::SEMIBOLD))
            .child(GiftInfoTable::new().children(rows))
            .child(SecondaryText::new(tr("gift_odds_note")).px(12.0))
    }
}
