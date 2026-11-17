use std::ops::Range;

use gpui::{App, BoxShadow, Context, Entity, FontWeight, Hsla, Render, ScrollHandle, SharedString, Subscription, Window, point, white};
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, Locale, tr, tr_args};
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::scroll_chrome_shadow_ui::{ScrollChromeShadowStyle, ScrollChromeShadowUi};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text_selection::SelectableText;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::ui::premium_diamond::{PremiumDiamond, PremiumPageHero};
use crate::core::ui::qr_code::rise_purchase_continue_sheet::RisePurchaseContinue;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_safe_area_insets,
};
use crate::modules::user::shared::premium_features::PremiumFeaturesConfig;
use crate::modules::user::stores::subscription::subscription_actions::subscription_actions::SubscriptionActionsStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_types::{
    SubscriptionBillingDateKind, SubscriptionStatusModel,
};
use crate::modules::user::stores::subscription::subscription_actions::subscription_wire_types::SubscriptionStatusGatewayModel;
use crate::modules::user::stores::subscription::subscription_interactions::subscription_interactions::SubscriptionInteractionsStore;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::{
    Product, SubscriptionPhoneHandoff, SubscriptionServicesStore,
};

const PREMIUM_PRODUCT_NAME: &str = "Riseonly Premium";

fn interactions(cx: &mut App) -> Entity<SubscriptionInteractionsStore> {
    SubscriptionInteractionsStore::shared(cx)
}

/// the two pending requests the page shows, restore under the footer and verify in the buy bar
fn shown_pending(cx: &mut App) -> (bool, bool) {
    let store = SubscriptionActionsStore::shared(cx);
    let actions = store.read(cx);
    (actions.restore_purchases_sai.is_pending(), actions.verify_purchase_sai.is_pending())
}

/// joins spans into one label with color runs
fn joined_text(spans: Vec<(String, Hsla)>) -> (String, Vec<(Range<usize>, Hsla)>) {
    let mut text = String::new();
    let mut runs = Vec::with_capacity(spans.len());
    for (span, color) in spans {
        let start = text.len();
        text.push_str(&span);
        runs.push((start..text.len(), color));
    }
    (text, runs)
}

/// page is an entity the route keeps cached,
/// redraws of route,
/// rail or window reuse its frame and only its stores redraw it
pub struct SubscriptionSettings {
    scroll_handle: ScrollHandle,
    shown_pending: (bool, bool),
    _subscriptions: Vec<Subscription>,
}

