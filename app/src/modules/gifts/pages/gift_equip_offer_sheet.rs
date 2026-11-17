use std::sync::Arc;

use gpui::{App, ElementId, FontWeight, SharedString, Window};
use rise_i18n::{FormatArg, format_printf, tr};
use rise_theme::ThemeStore;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::ui::bottom_sheet_ui::sheet_floating_header::{SheetFloatingHeader, SheetFloatingHeaderMetrics};
use crate::modules::gifts::components::gift_cosmetic_preview::GiftCosmeticStage;
use crate::modules::gifts::engine::rise_gifts_engine_models::RiseGiftInstanceExt;
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;
use crate::modules::gifts::shared::gift_people::GiftPeople;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::{
    GiftRecipient, GiftsInteractionsStore,
};
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;

const CONTENT_ID: &str = "gift-equip-offer-content";

fn child_id(name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(ElementId::Name(CONTENT_ID.into())), name.into())
}

/// a cosmetic somebody just gave the viewer isnt worn by itself
/// this compact sheet shows it on the viewer and asks to put it on
#[derive(IntoElement)]
pub struct GiftEquipOfferSheetContent;

impl GiftEquipOfferSheetContent {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for GiftEquipOfferSheetContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let interactions = GiftsInteractionsStore::shared(cx);
        let offer = interactions.read(cx).equip_offer.clone();
        let me = window.use_keyed_state(child_id("me"), cx, |_, _| None::<GiftRecipient>);

        let task_me = me.clone();
        use_task(child_id("me-task"), (), window, cx, async move |cx| {
            let _ = task_me.update(cx, |me, cx| {
                *me = GiftPeople::me(cx);
                cx.notify();
            });
        });

        let gift_id = offer.as_ref().map(|offer| offer.gift_id);
        let localizer = interactions.clone();
        use_task(child_id("localize-task"), gift_id, window, cx, async move |cx| {
            let Some(gift_id) = gift_id else {
                return;
            };
            let _ = localizer.update(cx, |interactions, cx| interactions.localize_chat_card_name(gift_id, cx));
        });

        let mut column = vstack(16.0)
            .w_full()
            .pb(px(16.0))
            .child(div().w_full().h(px(SheetFloatingHeaderMetrics::HEIGHT - 16.0)));
        if let Some(offer) = offer {
            column = column.child(
                div().w_full().px(px(16.0)).child(
                    GiftCosmeticStage::new(
                        child_id("stage"),
                        offer.kind(),
                        me.read(cx).clone(),
                        offer.model().and_then(|model| model.media.clone()),
                    )
                    .effect(offer.effect().and_then(|effect| effect.media.clone()))
                    .backdrop(offer.backdrop().cloned())
                    .pattern(offer.pattern().cloned())
                    .rarity(offer.rarity_class())
                    .animated(true),
                ),
            );
        }

        let closer = interactions;
        div()
            .relative()
            .size_full()
            .child(scroll_view(child_id("scroll")).shows_indicators(false).child(column))
            .child(
                div().absolute().top_0().left_0().right_0().child(
                    SheetFloatingHeader::new(child_id("header"), "", move |_, cx| {
                        closer.update(cx, |interactions, cx| interactions.decline_equip_offer(cx));
                    })
                    .shows_title(false),
                ),
            )
    }
}

/// later and put on buttons
#[derive(IntoElement)]
pub struct GiftEquipOfferSheetFooter;

impl GiftEquipOfferSheetFooter {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for GiftEquipOfferSheetFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme_store = ThemeStore::shared(cx);
        let theme = theme_store.read(cx).current_theme().clone();
        let interactions = GiftsInteractionsStore::shared(cx);
        let services = GiftsServicesStore::shared(cx);
        let offer = interactions.read(cx).equip_offer.clone();
        let is_equipping = interactions.read(cx).is_equipping;

        let mut column = vstack(12.0).w_full().px(px(16.0)).py(px(10.0));
        if let Some(offer) = offer
            && let Some(key) = offer.kind().gifted_offer_key()
        {
            let name = services.read(cx).localized_name(offer.gift_id, cx).unwrap_or_else(|| offer.name.clone());
            let mut lines = vstack(4.0).w_full().px(px(8.0)).child(
                div()
                    .w_full()
                    .text_center()
                    .swiftui_font(Font::system_weight(18.0, FontWeight::SEMIBOLD))
                    .text_color(theme.text.primary)
                    .child(SharedString::from(format_printf(&tr(key), &[FormatArg::Text(name.into())]))),
            );
            if let Some(sender) = offer.sender.as_ref()
                && sender.id != offer.owner_user_id
                && !sender.name.is_empty()
            {
                lines = lines.child(
                    SecondaryText::new(format_printf(&tr("gift_from"), &[FormatArg::Text(sender.name.clone().into())]))
                        .px(14.0)
                        .tac(TextAlignT::Center),
                );
            }
            column = column.child(lines);
        }

        let later = interactions.clone();
        let accept = interactions;
        column.child(
            hstack(10.0)
                .w_full()
                .child(
                    div().flex_1().min_w_0().child(
                        ButtonUi::new("gift-equip-offer-later")
                            .background_color(theme.bg._200)
                            .on_press(move |_, cx| {
                                later.update(cx, |interactions, cx| interactions.decline_equip_offer(cx));
                            })
                            .child(
                                MainText::new(tr("gift_equip_later"))
                                    .px(16.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.text.primary),
                            ),
                    ),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        ButtonUi::new("gift-equip-offer-put-on")
                            .is_pending(is_equipping)
                            .on_press(move |_, cx| {
                                accept.update(cx, |interactions, cx| interactions.accept_equip_offer(cx));
                            })
                            .child(MainText::new(tr("gift_equip_put_on")).px(16.0).font_weight(FontWeight::SEMIBOLD)),
                    ),
                ),
        )
    }
}
