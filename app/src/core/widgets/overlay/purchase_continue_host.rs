use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, Context, Entity, IntoElement, ParentElement, Render, RenderImage, SharedString, Styled,
    Subscription, Task, Window, div,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::ThemeStore;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::NativeBottomSheetPresentationHost;
use rise_ui::swiftui::prelude::*;
use rise_ui::top_presenter::TopPresenter;

use crate::core::ui::qr_code::rise_phone_handoff_code::RisePhoneHandoffCode;
use crate::core::ui::qr_code::rise_purchase_continue_sheet::{RisePurchaseContinue, RisePurchaseContinueMetrics};
use crate::modules::sparks::components::sparks_sheets::SparksPurchaseSheetContent;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksPurchasePhase;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;
use crate::modules::user::components::premium_gift::premium_gift_sheet::PremiumGiftSheet;
use crate::modules::user::pages::profile::settings::subscription_settings::subscription_settings::SubscriptionSettings;
use crate::modules::user::stores::subscription::premium_gift_store::PremiumGiftStore;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::SubscriptionServicesStore;

/// which purchase asked for the phone handoff
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PurchaseContinueSource {
    PremiumGift,
    Sparks,
    Subscription,
}

impl PurchaseContinueSource {
    fn closes_sheets_first(self) -> bool {
        matches!(self, Self::PremiumGift | Self::Subscription)
    }
}

/// what the code raster was made for
#[derive(Clone, PartialEq)]
struct PurchaseContinueCodeKey {
    link: SharedString,
    scale: f32,
}

/// the one continue purchase on phone modal of premium gifts, sparks and the premium page
/// at the app root above every sheet, one presentation with a constant id
/// what it says is taken once when a handoff opens, so a render of a store under it never rebuilds it
/// the code is rastered before the modal shows, so it opens whole with title, code and hint at once
pub struct PurchaseContinueHost {
    premium_gift: Entity<PremiumGiftStore>,
    sparks_services: Entity<SparksServicesStore>,
    sparks_interactions: Entity<SparksInteractionsStore>,
    subscription: Entity<SubscriptionServicesStore>,
    source: Option<PurchaseContinueSource>,
    awaiting: Option<PurchaseContinueSource>,
    window: Option<AnyWindowHandle>,
    sheets_close_task: Option<Task<()>>,
    content: Option<RisePurchaseContinue>,
    code_key: Option<PurchaseContinueCodeKey>,
    code: Option<Option<Arc<RenderImage>>>,
    code_task: Option<Task<()>>,
    _observations: [Subscription; 4],
}

impl PurchaseContinueHost {
    /// above every sheet (their layers start at 1 and grow with the stack) and below the full window covers
    pub const LAYER_PRIORITY: usize = 900;
    const SHEET_ID: &'static str = "root-purchase-continue";
    const SHEETS_CLOSE_POLL: Duration = Duration::from_millis(16);
    const SHEETS_CLOSE_LIMIT: Duration = Duration::from_millis(1_200);

    pub fn new(cx: &mut Context<Self>) -> Self {
        let premium_gift = PremiumGiftStore::shared(cx);
        let sparks_services = SparksServicesStore::shared(cx);
        let sparks_interactions = SparksInteractionsStore::shared(cx);
        let subscription = SubscriptionServicesStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let observations = [
            cx.observe(&premium_gift, |host, _, cx| host.sync(cx)),
            cx.observe(&sparks_services, |host, _, cx| host.sync(cx)),
            cx.observe(&subscription, |host, _, cx| host.sync(cx)),
            cx.observe(&theme_store, |_, _, cx| cx.notify()),
        ];
        let mut host = Self {
            premium_gift,
            sparks_services,
            sparks_interactions,
            subscription,
            source: None,
            awaiting: None,
            window: None,
            sheets_close_task: None,
            content: None,
            code_key: None,
            code: None,
            code_task: None,
            _observations: observations,
        };
        host.source = host.requested(cx);
        host.content = host.source.map(|source| host.content_for(source, cx));
        host
    }

    fn requested(&self, cx: &App) -> Option<PurchaseContinueSource> {
        if self.premium_gift.read(cx).continue_on_phone().is_some() {
            Some(PurchaseContinueSource::PremiumGift)
        } else if matches!(self.sparks_services.read(cx).purchase_phase, SparksPurchasePhase::ContinueOnPhone { .. }) {
            Some(PurchaseContinueSource::Sparks)
        } else if self.subscription.read(cx).continue_on_phone().is_some() {
            Some(PurchaseContinueSource::Subscription)
        } else {
            None
        }
    }

    fn content_for(&self, source: PurchaseContinueSource, cx: &App) -> RisePurchaseContinue {
        match source {
            PurchaseContinueSource::PremiumGift => PremiumGiftSheet::continue_on_phone_content(&self.premium_gift, &self.subscription, cx),
            PurchaseContinueSource::Sparks => SparksPurchaseSheetContent::continue_on_phone_content(&self.sparks_services, cx),
            PurchaseContinueSource::Subscription => SubscriptionSettings::continue_on_phone(self.subscription.read(cx)),
        }
    }