impl SubscriptionSettings {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let services = SubscriptionServicesStore::shared(cx);
        let actions = SubscriptionActionsStore::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        Self {
            scroll_handle: ScrollHandle::new(),
            shown_pending: shown_pending(cx),
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&pending_requests, |this, _, cx| {
                    let pending = shown_pending(cx);
                    if pending != this.shown_pending {
                        this.shown_pending = pending;
                        cx.notify();
                    }
                }),
            ],
        }
    }

    fn premium_header() -> impl IntoElement {
        vstack(6.0)
            .w_full()
            .pt(px(8.0))
            .child(PremiumPageHero::new())
            .child(MainText::new(PREMIUM_PRODUCT_NAME).px(28.0).font_weight(FontWeight::BOLD))
    }

    fn active_status_card(
        status: &SubscriptionStatusModel,
        iap_store: &SubscriptionServicesStore,
        theme: &AppTheme,
    ) -> impl IntoElement {
        let plan_title = iap_store.plan_title(status.plan.as_deref());
        div().w_full().px(px(12.0)).child(
            vstack(0.0)
                .w_full()
                .bg(theme.bg._200)
                .clip_shape(Shape::continuous(20.0))
                .child(
                    hstack(12.0)
                        .w_full()
                        .px(px(16.0))
                        .py(px(12.0))
                        .child(PremiumDiamond::new().size(30.0))
                        .child(
                            vstack(2.0)
                                .align(HorizontalAlignment::Leading)
                                .child(
                                    MainText::new(tr("subscription_status_active_title"))
                                        .px(15.0)
                                        .font_weight(FontWeight::SEMIBOLD),
                                )
                                .child(SecondaryText::new(plan_title).px(12.0)),
                        )
                        .child(spacer_min_w(4.0)),
                )
                .child(
                    div()
                        .w_full()
                        .pl(px(52.0))
                        .child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55))),
                )
                .child(
                    plain("subscription-settings-manage")
                        .w_full()
                        .on_press(|_, cx| interactions(cx).update(cx, |store, cx| store.open_manage_subscriptions(cx)))
                        .child(
                            hstack(12.0)
                                .w_full()
                                .px(px(16.0))
                                .py(px(12.0))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .flex_none()
                                        .frame_size(30.0, 30.0)
                                        .bg(theme.primary._100)
                                        .continuous_corners(8.0)
                                        .children(IconUi::sized("arrow.up.forward.app.fill", px(14.0), theme.content_on(theme.primary._100), false)),
                                )
                                .child(MainText::new(tr("subscription_manage_button")).px(15.0))
                                .child(spacer_min_w(4.0))
                                .children(IconUi::sized(
                                    rise_ui::IconUi::trailing_chevron(),
                                    px(12.0),
                                    theme.text.secondary.opacity(0.75),
                                    false,
                                )),
                        ),
                ),
        )
    }

    fn manage_footer(status: &SubscriptionStatusModel, theme: &AppTheme) -> impl IntoElement {
        vstack(6.0)
            .w_full()
            .px(px(24.0))
            .child(SecondaryText::new(tr("subscription_status_active")).px(12.0).tac(TextAlignT::Center).w_full())
            .children(
                status
                    .formatted_billing_date(&Locale::app_locale(), &TimeZone::current())
                    .filter(|date| !date.is_empty())
                    .map(|date| {
                        let label = match status.billing_date_kind {
                            SubscriptionBillingDateKind::Renewal => tr("subscription_next_payment"),
                            SubscriptionBillingDateKind::Expiration => tr("subscription_expires_on"),
                        };
                        MainText::new(format!("{label} {date}"))
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD)
                            .w_full()
                    }),
            )
            .child(
                plain("subscription-settings-manage-hint")
                    .w_full()
                    .on_press(|_, cx| interactions(cx).update(cx, |store, cx| store.open_manage_subscriptions(cx)))
                    .child(Self::manage_hint(theme)),
            )
    }

    fn manage_hint(theme: &AppTheme) -> impl IntoElement {
        let (text, runs) = joined_text(vec![
            (tr("subscription_manage_hint_prefix"), theme.text.secondary),
            (tr("subscription_manage_phone_app"), theme.primary._100),
            (tr("subscription_manage_hint_suffix"), theme.text.secondary),
        ]);
        MainText::new(text).px(12.0).tac(TextAlignT::Center).color_runs(runs).w_full()
    }

    fn plans_section(iap_store: &SubscriptionServicesStore, theme: &AppTheme) -> AnyElement {
        if iap_store.is_loading_products() && iap_store.products().is_empty() {
            return vstack(12.0)
                .w_full()
                .py(px(24.0))
                .child(progress_view())
                .child(SecondaryText::new(tr("subscription_loading_products")).px(13.0).tac(TextAlignT::Center).w_full())
                .into_any_element();
        }
        if iap_store.products().is_empty() {
            let load_error = iap_store.load_error().map(str::to_owned);
            let color = if load_error.is_none() { theme.text.secondary } else { theme.semantic.error_100 };
            let diagnostics = iap_store.store_diagnostics().map(str::to_owned);
            let secondary = theme.text.secondary;
            return vstack(12.0)
                .w_full()
                .px(px(24.0))
                .child(
                    SecondaryText::new(load_error.unwrap_or_else(|| tr("subscription_products_unavailable")))
                        .px(13.0)
                        .tac(TextAlignT::Center)
                        .color(color)
                        .w_full(),
                )
                .children(diagnostics.map(|diagnostics| {
                    div()
                        .w_full()
                        .text_center()
                        .text_size(px(11.0))
                        .text_color(secondary)
                        .child(SelectableText::new("subscription-settings-diagnostics", diagnostics))
                }))
                .child(
                    plain("subscription-settings-retry")
                        .on_press(|_, cx| {
                            SubscriptionServicesStore::shared(cx).update(cx, |store, cx| store.load_products(cx));
                        })
                        .child(
                            MainText::new(tr("subscription_retry"))
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.primary._100),
                        ),
                )
                .into_any_element();
        }

        let items: Vec<GroupedBtn> = iap_store.products().iter().map(|product| Self::plan_row(product, iap_store)).collect();
        div().w_full().child(GroupedBtns::new("subscription-settings-plans", items)).into_any_element()
    }

    fn plan_row(product: &Product, iap_store: &SubscriptionServicesStore) -> GroupedBtn {
        let display_title = iap_store.display_title(product);
        let title = if iap_store.is_recommended(product) {
            format!("{display_title} · {}", tr("subscription_recommended"))
        } else {
            display_title
        };
        let mut subtitle_parts = vec![iap_store.display_price(product)];
        if let Some(saving) = iap_store.savings_percent(product) {
            subtitle_parts.push(tr_args("subscription_save_percent", &[FormatArg::Int(saving)]));
        }
        if let Some(trial) = iap_store.intro_offer_label(product) {
            subtitle_parts.push(trial);
        }
        subtitle_parts.retain(|part| !part.is_empty());
        let product_id = product.id.clone();
        let mut row = GroupedBtn::new(SharedString::from(product.id.clone()), title).show_chevron(false);
        if !subtitle_parts.is_empty() {
            row = row.subtitle(subtitle_parts.join(" · "));
        }
        let row = row
            .callback(move |_, cx| {
                let _ = selection_changed();
                let id = product_id.clone();
                SubscriptionServicesStore::shared(cx).update(cx, |store, cx| store.select_product(id, cx));
            });
        if iap_store.selected_product_id() == Some(product.id.as_str()) { row.trailing("\u{2713}") } else { row }
    }

    fn features_group(theme: &AppTheme) -> impl IntoElement {
        let items: Vec<GroupedBtn> = PremiumFeaturesConfig::features(theme.primary._100)
            .into_iter()
            .map(|feature| {
                GroupedBtn::new(SharedString::from(feature.id), feature.title)
                    .icon(feature.icon)
                    .icon_background(feature.color)
                    .subtitle(feature.subtitle)
                    .show_chevron(false)
            })
            .collect();
        div().w_full().child(GroupedBtns::new("subscription-settings-features", items))
    }

    fn info_text(iap_store: &SubscriptionServicesStore) -> impl IntoElement {
        let note = selected_product(iap_store)
            .map(|product| iap_store.renewal_note(product))
            .unwrap_or_else(|| tr("subscription_info_text"));
        div().w_full().px(px(24.0)).child(SecondaryText::new(note).px(11.0).tac(TextAlignT::Center).w_full())
    }

    /// what the phone handoff modal of the premium page says
    /// the root purchase continue host takes it once when the handoff opens
    pub(crate) fn continue_on_phone(iap_store: &SubscriptionServicesStore) -> RisePurchaseContinue {
        const ID: &str = "subscription-settings-on-phone";
        match iap_store.continue_on_phone() {
            Some(SubscriptionPhoneHandoff::Manage) => {
                RisePurchaseContinue::new(ID, tr("subscription_manage_button"), tr("subscription_manage_on_phone"))
            }
            Some(SubscriptionPhoneHandoff::Restore) => {
                RisePurchaseContinue::new(ID, tr("subscription_restore_button"), tr("premium_gift_on_phone"))
            }
            Some(SubscriptionPhoneHandoff::Purchase { .. }) | None => {
                RisePurchaseContinue::purchase(ID, tr("premium_gift_on_phone")).link(iap_store.phone_link())
            }
        }
    }
}

