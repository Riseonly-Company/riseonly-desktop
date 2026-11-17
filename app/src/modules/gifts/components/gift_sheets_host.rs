use gpui::{AnyView, App, Context, Empty, Entity, FocusHandle, KeyDownEvent, Render, StyleRefinement, Subscription, Window};
use rise_theme::ThemeStore;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_carousel_ui::BottomSheetCarouselUi;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::engine::rise_gifts_engine_models::RiseGiftInstanceExt;
use crate::modules::gifts::pages::gift_equip_offer_sheet::{GiftEquipOfferSheetContent, GiftEquipOfferSheetFooter};
use crate::modules::gifts::pages::gift_instance_sheet::{GiftInstancePage, GiftInstancePageFooter};
use crate::modules::gifts::pages::gift_odds_sheet::GiftOddsSheetContent;
use crate::modules::gifts::pages::gift_shop_sheet::{GiftShopSheetContentView, GiftShopSheetFooter};
use crate::modules::gifts::pages::gift_sparks_info_sheet::{
    GiftSparksInfoLayout, GiftSparksInfoSheetContent, GiftSparksInfoSheetFooter,
};
use crate::modules::gifts::pages::gift_wardrobe_sheet::{GiftWardrobeSheetContent, GiftWardrobeSheetFooter};
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftsInteractionsStore;
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;

const KEY_ESCAPE: &str = "escape";
const KEY_ENTER: &str = "enter";

/// the sheet the keyboard talks to, the last one of the host stack that is up
#[derive(Clone, Copy, PartialEq, Eq)]
enum GiftSheetsHostTopSheet {
    Shop,
    Wardrobe,
    Instance,
    Odds,
    EquipOffer,
    SparksInfo,
}

/// hosts the gift shop, gift card odds customization picker equip offer and about sparks
/// once at the app root
/// so a profile or chat or toast can open them without owning them
pub struct GiftSheetsHost {
    interactions: Entity<GiftsInteractionsStore>,
    theme_store: Entity<ThemeStore>,
    shop_content: Entity<GiftShopSheetContentView>,
    shop_content_stale: bool,
    sparks_info_fraction: f32,
    focus_handle: FocusHandle,
    focus_to_restore: Option<FocusHandle>,
    _observations: [Subscription; 5],
}

