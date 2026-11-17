use gpui::{App, Context, Empty, Entity, FontWeight, SharedString, Subscription, Window, white};
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::haptics::selection_changed;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::qr_code::rise_purchase_continue_sheet::RisePurchaseContinue;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::user::stores::subscription::premium_gift_store::{PremiumGiftStore, PremiumGiftStoreMode};
use crate::modules::user::stores::subscription::subscription_actions::subscription_actions::SubscriptionActionsStore;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::{
    Product, SubscriptionServicesStore,
};

/// premium gifting sheet hosted once at the app root, driven by is_presented
/// any surface with a user id can open it, rows are the gift skus
pub struct PremiumGiftHost {
    store: Entity<PremiumGiftStore>,
    _subscriptions: Vec<Subscription>,
}

impl PremiumGiftHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = PremiumGiftStore::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&pending_requests, |host, _, cx| {
                if host.store.read(cx).is_presented() {
                    cx.notify();
                }
            }),
        ];
        Self { store, _subscriptions: subscriptions }
    }
}

impl Render for PremiumGiftHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (reader, closer, finisher) = (self.store.clone(), self.store.clone(), self.store.clone());

        BottomSheetUi::new(
            "premium-gift-sheet",
            Binding::new(
                move |cx| reader.read(cx).is_presented(),
                move |is_presented, _, cx| {
                    if !is_presented {
                        closer.update(cx, |store, cx| store.dismiss(cx));
                    }
                },
            ),
            |_, _| PremiumGiftSheet::new(),
            |_, _| Empty,
        )
        .experimental_bottom_sheet(false)
        .snap_fractions(vec![1.0])
        .on_did_dismiss(move |_, cx| finisher.update(cx, |store, cx| store.sheet_did_dismiss(cx)))
    }
}

#[derive(IntoElement, Default)]
pub struct PremiumGiftSheet;

impl PremiumGiftSheet {
    pub fn new() -> Self {
        Self
    }

    fn for_self(store: &PremiumGiftStore) -> bool {
        store.mode() == PremiumGiftStoreMode::ForSelf
    }

    fn title(store: &PremiumGiftStore) -> String {
        tr(if Self::for_self(store) { "premium_self_title" } else { "premium_gift_title" })
    }

    fn header(store: &Entity<PremiumGiftStore>, cx: &App) -> Div {
        let title = Self::title(store.read(cx));
        let closer = store.clone();

        div()
            .w_full()
            .pl(px(14.0))
            .pr(px(14.0))
            .pt(px(12.0))
            .pb(px(12.0))
            .child(
                zstack(Alignment::Center)
                    .w_full()
                    .child(MainText::new(title).px(16.0).font_weight(FontWeight::SEMIBOLD))
                    .child(hstack(DEFAULT_STACK_SPACING).w_full().child(spacer()).child(
                        PageHeaderIconButton::new("premium-gift-close", "xmark", move |_, cx| {
                            closer.update(cx, |store, cx| store.dismiss(cx));
                        })
                        .font_size(15.0),
                    )),
            )
    }