    /// a store changed: a handoff that opened or switched takes its text and code now, a closed one leaves
    /// the last content stays for the closing frame
    /// a premium handoff first closes every sheet and shows once they are gone
    fn sync(&mut self, cx: &mut Context<Self>) {
        let requested = self.requested(cx);
        if requested == self.source.or(self.awaiting) {
            return;
        }
        self.awaiting = None;
        self.sheets_close_task = None;
        match requested {
            Some(source) if source.closes_sheets_first() => {
                tracing::warn!(target: "riseonly::X2374", ?source, previous = ?self.source, "purchase continue closes the sheets first");
                self.content = Some(self.content_for(source, cx));
                self.source = None;
                self.awaiting = Some(source);
                self.close_sheets_then_present(cx);
            }
            Some(source) => {
                tracing::warn!(target: "riseonly::X2374", ?source, previous = ?self.source, "purchase continue opened");
                self.content = Some(self.content_for(source, cx));
                self.source = Some(source);
            }
            None => {
                tracing::warn!(target: "riseonly::X2374", source = ?self.source, "purchase continue closed by its store");
                self.source = None;
            }
        }
        cx.notify();
    }

    fn close_sheets_then_present(&mut self, cx: &mut Context<Self>) {
        let Some(window) = self.window else {
            self.present_awaited(cx);
            return;
        };
        let started = RiseClock::monotonic();
        self.sheets_close_task = Some(cx.spawn(async move |host, cx| {
            let _ = window.update(cx, |_, window, cx| TopPresenter::dismiss_sheets(window, cx));
            loop {
                cx.background_executor().timer(Self::SHEETS_CLOSE_POLL).await;
                let is_clear = cx.update(|cx| !TopPresenter::has_sheet_in(window.window_id(), cx));
                if is_clear || RiseClock::monotonic().saturating_sub(started) >= Self::SHEETS_CLOSE_LIMIT {
                    break;
                }
            }
            let _ = host.update(cx, |host, cx| host.present_awaited(cx));
        }));
    }

    fn present_awaited(&mut self, cx: &mut Context<Self>) {
        self.sheets_close_task = None;
        let Some(source) = self.awaiting.take() else {
            return;
        };
        if self.requested(cx) != Some(source) {
            return;
        }
        tracing::warn!(target: "riseonly::X2374", ?source, "purchase continue opened after the sheets closed");
        self.source = Some(source);
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X2374", source = ?self.source, "purchase continue dismissed by the modal: close button, escape or backdrop");
        match self.source {
            Some(PurchaseContinueSource::PremiumGift) => {
                self.premium_gift.update(cx, |store, cx| store.dismiss_continue_on_phone(cx));
            }
            Some(PurchaseContinueSource::Sparks) => {
                self.sparks_interactions.update(cx, |interactions, cx| interactions.dismiss_continue_on_phone(cx));
            }
            Some(PurchaseContinueSource::Subscription) => {
                self.subscription.update(cx, |store, cx| store.dismiss_continue_on_phone(cx));
            }
            None => {}
        }
        self.sync(cx);
    }

    /// rasters the code of the open handoff once per link and scale, the modal waits for it
    fn ensure_code(&mut self, scale: f32, cx: &mut Context<Self>) {
        if self.source.is_none() && self.awaiting.is_none() {
            return;
        }
        let Some(content) = self.content.as_ref() else {
            return;
        };
        let key = PurchaseContinueCodeKey { link: content.code_link(), scale };
        if self.code_key.as_ref() == Some(&key) {
            return;
        }
        self.code_key = Some(key.clone());
        self.code = None;
        let assets = Arc::clone(cx.asset_source());
        let started = RiseClock::monotonic();
        self.code_task = Some(cx.spawn(async move |host, cx| {
            let image =
                RisePhoneHandoffCode::raster(key.link.clone(), RisePurchaseContinueMetrics::CODE_SIDE, key.scale, assets).await;
            let _ = host.update(cx, |host, cx| {
                if host.code_key.as_ref() != Some(&key) {
                    return;
                }
                tracing::warn!(
                    target: "riseonly::X2374",
                    rastered = image.is_some(),
                    ms = RiseClock::monotonic().saturating_sub(started).as_millis() as u64,
                    "purchase continue code ready"
                );
                host.code = Some(image);
                host.code_task = None;
                cx.notify();
            });
        }));
    }
}

impl Render for PurchaseContinueHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.window = Some(window.window_handle());
        self.ensure_code(window.scale_factor(), cx);
        let is_code_ready = self.code.is_some();
        if self.source.is_some() {
            tracing::warn!(target: "riseonly::X2374", source = ?self.source, code_ready = is_code_ready, "purchase continue layer render");
        }
        let background = rise_ui::theme(cx).bg._100;
        let (reader, writer) = (cx.entity(), cx.entity());
        let is_presented = Binding::new(
            move |cx: &App| reader.read(cx).source.is_some(),
            move |value: bool, _: &mut Window, cx: &mut App| {
                if !value {
                    writer.update(cx, |host, cx| host.dismiss(cx));
                }
            },
        );
        let code = self.code.clone().flatten();
        let content = self.content.clone().map(|content| content.code_image(code));
        let sheet = NativeBottomSheetPresentationHost::new(Self::SHEET_ID, is_presented, move |_, _| {
            div().w_full().children(content.clone())
        })
        .is_content_ready(is_code_ready)
        .presentation_drag_indicator(false)
        .presentation_background(background)
        .compact(true)
        .layer_priority(Self::LAYER_PRIORITY);
        div().size_0().child(sheet)
    }
}
