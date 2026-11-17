use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, Font as TextFont, FontWeight, HighlightStyle, Hsla, SharedString,
    StyledText, TextRun, Window, black, font, transparent_black, white,
};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};
use rise_i18n::number_format::decimal_number_formatter::DecimalNumberFormatter;
use rise_i18n::{FormatArg, I18n, format_printf, tr};
use rise_theme::theme::app_theme::AppTheme;
use rise_theme::{FONT_FAMILY, ThemeStore};
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::radial_gradient::RadialGradient;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::UnitPoint;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::{SheetFloatingHeader, SheetFloatingHeaderMetrics};
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::components::gift_animation_view::GiftAnimationView;
use crate::modules::gifts::components::gift_card_view::{
    GiftBackdropColors, GiftPatternLayer, GiftRarityPill, GiftRarityStyle,
};
use crate::modules::gifts::components::gift_cosmetic_playback::GiftCosmeticGeometry;
use crate::modules::gifts::components::gift_cosmetic_preview::{
    GiftChanceFormat, GiftCosmeticPreview, GiftDropChanceText, GiftEquippedBadge, GiftRarityTierLabel,
};
use crate::modules::gifts::components::gift_info_table::{GiftInfoPerson, GiftInfoRow, GiftInfoTable};
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAttributeExt, RiseGiftAttributeKind, RiseGiftInstanceDTO, RiseGiftInstanceExt, RiseGiftItemKind,
    RiseGiftUserDTO,
};
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;
use crate::modules::gifts::shared::gift_people::GiftPeople;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::{
    GiftRecipient, GiftsInteractionsStore,
};
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;
use crate::modules::media_picker::ui::media_picker_header_icon_button::MediaPickerHeaderIconButton;
use crate::modules::sparks::components::sparks_icon_view::SparksAmountLabel;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn page_id(id: &str, name: &'static str) -> ElementId {
    ElementId::Name(format!("{name}-{id}").into())
}

/// states of the card plus what the title preference key is worked out from
#[derive(Default)]
struct GiftInstanceSheetContentState {
    owner: Option<GiftRecipient>,
    is_owner_menu_presented: bool,
    scroll_top: f32,
    title_top: Option<f32>,
}

impl GiftInstanceSheetContentState {
    /// whether the gift title went under the header, which then shows it on its own chrome
    fn title_under_header(&self) -> bool {
        self.title_top.is_some_and(|top| top - self.scroll_top < SheetFloatingHeaderMetrics::HEIGHT - 8.0)
    }
}

#[derive(IntoElement)]
pub struct GiftInstanceSheetContent {
    id: ElementId,
    pub instance: Option<RiseGiftInstanceDTO>,
    pub is_current: bool,
}

struct TableRow {
    view: Box<dyn FnOnce(bool) -> AnyElement>,
}

impl TableRow {
    fn new(view: impl FnOnce(bool) -> AnyElement + 'static) -> Self {
        Self { view: Box::new(view) }
    }
}

thread_local! {
    static DATE_FORMATTER: RefCell<Option<(String, DateFormatter)>> = const { RefCell::new(None) };
}

impl GiftInstanceSheetContent {
    const ART_SIDE: f32 = 220.0;

    pub fn new(id: impl Into<ElementId>, instance: Option<RiseGiftInstanceDTO>, is_current: bool) -> Self {
        Self { id: id.into(), instance, is_current }
    }

    fn hero(
        &self,
        instance: &RiseGiftInstanceDTO,
        state: &Entity<GiftInstanceSheetContentState>,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        let colors = GiftBackdropColors::resolve(instance.backdrop(), instance.rarity_class());
        let on_art = colors.is_rolled;

        let title_state = state.clone();
        let title = div()
            .relative()
            .w_full()
            .child(self.title(instance, on_art, theme))
            .child(measure(child_id(&self.id, "title-frame"), move |bounds, _, cx| {
                let top = f32::from(bounds.top());
                title_state.update(cx, |state, cx| {
                    if state.title_top != Some(top) {
                        state.title_top = Some(top);
                        cx.notify();
                    }
                });
            }));

        let mut column = vstack(6.0)
            .w_full()
            .pb(px(if on_art { 22.0 } else { 4.0 }))
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .pt(px(SheetFloatingHeaderMetrics::HEIGHT - 10.0))
                    .child(self.art(instance, &colors, state.read(cx).owner.clone())),
            )
            .child(title);
        if let Some(subtitle) = Self::subtitle(instance) {
            column = column.child(
                div()
                    .swiftui_font(Font::system_weight(15.0, FontWeight::NORMAL))
                    .text_color(if on_art { white().opacity(0.75) } else { theme.text.secondary })
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(SharedString::from(subtitle)),
            );
        }
        let viewer_is_owner = AuthServicesStore::shared(cx).read(cx).is_current_user(&instance.owner_user_id);
        if instance.is_cosmetic() && instance.is_equipped() && viewer_is_owner {
            column = column.child(div().pt(px(4.0)).child(GiftEquippedBadge::new()));
        }