impl GiftSheetsHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let interactions = GiftsInteractionsStore::shared(cx);
        let services = GiftsServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let shop_content = cx.new(|_| GiftShopSheetContentView::new());
        let observations = [
            cx.observe(&interactions, |_, _, cx| cx.notify()),
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&theme_store, |_, _, cx| cx.notify()),
            cx.observe(&shop_content, |host, _, cx| {
                host.shop_content_stale = true;
                cx.notify();
            }),
        ];
        Self {
            interactions,
            theme_store,
            shop_content,
            shop_content_stale: false,
            sparks_info_fraction: GiftSparksInfoLayout::INITIAL_FRACTION,
            focus_handle: cx.focus_handle(),
            focus_to_restore: None,
            _observations: observations,
        }
    }

    fn adopt_sparks_info_height(&mut self, height: f32, button_height: f32, window: &Window, cx: &mut Context<Self>) {
        let target = GiftSparksInfoLayout::fraction(height, button_height, window);
        if (target - self.sparks_info_fraction).abs() <= 0.005 {
            return;
        }
        self.sparks_info_fraction = target;
        cx.notify();
    }

    fn top_sheet(&self, cx: &App) -> Option<GiftSheetsHostTopSheet> {
        let interactions = self.interactions.read(cx);
        if interactions.is_sparks_info_presented {
            return Some(GiftSheetsHostTopSheet::SparksInfo);
        }
        if interactions.equip_offer.is_some() {
            return Some(GiftSheetsHostTopSheet::EquipOffer);
        }
        if interactions.odds_request.is_some() {
            return Some(GiftSheetsHostTopSheet::Odds);
        }
        if interactions.opened_instance.is_some() {
            return Some(GiftSheetsHostTopSheet::Instance);
        }
        if interactions.wardrobe_kind.is_some() {
            return Some(GiftSheetsHostTopSheet::Wardrobe);
        }
        interactions.is_shop_presented().then_some(GiftSheetsHostTopSheet::Shop)
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(top) = self.top_sheet(cx) else {
            return;
        };
        let handled = match event.keystroke.key.as_str() {
            KEY_ESCAPE => {
                self.close(top, cx);
                true
            }
            KEY_ENTER
                if !event.is_held
                    && !event.keystroke.modifiers.modified()
                    && (self.focus_handle.is_focused(window) || !ScreenFocus::text_field_is_focused(window, cx)) =>
            {
                self.press_main_button(top, cx)
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn close(&self, top: GiftSheetsHostTopSheet, cx: &mut Context<Self>) {
        self.interactions.update(cx, |interactions, cx| match top {
            GiftSheetsHostTopSheet::Shop => interactions.close_shop(cx),
            GiftSheetsHostTopSheet::Wardrobe => interactions.close_wardrobe(cx),
            GiftSheetsHostTopSheet::Instance => interactions.close_instance(cx),
            GiftSheetsHostTopSheet::Odds => interactions.close_odds(cx),
            GiftSheetsHostTopSheet::EquipOffer => interactions.decline_equip_offer(cx),
            GiftSheetsHostTopSheet::SparksInfo => interactions.close_sparks_info(cx),
        });
    }

    fn press_main_button(&self, top: GiftSheetsHostTopSheet, cx: &mut Context<Self>) -> bool {
        let auth = AuthServicesStore::shared(cx);
        match top {
            GiftSheetsHostTopSheet::SparksInfo => {
                self.interactions.update(cx, |interactions, cx| interactions.close_sparks_info(cx));
                true
            }
            GiftSheetsHostTopSheet::EquipOffer => {
                self.interactions.update(cx, |interactions, cx| {
                    if !interactions.is_equipping {
                        interactions.accept_equip_offer(cx);
                    }
                });
                true
            }
            GiftSheetsHostTopSheet::Instance => {
                let Some(instance) = self.interactions.read(cx).opened_instance.clone() else {
                    return false;
                };
                let owns_cosmetic = instance.is_cosmetic() && auth.read(cx).is_current_user(&instance.owner_user_id);
                self.interactions.update(cx, |interactions, cx| {
                    if !owns_cosmetic {
                        interactions.close_instance(cx);
                    } else if !interactions.is_equipping {
                        interactions.toggle_equipped(instance, cx);
                    }
                });
                true
            }
            GiftSheetsHostTopSheet::Odds | GiftSheetsHostTopSheet::Wardrobe | GiftSheetsHostTopSheet::Shop => false,
        }
    }
}

impl Render for GiftSheetsHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme_store.read(cx).current_theme().clone();
        let sheet_background = Some(theme.bg._100);
        let footer_background = Some(theme.bg._100);
        let button_height = f32::from(theme.button.height_300);
        let interactions = self.interactions.clone();

        let any_presented = self.top_sheet(cx).is_some();
        let this = cx.weak_entity();
        on_change("gift-sheets-presented", any_presented, false, window, cx, move |_, presented, window, cx| {
            let presented = *presented;
            let _ = this.update(cx, |host, cx| {
                if presented {
                    host.focus_to_restore = window.focused(cx);
                    window.focus(&host.focus_handle, cx);
                } else if host.focus_handle.is_focused(window) || !ScreenFocus::text_field_is_focused(window, cx) {
                    match host.focus_to_restore.take() {
                        Some(previous) => window.focus(&previous, cx),
                        None => window.blur(),
                    }
                }
            });
        });

        let reader = interactions.clone();
        let closer = interactions.clone();
        let shop_content = self.shop_content.clone();
        let shop_content_stale = std::mem::take(&mut self.shop_content_stale);
        tracing::warn!(target: "riseonly::X2374", shop_stale = shop_content_stale, "gift sheets host render");
        let shop = BottomSheetUi::new(
            "gift-shop-sheet",
            Binding::new(
                move |cx| reader.read(cx).is_shop_presented(),
                move |presented, _, cx| {
                    if !presented {
                        closer.update(cx, |interactions, cx| interactions.close_shop(cx));
                    }
                },
            ),
            move |_, _| {
                if shop_content_stale {
                    AnyView::from(shop_content.clone()).into_any_element()
                } else {
                    shop_content.clone().cached(StyleRefinement::default().size_full()).into_any_element()
                }
            },
            |_, _| GiftShopSheetFooter::new(),
        )
        .snap_fractions(vec![0.93])
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(sheet_background)
        .footer_background(footer_background);

        let reader = interactions.clone();
        let closer = interactions.clone();
        let dismissed = interactions.clone();
        let wardrobe = BottomSheetUi::new(
            "gift-wardrobe-sheet",
            Binding::new(
                move |cx| reader.read(cx).wardrobe_kind.is_some(),
                move |presented, _, cx| {
                    if !presented {
                        closer.update(cx, |interactions, cx| interactions.close_wardrobe(cx));
                    }
                },
            ),
            |_, _| GiftWardrobeSheetContent::new(),
            |_, _| GiftWardrobeSheetFooter::new(),
        )
        .snap_fractions(vec![0.72, 0.93])
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(sheet_background)
        .footer_background(footer_background)
        .on_did_dismiss(move |_, cx| dismissed.update(cx, |interactions, cx| interactions.wardrobe_did_dismiss(cx)));

        let page_ids = interactions.update(cx, |interactions, cx| interactions.instance_page_ids(cx));
        let selection = interactions.read(cx).opened_instance.as_ref().map(|instance| instance.id.clone());
        let reader = interactions.clone();
        let closer = interactions.clone();
        let selector = interactions.clone();
        let approaching = interactions.clone();
        let dismissed = interactions.clone();
        let instance = BottomSheetCarouselUi::new(
            "gift-instance-sheet",
            Binding::new(
                move |cx| reader.read(cx).opened_instance.is_some(),
                move |presented, _, cx| {
                    if !presented {
                        closer.update(cx, |interactions, cx| interactions.close_instance(cx));
                    }
                },
            ),
            page_ids,
            selection,
            move |id: String, _, cx| selector.update(cx, |interactions, cx| interactions.show_instance(&id, cx)),
            |id: &String, _, _| GiftInstancePage::new(id.clone()),
            |id: &String, _, _| GiftInstancePageFooter::new(id.clone()),
        )
        .snap_fractions(|_| vec![0.86])
        .content_under_handle(true)
        .content_scrollable(true)
        .footer_shows_top_border(false)
        .sheet_background(sheet_background)
        .footer_background(footer_background)
        .on_approach_end(move |_, cx| approaching.update(cx, |interactions, cx| interactions.load_more_opened_list(cx)))
        .on_did_dismiss(move |_, cx| dismissed.update(cx, |interactions, cx| interactions.instance_did_dismiss(cx)));

        let reader = interactions.clone();
        let closer = interactions.clone();
        let odds = BottomSheetUi::new(
            "gift-odds-sheet",
            Binding::new(
                move |cx| reader.read(cx).odds_request.is_some(),
                move |presented, _, cx| {
                    if !presented {
                        closer.update(cx, |interactions, cx| interactions.close_odds(cx));
                    }
                },
            ),
            |_, _| GiftOddsSheetContent::new(),
            |_, _| Empty,
        )
        .snap_fractions(vec![0.72])
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(sheet_background)
        .footer_background(footer_background);

        let reader = interactions.clone();
        let closer = interactions.clone();
        let equip_offer = BottomSheetUi::new(
            "gift-equip-offer-sheet",
            Binding::new(
                move |cx| reader.read(cx).equip_offer.is_some(),
                move |presented, _, cx| {
                    if !presented {
                        closer.update(cx, |interactions, cx| interactions.decline_equip_offer(cx));
                    }
                },
            ),
            |_, _| GiftEquipOfferSheetContent::new(),
            |_, _| GiftEquipOfferSheetFooter::new(),
        )
        .snap_fractions(vec![0.62])
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(sheet_background)
        .footer_background(footer_background);

        let reader = interactions.clone();
        let closer = interactions;
        let this = cx.weak_entity();
        let sparks_info = BottomSheetUi::new(
            "gift-sparks-info-sheet",
            Binding::new(
                move |cx| reader.read(cx).is_sparks_info_presented,
                move |presented, _, cx| {
                    if !presented {
                        closer.update(cx, |interactions, cx| interactions.close_sparks_info(cx));
                    }
                },
            ),
            move |_, _| {
                let this = this.clone();
                GiftSparksInfoSheetContent::new(move |height, window, cx| {
                    let _ = this.update(cx, |host, cx| {
                        host.adopt_sparks_info_height(height, button_height, window, cx);
                    });
                })
            },
            |_, _| GiftSparksInfoSheetFooter::new(),
        )
        .snap_fractions(vec![self.sparks_info_fraction])
        .modal_sized_by_detent(true)
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(sheet_background)
        .footer_background(footer_background);

        div()
            .absolute()
            .inset_0()
            .on_key_down(cx.listener(|host, event: &KeyDownEvent, window, cx| host.handle_key_down(event, window, cx)))
            .child(div().absolute().size_0().track_focus(&self.focus_handle))
            .child(shop)
            .child(wardrobe)
            .child(instance)
            .child(odds)
            .child(equip_offer)
            .child(sparks_info)
    }
}