fn selected_product(iap_store: &SubscriptionServicesStore) -> Option<&Product> {
    let selected = iap_store.selected_product_id()?;
    iap_store.products().iter().find(|product| product.id == selected)
}

impl Render for SubscriptionSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let scroll_handle = self.scroll_handle.clone();
        let services = SubscriptionServicesStore::shared(cx);
        let actions = SubscriptionActionsStore::shared(cx);
        let status = actions
            .read(cx)
            .subscription_status_sai
            .data()
            .map(|data| data.as_subscription_status_model(RiseClock::now()));
        let nav_store = env::<NavigationStoreKey>(cx);
        let active_status = status.filter(|status| status.is_active);
        let is_active = active_status.is_some();
        let iap_store = services.read(cx);
        let last_error = iap_store
            .last_error()
            .filter(|error| !error.is_empty())
            .map(str::to_owned);

        let mut sections = vstack(30.0).w_full().pt(px(12.0)).child(Self::premium_header());
        sections = match &active_status {
            Some(status) => sections
                .child(Self::active_status_card(status, iap_store, &theme))
                .child(Self::features_group(&theme))
                .child(Self::manage_footer(status, &theme))
                .child(SubscriptionLegalLinks::new(theme.clone()))
                .child(div().flex_none().h(px(24.0))),
            None => sections
                .child(Self::plans_section(iap_store, &theme))
                .child(Self::features_group(&theme))
                .child(Self::info_text(iap_store)),
        };
        sections = sections.children(last_error.map(|error| {
            div().w_full().px(px(24.0)).child(
                SecondaryText::new(error).px(13.0).tac(TextAlignT::Center).color(theme.semantic.error_100).w_full(),
            )
        }));
        if !is_active {
            sections = sections.child(SubscriptionPageFooter::new(theme.clone())).child(SubscriptionBuyBarClearance);
        }

        SettingsWrapperUi::new(tr("settings_subscription_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            div()
                .relative()
                .size_full()
                .child(
                    scroll_view("subscription-settings-scroll")
                        .size_full()
                        .bg(theme.bg._100)
                        .scroll_handle(scroll_handle.clone())
                        .list_shell_scroll_under_chrome(cx)
                        .child(sections),
                )
                .when(!is_active, |stack| stack.child(SubscriptionBuyBar::new(theme.clone(), scroll_handle)))
        )
        .on_appear("subscription-settings-appear", |_, cx| {
            interactions(cx).update(cx, |store, cx| {
                store.ensure_iap_started(cx);
                store.ensure_subscription_status_loaded(cx);
            });
        })
        .and_on_disappear(|_, cx| {
            let store = SubscriptionServicesStore::shared(cx);
            if store.read(cx).continue_on_phone().is_some() {
                tracing::warn!(target: "riseonly::X2374", "purchase continue closed: premium page left the frame");
            }
            store.update(cx, |store, cx| store.dismiss_continue_on_phone(cx));
        })
    }
}