        let mut hero = div().relative().w_full().overflow_hidden();
        if on_art {
            hero = hero.child(
                div().absolute().inset_0().child(
                    RadialGradient::new([colors.center, colors.edge], UnitPoint::new(0.5, 0.4), 0.0, 330.0)
                        .view()
                        .size_full(),
                ),
            );
        }
        hero = hero.child(column);
        if on_art {
            hero = hero.child(
                pass_through().absolute().top_0().left_0().right_0().h(px(96.0)).child(
                    LinearGradient::colors(
                        [black().opacity(0.3), black().opacity(0.0)],
                        UnitPoint::TOP,
                        UnitPoint::BOTTOM,
                    )
                    .view()
                    .size_full(),
                ),
            );
        }
        hero.into_any_element()
    }

    /// the model with its frame and effect, the pattern spreads beyond it over the backdrop
    /// a cosmetic shows itself on its owner instead
    fn art(
        &self,
        instance: &RiseGiftInstanceDTO,
        colors: &GiftBackdropColors,
        owner: Option<GiftRecipient>,
    ) -> AnyElement {
        let symbol = instance.pattern().map(|pattern| pattern.image_url.clone()).filter(|symbol| !symbol.is_empty());
        let pattern_side = Self::ART_SIDE * 1.75;
        let pattern = symbol.map(|symbol| {
            let mut layer = GiftPatternLayer::new(child_id(&self.id, "pattern"), symbol, colors.pattern);
            if instance.is_cosmetic() {
                layer = layer.strength(GiftPatternLayer::COSMETIC_STRENGTH);
            }
            pass_through()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(pass_through().flex_none().size(px(pattern_side)).child(layer))
        });

        let content = if instance.is_cosmetic() {
            self.cosmetic_art(instance, owner)
        } else {
            let mut stack = zstack(Alignment::Center).size(px(Self::ART_SIDE));
            if let Some(picture) = instance.frame().map(|frame| frame.image_url.clone()).filter(|url| !url.is_empty()) {
                stack = stack.child(
                    CleverImageUi::new(picture)
                        .id(child_id(&self.id, "frame"))
                        .content_mode(ContentMode::Fit)
                        .corner_radius(0.0)
                        .persist_to_disk(true)
                        .shows_progress_indicator(false)
                        .maximum_decode_pixel_dimension(1_024_i64)
                        .shows_failure_icon(false)
                        .placeholder_color(transparent_black()),
                );
            }
            stack = stack.child(
                div().size_full().p(px(8.0)).child(
                    GiftAnimationView::new(
                        child_id(&self.id, "model"),
                        instance.model().and_then(|model| model.media.clone()),
                    )
                    .large(true)
                    .animated(true)
                    .placeholder(true),
                ),
            );
            if let Some(effect) = instance.effect().and_then(|effect| effect.media.clone()) {
                stack = stack.child(
                    pass_through().size_full().p(px(8.0)).child(
                        GiftAnimationView::new(child_id(&self.id, "effect"), Some(effect)).large(true).animated(true),
                    ),
                );
            }
            stack.into_any_element()
        };

        let wide = matches!(instance.kind(), RiseGiftItemKind::Nameplate | RiseGiftItemKind::ProfileEffect);
        let mut art = div().relative().flex().justify_center();
        if wide && instance.is_cosmetic() {
            art = art.w_full();
        }
        art.children(pattern).child(content).into_any_element()
    }

    fn cosmetic_art(&self, instance: &RiseGiftInstanceDTO, owner: Option<GiftRecipient>) -> AnyElement {
        let preview = GiftCosmeticPreview::new(
            child_id(&self.id, "preview"),
            instance.kind(),
            owner,
            instance.model().and_then(|model| model.media.clone()),
        )
        .effect(instance.effect().and_then(|effect| effect.media.clone()))
        .animated(true);
        match instance.kind() {
            RiseGiftItemKind::Nameplate => div()
                .w_full()
                .px(px(16.0))
                .child(div().w_full().h(px(96.0)).child(preview))
                .into_any_element(),
            RiseGiftItemKind::ProfileEffect => div()
                .w_full()
                .px(px(24.0))
                .child(div().w_full().aspect_ratio(GiftCosmeticGeometry::BANNER_ASPECT).child(preview))
                .into_any_element(),
            RiseGiftItemKind::Item | RiseGiftItemKind::AvatarFrame => {
                div().size(px(Self::ART_SIDE)).child(preview).into_any_element()
            }
        }
    }

    fn title(&self, instance: &RiseGiftInstanceDTO, on_art: bool, theme: &AppTheme) -> AnyElement {
        let ink = if on_art { white() } else { theme.text.primary };
        let mut text = instance.name.clone();
        let mut highlights = Vec::new();
        if instance.is_numbered() {
            let start = text.len();
            text.push_str(&format!(" #{}", instance.serial));
            highlights.push((
                start..text.len(),
                HighlightStyle { color: Some(ink.opacity(0.62)), ..Default::default() },
            ));
        }
        div()
            .w_full()
            .px(px(24.0))
            .text_center()
            .swiftui_font(Font::system_weight(24.0, FontWeight::BOLD))
            .text_color(ink)
            .child(StyledText::new(SharedString::from(text)).with_highlights(highlights))
            .into_any_element()
    }

    /// the model name when the gift has models to choose from
    fn subtitle(instance: &RiseGiftInstanceDTO) -> Option<String> {
        let model = instance.model()?;
        (model.share() < 1.0 && !model.name.is_empty()).then(|| model.name.clone())
    }

    fn table(&self, instance: &RiseGiftInstanceDTO, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let rows = self.rows(instance, theme, cx);
        let views = rows.into_iter().enumerate().map(|(index, row)| (row.view)(index > 0));
        GiftInfoTable::new().children(views).into_any_element()
    }

    fn gift_back_button(
        id: ElementId,
        interactions: Entity<GiftsInteractionsStore>,
        to: RiseGiftUserDTO,
        compact: bool,
        primary: Hsla,
    ) -> AnyElement {
        let label: AnyElement = if compact {
            IconUi::sized("gift.fill", px(13.0), primary, false)
                .map_or_else(|| div().size(px(13.0)).into_any_element(), IntoElement::into_any_element)
        } else {
            div()
                .swiftui_font(Font::system_weight(12.0, FontWeight::SEMIBOLD))
                .whitespace_nowrap()
                .child(tr("gift_info_gift_back"))
                .into_any_element()
        };
        let tooltip_id = ElementId::NamedChild(Arc::new(id.clone()), "tooltip".into());
        div()
            .id(tooltip_id)
            .flex_none()
            .tooltip_label(tr("gift_info_gift_back"))
            .child(
                plain(id)
                    .text_color(primary)
                    .px(px(9.0))
                    .py(px(5.0))
                    .capsule()
                    .bg(primary.opacity(0.15))
                    .on_press(move |_, cx| {
                        interactions.update(cx, |interactions, cx| interactions.gift_back(&to, cx));
                    })
                    .child(label),
            )
            .into_any_element()
    }

    fn rows(&self, instance: &RiseGiftInstanceDTO, theme: &AppTheme, cx: &mut App) -> Vec<TableRow> {
        let auth = AuthServicesStore::shared(cx);
        let interactions = GiftsInteractionsStore::shared(cx);
        let services = GiftsServicesStore::shared(cx);
        let navigation = env::<NavigationStoreKey>(cx);
        let (text_primary, text_secondary, primary) = (theme.text.primary, theme.text.secondary, theme.primary._100);
        let viewer_is_owner = auth.read(cx).is_current_user(&instance.owner_user_id);
        let mut rows: Vec<TableRow> = Vec::new();

        if instance.is_grant() {
            rows.push(TableRow::new(|rule| {
                GiftInfoRow::new(
                    Some(tr("gift_info_from")),
                    div().font_weight(FontWeight::MEDIUM).child(SharedString::new_static("Riseonly")),
                )
                .rule_above(rule)
                .into_any_element()
            }));
        } else if let Some(sender) = instance.sender.clone() {
            let own = sender.id == instance.owner_user_id;
            let offers_gift_back = !own && viewer_is_owner && !auth.read(cx).is_current_user(&sender.id);
            let hidden_from_guests = !own && instance.anonymous;
            let id = self.id.clone();
            let interactions = interactions.clone();
            let navigation = navigation.clone();
            rows.push(TableRow::new(move |rule| {
                let open_sender = sender.clone();
                let open_interactions = interactions.clone();
                let person = GiftInfoPerson::new(sender.id.clone(), sender.name.clone(), Some(sender.avatar.clone()))
                    .on_open(move |_, cx| {
                        Self::open_profile(&open_sender, &open_interactions, navigation.as_ref(), cx);
                    });
                let mut value = vstack(3.0).align(HorizontalAlignment::Leading).w_full();
                if offers_gift_back {
                    let person = person.fills_width();
                    let button_id = child_id(&id, "gift-back");
                    let button_interactions = interactions.clone();
                    let to = sender.clone();
                    value = value.child(GiftGiverWithGiftBack::new(
                        child_id(&id, "giver"),
                        sender.name.clone(),
                        person,
                        move |compact| {
                            Self::gift_back_button(button_id, button_interactions, to, compact, primary)
                        },
                    ));
                } else {
                    value = value.child(person);
                }
                if hidden_from_guests {
                    value = value.child(
                        div()
                            .swiftui_font(Font::system_weight(13.0, FontWeight::NORMAL))
                            .text_color(text_secondary)
                            .child(tr("gift_anonymous_badge")),
                    );
                }
                let label = if own { SharedString::new_static("Riseonly") } else { tr("gift_info_from").into() };
                GiftInfoRow::new(Some(label), value).rule_above(rule).into_any_element()
            }));
        } else {
            rows.push(TableRow::new(move |rule| {
                GiftInfoRow::new(
                    Some(tr("gift_info_from")),
                    div().text_color(text_secondary).child(tr("gift_from_anonymous")),
                )
                .rule_above(rule)
                .into_any_element()
            }));
        }

        if instance.created_at_ms > 0 {
            let date = Self::date(instance.created_at_ms);
            rows.push(TableRow::new(move |rule| {
                GiftInfoRow::new(Some(tr("gift_info_date")), div().child(date)).rule_above(rule).into_any_element()
            }));
        }

        let rolled: Vec<_> = instance.attributes.iter().filter(|attribute| attribute.attribute_kind().is_some()).collect();
        let shows_model = rolled.len() > 1 || instance.model().map_or(1.0, |model| model.share()) < 1.0;
        for kind in instance.kind().attribute_kinds().iter().copied() {
            let Some(attribute) = rolled.iter().find(|attribute| attribute.attribute_kind() == Some(kind)) else {
                continue;
            };
            if kind == RiseGiftAttributeKind::Model && !shows_model {
                continue;
            }
            let row_id = ElementId::NamedChild(Arc::new(self.id.clone()), format!("kind-{}", kind.slug()).into());
            let name = attribute.display_name();
            let tier = attribute.tier();
            let permille = attribute.weight_permille;
            let gift_id = instance.gift_id;
            let interactions = interactions.clone();
            rows.push(TableRow::new(move |rule| {
                let mut title = hstack(6.0).w_full().child(
                    div().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(name),
                );
                if let Some(tier) = tier {
                    title = title.child(div().flex_none().child(GiftRarityTierLabel::new(tier)));
                }
                title = title.child(spacer_min_w(0.0));
                let mut value = vstack(3.0).align(HorizontalAlignment::Leading).w_full().child(title);
                if permille > 0 {
                    value = value.child(GiftDropChanceText::new(permille).tier(tier));
                }
                plain(row_id)
                    .w_full()
                    .on_press(move |_, cx| {
                        interactions.update(cx, |interactions, cx| interactions.open_odds(gift_id, kind, cx));
                    })
                    .child(GiftInfoRow::new(Some(tr(&format!("gift_kind_{}", kind.slug()))), value).rule_above(rule))
                    .into_any_element()
            }));
        }

        if let Some(share) = GiftChanceFormat::share(instance).filter(|share| *share < 1.0) {
            let rarity = instance.rarity_class();
            let highlighted = GiftChanceFormat::highlights(Some(rarity));
            rows.push(TableRow::new(move |rule| {
                let value = hstack(8.0)
                    .w_full()
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(16.0, FontWeight::SEMIBOLD).monospaced_digit())
                            .text_color(if highlighted { GiftRarityStyle::tint(rarity) } else { text_primary })
                            .whitespace_nowrap()
                            .child(GiftChanceFormat::percent_share(share)),
                    )
                    .child(GiftRarityPill::new(rarity))
                    .child(spacer_min_w(0.0));
                GiftInfoRow::new(Some(tr("gift_info_chance")), value).rule_above(rule).into_any_element()
            }));
        }

        if instance.is_numbered() {
            let gift = services
                .read(cx)
                .catalog_details(cx)
                .get(&instance.gift_id)
                .and_then(|details| details.gift.as_ref())
                .filter(|gift| gift.supply_total > 0)
                .map(|gift| (gift.supply_total, gift.supply_left));
            if let Some((total, left)) = gift {
                let issued = (total - left).max(0);
                let text = format_printf(
                    &tr("gift_info_issued"),
                    &[FormatArg::Text(Self::grouped(issued).into()), FormatArg::Text(Self::grouped(total).into())],
                );
                rows.push(TableRow::new(move |rule| {
                    GiftInfoRow::new(
                        Some(tr("gift_info_availability")),
                        div()
                            .w_full()
                            .swiftui_font(Font::system_weight(16.0, FontWeight::NORMAL).monospaced_digit())
                            .child(MinimumScaleText::new(text, 0.8).line_limit(1)),
                    )
                    .rule_above(rule)
                    .into_any_element()
                }));
            }
        }

        if instance.value_sparks > 0 {
            let value_sparks = instance.value_sparks;
            rows.push(TableRow::new(move |rule| {
                GiftInfoRow::new(
                    Some(tr("gift_info_value")),
                    SparksAmountLabel::new(value_sparks).px(16.0).icon_size(17.0).color(text_primary),
                )
                .rule_above(rule)
                .into_any_element()
            }));
        }

        let caption = trimming_whitespaces_and_newlines(&instance.caption).to_owned();
        if !caption.is_empty() {
            rows.push(TableRow::new(move |rule| {
                GiftInfoRow::new(None::<SharedString>, div().w_full().child(caption))
                    .rule_above(rule)
                    .into_any_element()
            }));
        }
        rows
    }

    fn owner_menu(&self, state: &Entity<GiftInstanceSheetContentState>, cx: &mut App) -> Option<AnyElement> {
        let instance = self.instance.clone()?;
        let auth = AuthServicesStore::shared(cx);
        if !auth.read(cx).is_current_user(&instance.owner_user_id) {
            return None;
        }
        let opener = state.clone();
        let is_presented = Binding::entity(
            state.clone(),
            |state: &GiftInstanceSheetContentState| state.is_owner_menu_presented,
            |state: &mut GiftInstanceSheetContentState, is_presented: bool, cx: &mut Context<GiftInstanceSheetContentState>| {
                state.is_owner_menu_presented = is_presented;
                cx.notify();
            },
        );
        let listed = instance.clone();
        let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| Self::owner_menu_actions(&listed));
        let acted = instance;
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::perform_owner_menu_action(&id, acted.clone(), cx);
        });
        let source_id = child_id(&self.id, "menu-source");
        let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| {
            MediaPickerHeaderIconButton::new(source_id.clone(), "ellipsis", |_, _| {}).font_size(17.0).into_any_element()
        });
        let button = MediaPickerHeaderIconButton::new(child_id(&self.id, "menu-button"), "ellipsis", move |_, cx| {
            opener.update(cx, |state, cx| {
                state.is_owner_menu_presented = true;
                cx.notify();
            });
        })
        .font_size(17.0);
        Some(
            div()
                .id(child_id(&self.id, "menu-label"))
                .tooltip_label(tr("share_action_more"))
                .child(button)
                .tap_context_menu(
                    child_id(&self.id, "menu"),
                    is_presented,
                    HoldContextMenuAlignment::Trailing,
                    false,
                    false,
                    false,
                    None,
                    actions,
                    on_action,
                    source,
                )
                .into_any_element(),
        )
    }

    fn owner_menu_actions(instance: &RiseGiftInstanceDTO) -> Vec<HoldContextMenuAction> {
        let mut actions = Vec::new();
        if instance.is_cosmetic() {
            actions.push(
                HoldContextMenuAction::new(
                    "toggle_equipped",
                    tr(if instance.is_equipped() { "gift_equip_take_off" } else { "gift_equip_put_on" }),
                )
                .system_image(if instance.is_equipped() { "xmark.circle" } else { "checkmark.circle" }),
            );
        }
        actions.push(
            HoldContextMenuAction::new("toggle_pinned", tr(if instance.is_pinned() { "gift_unpin" } else { "gift_pin" }))
                .system_image(if instance.is_pinned() { "pin.slash" } else { "pin" }),
        );
        actions.push(
            HoldContextMenuAction::new(
                "toggle_visibility",
                tr(if instance.is_displayed() { "gift_hide" } else { "gift_show" }),
            )
            .system_image(if instance.is_displayed() { "eye.slash" } else { "eye" }),
        );
        actions
    }

    fn perform_owner_menu_action(id: &str, instance: RiseGiftInstanceDTO, cx: &mut App) {
        let interactions = GiftsInteractionsStore::shared(cx);
        match id {
            "toggle_equipped" => interactions.update(cx, |store, cx| store.toggle_equipped(instance, cx)),
            "toggle_pinned" => interactions.update(cx, |store, cx| store.toggle_pinned_instance(instance, cx)),
            "toggle_visibility" => {
                interactions.update(cx, |store, cx| store.request_visibility_change_instance(instance, cx))
            }
            _ => {}
        }
    }

    fn open_profile(
        person: &RiseGiftUserDTO,
        interactions: &Entity<GiftsInteractionsStore>,
        navigation: Option<&Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        interactions.update(cx, |interactions, cx| interactions.close_instance(cx));
        let Some(navigation) = navigation else {
            return;
        };
        let tag = (!person.tag.is_empty()).then(|| person.tag.clone());
        let route = RootRoute::UserPage { user_id: Some(person.id.clone()), tag };
        navigation.update(cx, |navigation, cx| navigation.push(route, NavigationStoreSettlingPush::Drop, cx));
    }

    fn date(milliseconds: i64) -> String {
        let code = I18n::shared().code().to_owned();
        let date = Date::from_time_interval_since_1970(milliseconds as f64 / 1_000.0);
        DATE_FORMATTER.with_borrow_mut(|slot| {
            if slot.as_ref().is_none_or(|(locale, _)| *locale != code) {
                let mut formatter = DateFormatter::new();
                formatter.set_date_style(Style::Short);
                formatter.set_time_style(Style::Short);
                *slot = Some((code, formatter));
            }
            slot.as_ref().map(|(_, formatter)| formatter.string_from(&date)).unwrap_or_default()
        })
    }

    /// exact count with the language grouping, for the edition row where theres room
    fn grouped(value: i64) -> String {
        DecimalNumberFormatter::new(I18n::shared().code()).string(value.max(0) as u64)
    }
}