    fn recipient_card(store: &PremiumGiftStore) -> Option<Div> {
        let recipient = store.recipient()?;
        let subtitle = if Self::for_self(store) {
            tr("premium_self_subtitle")
        } else {
            tr_args("premium_gift_subtitle", &[FormatArg::Text(recipient.name.as_str().into())])
        };
        let source = (!recipient.logo.is_empty()).then(|| SharedString::from(recipient.logo.clone()));

        Some(
            vstack(10.0)
                .w_full()
                .pt(px(8.0))
                .child(
                    UserLogoUi::new("premium-gift-recipient")
                        .source(source)
                        .name(SharedString::from(recipient.name.clone()))
                        .user_id(SharedString::from(recipient.id.clone()))
                        .size(72.0)
                        .show_story_ring(false),
                )
                .child(
                    MainText::new(recipient.name.clone())
                        .px(17.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(
                    div()
                        .w_full()
                        .pl(px(24.0))
                        .pr(px(24.0))
                        .child(SecondaryText::new(subtitle).px(13.0).tac(TextAlignT::Center).w_full()),
                ),
        )
    }

    fn plans(store: &Entity<PremiumGiftStore>, cx: &mut App) -> AnyElement {
        let services = SubscriptionServicesStore::shared(cx);
        let primary = rise_ui::theme(cx).primary._100;
        let gift_store = store.read(cx);

        if gift_store.is_loading_products() && gift_store.products().is_empty() {
            return div().pt(px(24.0)).pb(px(24.0)).child(progress_view()).into_any_element();
        }

        if gift_store.products().is_empty() {
            let message = gift_store.load_error().map_or_else(|| tr("premium_gift_products_unavailable"), str::to_owned);
            let retry = store.clone();
            return vstack(12.0)
                .w_full()
                .pl(px(24.0))
                .pr(px(24.0))
                .child(SecondaryText::new(message).px(13.0).tac(TextAlignT::Center).w_full())
                .child(
                    plain("premium-gift-retry")
                        .on_press(move |_, cx| retry.update(cx, |store, cx| store.load_products(cx)))
                        .child(MainText::new(tr("subscription_retry")).color(primary)),
                )
                .into_any_element();
        }

        let items = gift_store
            .products()
            .iter()
            .map(|product| {
                let price = services.read(cx).display_price(product);
                let selecter = store.clone();
                let product_id = product.id.clone();
                let mut row = GroupedBtn::new(product.id.clone(), gift_store.period_title(product))
                    .show_chevron(false)
                    .callback(move |_, cx| {
                        selection_changed();
                        let product_id = product_id.clone();
                        selecter.update(cx, |store, cx| store.select_product(product_id, cx));
                    });
                if !price.is_empty() {
                    row = row.subtitle(price);
                }
                if gift_store.selected_product_id() == Some(product.id.as_str()) {
                    row = row.trailing("✓");
                }
                row
            })
            .collect();

        div().w_full().child(GroupedBtns::new("premium-gift-plans", items)).into_any_element()
    }

    /// what the phone handoff modal of a premium gift purchase says, from this sheet or the gift shop
    /// the root purchase continue host takes it once when the handoff opens
    pub fn continue_on_phone_content(
        store: &Entity<PremiumGiftStore>,
        services: &Entity<SubscriptionServicesStore>,
        cx: &App,
    ) -> RisePurchaseContinue {
        let gift_store = store.read(cx);
        let detail = gift_store
            .continue_on_phone()
            .and_then(|_| gift_store.selected_product_id())
            .and_then(|selected| {
                gift_store
                    .products()
                    .iter()
                    .find(|product| product.id == selected)
                    .cloned()
                    .or_else(|| gift_store.shelf_products().into_iter().find(|product| product.id == selected))
            })
            .map(|product| {
                let period = gift_store.period_title(&product);
                let price = services.read(cx).display_price(&product);
                if price.is_empty() { period } else { format!("{period} \u{00B7} {price}") }
            });
        RisePurchaseContinue::purchase("premium-gift-on-phone", tr("premium_gift_on_phone"))
            .detail(detail)
            .link(gift_store.phone_link())
    }

    fn buy_label(store: &PremiumGiftStore, product: Option<&Product>, services: &SubscriptionServicesStore) -> String {
        let cta = if Self::for_self(store) { "premium_self_cta" } else { "premium_gift_cta" };
        match product.map(|product| services.display_price(product)) {
            Some(price) if !price.is_empty() => tr_args(cta, &[FormatArg::Text(price.as_str().into())]),
            _ => Self::title(store),
        }
    }

    fn buy_bar(store: &Entity<PremiumGiftStore>, cx: &mut App) -> Div {
        let primary = rise_ui::theme(cx).primary._100;
        let services = SubscriptionServicesStore::shared(cx);
        let actions = SubscriptionActionsStore::shared(cx);
        let verify_is_pending = actions.read(cx).verify_gift_purchase_sai.is_pending();
        let gift_store = store.read(cx);
        let product = gift_store
            .products()
            .iter()
            .find(|product| Some(product.id.as_str()) == gift_store.selected_product_id());
        let is_busy = gift_store.is_purchasing() || verify_is_pending || product.is_none();

        let content = if is_busy && product.is_some() {
            progress_view().tint(white()).into_any_element()
        } else {
            MainText::new(Self::buy_label(gift_store, product, services.read(cx)))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(white())
                .line_limit(1)
                .minimum_scale_factor(0.82)
                .into_any_element()
        };
        let purchaser = store.clone();

        div().w_full().pl(px(16.0)).pr(px(16.0)).pt(px(10.0)).pb(px(16.0)).child(
            plain("premium-gift-buy")
                .disabled(is_busy)
                .on_press(move |_, cx| purchaser.update(cx, |store, cx| store.purchase_selected(cx)))
                .w_full()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w_full()
                        .h(px(52.0))
                        .bg(primary.opacity(if is_busy { 0.65 } else { 1.0 }))
                        .clip_shape(Shape::Capsule)
                        .child(content),
                ),
        )
    }
}

impl RenderOnce for PremiumGiftSheet {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = PremiumGiftStore::shared(cx);
        let (background, text_secondary) = {
            let theme = rise_ui::theme(cx);
            (theme.bg._100, theme.text.secondary)
        };
        let header = Self::header(&store, cx);
        let plans = Self::plans(&store, cx);
        let buy_bar = Self::buy_bar(&store, cx);

        let (recipient_card, error, note) = {
            let gift_store = store.read(cx);
            let error = gift_store
                .last_error()
                .filter(|error| !error.is_empty())
                .map(str::to_owned)
                .map(|error| {
                    div().w_full().pl(px(24.0)).pr(px(24.0)).child(
                        MainText::new(error).px(13.0).tac(TextAlignT::Center).color(text_secondary).w_full(),
                    )
                });
            let note = tr(if Self::for_self(gift_store) { "premium_self_note" } else { "premium_gift_note" });
            (Self::recipient_card(gift_store), error, note)
        };

        let loader = store.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .child(header)
            .child(
                div().id("premium-gift-scroll").flex_1().min_h_0().w_full().overflow_y_scroll().child(
                    vstack(18.0)
                        .w_full()
                        .pt(px(8.0))
                        .pb(px(24.0))
                        .children(recipient_card)
                        .child(plans)
                        .children(error)
                        .child(
                            div()
                                .w_full()
                                .pl(px(24.0))
                                .pr(px(24.0))
                                .child(SecondaryText::new(note).px(12.0).tac(TextAlignT::Center).w_full()),
                        ),
                ),
            )
            .child(buy_bar)
            .on_appear("premium-gift-sheet-task", move |_, cx| {
                loader.update(cx, |store, cx| store.load_products_if_needed(cx));
            })
    }
}