struct SubscriptionLegalLinksFit {
    available_width: f32,
    row_width: f32,
}

/// legal links required on any surface that sells or manages a subscription
/// two buttons not one sentence, one hit target cant serve two destinations
#[derive(IntoElement)]
struct SubscriptionLegalLinks {
    theme: AppTheme,
}

impl SubscriptionLegalLinks {
    fn new(theme: AppTheme) -> Self {
        Self { theme }
    }

    fn label(key: &str, lines: Option<usize>, color: Hsla) -> MainText {
        MainText::new(tr(key)).px(12.0).tac(TextAlignT::Center).color(color).line_limit(lines)
    }

    fn terms_button(id: &'static str, lines: Option<usize>, color: Hsla) -> PlainButton {
        plain(id)
            .on_press(|_, cx| interactions(cx).update(cx, |store, cx| store.open_terms_of_use(cx)))
            .child(Self::label("subscription_terms_button", lines, color))
    }

    fn privacy_button(id: &'static str, lines: Option<usize>, color: Hsla) -> PlainButton {
        plain(id)
            .on_press(|_, cx| interactions(cx).update(cx, |store, cx| store.open_privacy_policy(cx)))
            .child(Self::label("subscription_privacy_button", lines, color))
    }
}

impl RenderOnce for SubscriptionLegalLinks {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = self.theme.text.secondary;
        let fit = window.use_keyed_state("subscription-legal-links-fit", cx, |_, _| SubscriptionLegalLinksFit {
            available_width: 0.0,
            row_width: 0.0,
        });
        let fits = {
            let fit = fit.read(cx);
            fit.row_width <= fit.available_width
        };
        let available = fit.clone();
        let reported = fit;