impl RenderOnce for GiftInstanceSheetContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let theme = theme_store.read(cx).current_theme().clone();
        let interactions = GiftsInteractionsStore::shared(cx);
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| {
            GiftInstanceSheetContentState::default()
        });

        let gift_id = self.instance.as_ref().map(|instance| instance.gift_id);
        use_task(child_id(&self.id, "catalog-task"), gift_id, window, cx, async move |cx| {
            let Some(gift_id) = gift_id else {
                return;
            };
            let _ = GiftsActionsStore::load_catalog_item(gift_id, false, cx).await;
        });

        let owner_id = self.instance.as_ref().map(|instance| instance.owner_user_id.clone());
        let owner_state = state.clone();
        let task_owner_id = owner_id.clone();
        use_task(child_id(&self.id, "owner-task"), owner_id, window, cx, async move |cx| {
            let _ = owner_state.update(cx, |state, cx| {
                state.owner = task_owner_id.as_deref().and_then(|id| GiftPeople::person(id, cx));
                cx.notify();
            });
        });

        let confirming = self.is_current && interactions.read(cx).confirming_hide;
        let alert_store = interactions.clone();
        on_change(child_id(&self.id, "hide-alert"), confirming, false, window, cx, move |_, confirming, _, cx| {
            if !*confirming {
                return;
            }
            let cancel_store = alert_store.clone();
            let confirm_store = alert_store;
            NativeConfirmAlert::present_delete(
                &tr("gift_hide_confirm_title"),
                &tr("gift_hide_confirm_message"),
                Some(tr("gift_hide_action")),
                None,
                false,
                None,
                Some(Box::new(move |cx: &mut App| {
                    cancel_store.update(cx, |store, cx| {
                        store.confirming_hide = false;
                        cx.notify();
                    });
                })),
                move |cx| {
                    confirm_store.update(cx, |store, cx| {
                        store.confirming_hide = false;
                        store.toggle_displayed(cx);
                    });
                },
                cx,
            );
        });

        let mut column = vstack(0.0).w_full().pb(px(16.0));
        if let Some(instance) = self.instance.as_ref() {
            column = column.child(self.hero(instance, &state, &theme, cx)).child(
                div().w_full().px(px(16.0)).pt(px(16.0)).child(self.table(instance, &theme, cx)),
            );
        } else {
            column = column.child(div().w_full().h(px(260.0)));
        }

        let title_under_header = state.read(cx).title_under_header();
        let title = self.instance.as_ref().map(|instance| instance.name.clone()).unwrap_or_default();
        let closer = interactions;
        let mut header = SheetFloatingHeader::new(child_id(&self.id, "header"), title, move |_, cx| {
            closer.update(cx, |interactions, cx| interactions.close_instance(cx));
        })
        .shows_title(title_under_header)
        .shows_chrome(title_under_header);
        if let Some(menu) = self.owner_menu(&state, cx) {
            header = header.trailing(menu);
        }

        let frame_state = state;
        div()
            .relative()
            .size_full()
            .child(scroll_view(child_id(&self.id, "scroll")).shows_indicators(false).child(column))
            .child(div().absolute().top_0().left_0().right_0().child(header))
            .child(measure(child_id(&self.id, "scroll-frame"), move |bounds, _, cx| {
                let top = f32::from(bounds.top());
                frame_state.update(cx, |state, cx| {
                    if state.scroll_top != top {
                        state.scroll_top = top;
                        cx.notify();
                    }
                });
            }))
    }
}

#[derive(IntoElement)]
struct GiftGiverWithGiftBack {
    id: ElementId,
    name: String,
    person: AnyElement,
    gift_back: Box<dyn FnOnce(bool) -> AnyElement>,
}

impl GiftGiverWithGiftBack {
    const CAPTION_PADDING: f32 = 18.0;
    const MEASURE_SLACK: f32 = 2.0;
    const CAPTION_FONT_SIZE: f32 = 12.0;

    fn new(
        id: ElementId,
        name: String,
        person: impl IntoElement,
        gift_back: impl FnOnce(bool) -> AnyElement + 'static,
    ) -> Self {
        Self { id, name, person: person.into_any_element(), gift_back: Box::new(gift_back) }
    }

    fn text_width(text: &str, size: f32, face: TextFont, color: Hsla, window: &Window) -> f32 {
        let runs = [TextRun {
            len: text.len(),
            font: face,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        f32::from(window.text_system().shape_line(SharedString::from(text.to_owned()), px(size), &runs, None).width())
            .ceil()
    }

    /// the avatar, the gap and the whole name as the person lays them out
    fn person_width(name: &str, color: Hsla, window: &Window) -> f32 {
        let name_width =
            Self::text_width(name, GiftInfoPerson::NAME_FONT_SIZE, GiftInfoPerson::name_ui_font(), color, window);
        GiftInfoPerson::AVATAR_SIDE + GiftInfoPerson::SPACING + name_width + Self::MEASURE_SLACK
    }

    fn compact(person_width: f32, width: f32, color: Hsla, window: &Window) -> bool {
        if width <= 0.0 {
            return false;
        }
        let caption_font = TextFont { weight: FontWeight::SEMIBOLD, ..font(FONT_FAMILY) };
        let caption_width =
            Self::text_width(&tr("gift_info_gift_back"), Self::CAPTION_FONT_SIZE, caption_font, color, window);
        let needed = person_width + GiftInfoPerson::SPACING + caption_width + Self::CAPTION_PADDING;
        needed > width
    }
}

impl RenderOnce for GiftGiverWithGiftBack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let width = window.use_keyed_state(child_id(&self.id, "width"), cx, |_, _| 0.0_f32);
        let color = ThemeStore::shared(cx).read(cx).current_theme().text.primary;
        let person_width = Self::person_width(&self.name, color, window);
        let compact = Self::compact(person_width, *width.read(cx), color, window);
        hstack(0.0)
            .relative()
            .w_full()
            .child(div().min_w_0().flex_grow(1.0).flex_shrink(1.0).flex_basis(px(person_width)).child(self.person))
            .child(div().flex_none().w(px(GiftInfoPerson::SPACING)))
            .child((self.gift_back)(compact))
            .child(measure(child_id(&self.id, "frame"), move |bounds, _, cx| {
                let value = f32::from(bounds.size.width);
                width.update(cx, |width, cx| {
                    if (value - *width).abs() > 0.5 {
                        *width = value;
                        cx.notify();
                    }
                });
            }))
    }
}