        let links = if fits {
            hstack(16.0)
                .w_full()
                .justify_center()
                .child(Self::terms_button("subscription-terms-row", Some(1), color))
                .child(Self::privacy_button("subscription-privacy-row", Some(1), color))
        } else {
            vstack(8.0)
                .w_full()
                .child(Self::terms_button("subscription-terms-stack", None, color))
                .child(Self::privacy_button("subscription-privacy-stack", None, color))
        };
        div()
            .relative()
            .w_full()
            .child(measure("subscription-legal-links-available", move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                available.update(cx, |fit, cx| {
                    if fit.available_width != width {
                        fit.available_width = width;
                        cx.notify();
                    }
                });
            }))
            .child(links)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .opacity(0.0)
                    .whitespace_nowrap()
                    .child(
                        hstack(16.0)
                            .child(Self::label("subscription_terms_button", Some(1), color))
                            .child(Self::label("subscription_privacy_button", Some(1), color)),
                    )
                    .child(measure("subscription-legal-links-row", move |bounds, _, cx| {
                        let width = f32::from(bounds.size.width);
                        reported.update(cx, |fit, cx| {
                            if fit.row_width != width {
                                fit.row_width = width;
                                cx.notify();
                            }
                        });
                    })),
            )
    }
}

/// geometry of the pinned purchase bar, must agree with the clearance the scroll content reserves
/// fixed button height keeps the bar from resizing when the label swaps to a spinner
enum SubscriptionBuyBarMetrics {}

impl SubscriptionBuyBarMetrics {
    const BUTTON_HEIGHT: f32 = 50.0;
    const TOP_PADDING: f32 = 12.0;
    const BOTTOM_GAP: f32 = 12.0;
    const MINIMUM_BOTTOM_INSET: f32 = 8.0;
    const FOOTER_BREATHING_ROOM: f32 = 16.0;

    fn bar_height(bottom_inset: f32) -> f32 {
        Self::BUTTON_HEIGHT + Self::TOP_PADDING + bottom_inset.max(Self::MINIMUM_BOTTOM_INSET) + Self::BOTTOM_GAP
    }
}

#[derive(IntoElement)]
struct SubscriptionBuyBarClearance;

impl RenderOnce for SubscriptionBuyBarClearance {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let bottom = list_shell_safe_area_insets(cx).bottom;
        div().flex_none().h(px(SubscriptionBuyBarMetrics::bar_height(bottom) + SubscriptionBuyBarMetrics::FOOTER_BREATHING_ROOM))
    }
}

#[derive(IntoElement)]
struct SubscriptionPageFooter {
    theme: AppTheme,
}

impl SubscriptionPageFooter {
    fn new(theme: AppTheme) -> Self {
        Self { theme }
    }

    fn cancel_hint(theme: &AppTheme) -> impl IntoElement {
        let (text, runs) = joined_text(vec![
            (tr("subscription_cancel_anytime_prefix"), theme.text.secondary),
            (tr("subscription_cancel_phone_app"), theme.primary._100),
        ]);
        MainText::new(text).px(12.0).tac(TextAlignT::Center).color_runs(runs).w_full()
    }
}