#[derive(IntoElement)]
pub struct GiftInstanceSheetFooter {
    id: ElementId,
    pub instance: Option<RiseGiftInstanceDTO>,
}

impl GiftInstanceSheetFooter {
    pub fn new(id: impl Into<ElementId>, instance: Option<RiseGiftInstanceDTO>) -> Self {
        Self { id: id.into(), instance }
    }

    fn visibility_note(
        &self,
        instance: &RiseGiftInstanceDTO,
        interactions: &Entity<GiftsInteractionsStore>,
        theme: &AppTheme,
        is_changing: bool,
    ) -> AnyElement {
        let displayed = instance.is_displayed();
        let action = hstack(2.0)
            .text_color(theme.primary._100)
            .whitespace_nowrap()
            .child(tr(if displayed { "gift_hide_action" } else { "gift_show_action" }))
            .children(IconUi::sized("chevron.right", px(11.0), theme.primary._100, false));
        let store = interactions.clone();
        let target = instance.clone();
        let button = plain(child_id(&self.id, "visibility"))
            .flex_none()
            .disabled(is_changing)
            .on_press(move |_, cx| {
                let target = target.clone();
                store.update(cx, |store, cx| store.request_visibility_change_instance(target, cx));
            })
            .child(action);
        hstack(4.0)
            .text_size(px(14.0))
            .child(
                div()
                    .min_w_0()
                    .text_color(theme.text.secondary)
                    .child(MinimumScaleText::new(tr(if displayed { "gift_visible_note" } else { "gift_hidden_note" }), 0.85).line_limit(1)),
            )
            .child(button)
            .into_any_element()
    }
}

impl RenderOnce for GiftInstanceSheetFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let theme = theme_store.read(cx).current_theme().clone();
        let interactions = GiftsInteractionsStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let (is_changing, is_equipping) = {
            let store = interactions.read(cx);
            (store.is_changing_instance, store.is_equipping)
        };
        let owned = self.instance.clone().filter(|instance| auth.read(cx).is_current_user(&instance.owner_user_id));

        let mut column = vstack(12.0).w_full().px(px(16.0)).py(px(10.0));
        if let Some(instance) = owned.as_ref() {
            column = column.child(self.visibility_note(instance, &interactions, &theme, is_changing));
        }
        if let Some(instance) = owned.filter(|instance| instance.is_cosmetic()) {
            let offers = !instance.is_equipped() && instance.is_gifted_by_other();
            if offers && let Some(key) = instance.kind().gifted_offer_key() {
                column = column.child(
                    div()
                        .w_full()
                        .text_center()
                        .swiftui_font(Font::system_weight(15.0, FontWeight::MEDIUM))
                        .text_color(theme.text.primary)
                        .child(format_printf(&tr(key), &[FormatArg::Text(instance.name.clone().into())])),
                );
            }
            let closer = interactions.clone();
            let toggler = interactions;
            let equipped = instance.is_equipped();
            column = column.child(
                hstack(10.0)
                    .w_full()
                    .child(
                        div().flex_1().min_w_0().child(
                            ButtonUi::new(child_id(&self.id, "close"))
                                .background_color(theme.bg._200)
                                .on_press(move |_, cx| {
                                    closer.update(cx, |store, cx| store.close_instance(cx));
                                })
                                .child(
                                    MainText::new(tr(if offers { "gift_equip_later" } else { "gift_info_ok" }))
                                        .px(16.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.text.primary),
                                ),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            ButtonUi::new(child_id(&self.id, "equip"))
                                .is_pending(is_equipping)
                                .on_press(move |_, cx| {
                                    let target = instance.clone();
                                    toggler.update(cx, |store, cx| store.toggle_equipped(target, cx));
                                })
                                .child(
                                    MainText::new(tr(if equipped { "gift_equip_take_off" } else { "gift_equip_put_on" }))
                                        .px(16.0)
                                        .font_weight(FontWeight::SEMIBOLD),
                                ),
                        ),
                    ),
            );
        } else {
            let closer = interactions;
            column = column.child(
                ButtonUi::new(child_id(&self.id, "ok"))
                    .on_press(move |_, cx| {
                        closer.update(cx, |store, cx| store.close_instance(cx));
                    })
                    .child(MainText::new(tr("gift_info_ok")).px(16.0).font_weight(FontWeight::SEMIBOLD)),
            );
        }
        column
    }
}