impl RenderOnce for SubscriptionPageFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let actions = SubscriptionActionsStore::shared(cx);
        let is_restoring = actions.read(cx).restore_purchases_sai.is_pending();
        let secondary = self.theme.text.secondary;
        let restore_label = if is_restoring {
            progress_view().tint(secondary).into_any_element()
        } else {
            MainText::new(tr("subscription_restore_button")).px(13.0).color(secondary).into_any_element()
        };

        vstack(14.0)
            .w_full()
            .px(px(24.0))
            .child(
                plain("subscription-settings-restore")
                    .w_full()
                    .disabled(is_restoring)
                    .on_press(|_, cx| interactions(cx).update(cx, |store, cx| store.restore_purchases(cx)))
                    .child(div().flex().justify_center().w_full().child(restore_label)),
            )
            .child(
                plain("subscription-settings-cancel-hint")
                    .w_full()
                    .on_press(|_, cx| interactions(cx).update(cx, |store, cx| store.open_app_store_subscriptions(cx)))
                    .child(Self::cancel_hint(&self.theme)),
            )
            .child(SubscriptionLegalLinks::new(self.theme))
    }
}

#[derive(IntoElement)]
struct SubscriptionBuyBar {
    theme: AppTheme,
    scroll_handle: ScrollHandle,
}

impl SubscriptionBuyBar {
    fn new(theme: AppTheme, scroll_handle: ScrollHandle) -> Self {
        Self { theme, scroll_handle }
    }

    fn label(iap_store: &SubscriptionServicesStore) -> String {
        let Some(selected) = selected_product(iap_store) else {
            return tr("subscription_buy_text");
        };
        let price = iap_store.display_price(selected);
        if let Some(trial) = iap_store.intro_offer_label(selected) {
            return format!(
                "{} · {}",
                tr_args("subscription_trial_cta", &[FormatArg::Text(trial.as_str().into())]),
                tr_args("subscription_trial_then", &[FormatArg::Text(price.as_str().into())]),
            );
        }
        format!("{} {price}", tr("subscription_buy_text"))
    }
}

impl RenderOnce for SubscriptionBuyBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let primary = self.theme.primary._100;
        let scroll_handle = self.scroll_handle.clone();
        let bottom = list_shell_safe_area_insets(cx).bottom;
        let services = SubscriptionServicesStore::shared(cx);
        let actions = SubscriptionActionsStore::shared(cx);
        let is_verifying = actions.read(cx).verify_purchase_sai.is_pending();
        let iap_store = services.read(cx);
        let shows_progress =
            iap_store.is_purchasing() || is_verifying || (iap_store.is_loading_products() && iap_store.products().is_empty());
        let is_busy = shows_progress || selected_product(iap_store).is_none();
        let content = if shows_progress {
            progress_view().tint(white()).into_any_element()
        } else {
            MainText::new(Self::label(iap_store))
                .px(16.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(white())
                .line_limit(1)
                .minimum_scale_factor(0.82)
                .into_any_element()
        };

        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .px(px(16.0))
            .pt(px(SubscriptionBuyBarMetrics::TOP_PADDING))
            .pb(px(bottom.max(SubscriptionBuyBarMetrics::MINIMUM_BOTTOM_INSET) + SubscriptionBuyBarMetrics::BOTTOM_GAP))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .child(ScrollChromeShadowUi::new(ScrollChromeShadowStyle::floating_bottom_action())),
            )
            .child(
                div().relative().w_full().rounded(px(SubscriptionBuyBarMetrics::BUTTON_HEIGHT / 2.0)).bg(self.theme.bg._100).child(
                plain("subscription-settings-buy")
                    .relative()
                    .w_full()
                    .disabled(is_busy)
                    .on_press(move |_, cx| {
                        interactions(cx).update(cx, |store, cx| store.buy_subscription(cx));
                        scroll_handle.scroll_to_bottom();
                    })
                    .shadow(vec![BoxShadow {
                        color: primary.opacity(0.32),
                        offset: point(px(0.0), px(6.0)),
                        blur_radius: px(28.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .capsule()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w_full()
                            .h(px(SubscriptionBuyBarMetrics::BUTTON_HEIGHT))
                            .bg(primary.opacity(if is_busy { 0.65 } else { 1.0 }))
                            .clip_shape(Shape::Capsule)
                            .child(content),
                    ),
                ),
            )
    }
}