#[derive(Default)]
struct GiftInstancePageLast(Option<RiseGiftInstanceDTO>);

/// the gift with this id as the stores have it now or the last one this page drew
/// while the carousel closes the stores let go of it and the page keeps it so the sheet
/// slides away with its gift
fn gift_instance_page_instance(id: &str, name: &'static str, window: &mut Window, cx: &mut App) -> Option<RiseGiftInstanceDTO> {
    let interactions = GiftsInteractionsStore::shared(cx);
    let fresh = interactions.update(cx, |store, cx| store.page_instance(id, cx));
    let last = window.use_keyed_state(page_id(id, name), cx, |_, _| GiftInstancePageLast::default());
    if let Some(value) = fresh {
        last.update(cx, |last, _| last.0 = Some(value.clone()));
        return Some(value);
    }
    last.read(cx).0.clone()
}

/// one page of the gift carousel, the gift with this id as the stores have it now
/// the page keeps the last one it drew while the carousel closes so the sheet slides away
/// with its gift
#[derive(IntoElement)]
pub struct GiftInstancePage {
    pub id: String,
}

impl GiftInstancePage {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for GiftInstancePage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let instance = gift_instance_page_instance(&self.id, "gift-instance-page-last", window, cx);
        let interactions = GiftsInteractionsStore::shared(cx);
        let is_current =
            interactions.read(cx).opened_instance.as_ref().is_some_and(|opened| opened.id == self.id);
        GiftInstanceSheetContent::new(page_id(&self.id, "gift-instance-content"), instance, is_current)
    }
}

/// footer of one carousel page, holding on to its gift the same way
#[derive(IntoElement)]
pub struct GiftInstancePageFooter {
    pub id: String,
}

impl GiftInstancePageFooter {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for GiftInstancePageFooter {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let instance = gift_instance_page_instance(&self.id, "gift-instance-footer-last", window, cx);
        GiftInstanceSheetFooter::new(page_id(&self.id, "gift-instance-footer"), instance)
    }
}
