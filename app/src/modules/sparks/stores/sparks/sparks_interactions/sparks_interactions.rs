use gpui::{App, AsyncApp, Bounds, Context, Entity, Pixels, Point, Task};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_errors::{ErrorCode, ErrorParam};
use rise_i18n::tr;
use rise_canon::rise_ledger::{HistoryFilter as RiseLedgerHistoryFilter, SPARKS_SEND_UNDO_WINDOW_MS};
use rise_platform::haptics::{NotificationKind, notification};

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::purchase_celebration::{PurchaseCelebration, PurchaseCelebrationDirector};
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierType, NotifierUndoOptions,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::sparks::engine::rise_sparks_presentation::RiseSparksPresentation;
use crate::modules::sparks::engine::rise_sparks_engine_models::{RiseSparksLimits, RiseSparksLoadPhase};
use crate::modules::sparks::engine::rise_sparks_purchases::{
    RiseSparksPackOffer, RiseSparksPurchaseOutcome, RiseSparksPurchases,
};
use crate::modules::sparks::engine::rise_sparks_repository::{RiseSparksFailure, RiseSparksTransferError};
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_actions::SparksActionsStore;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::{
    SparksBurst, SparksOffersPhase, SparksPurchasePhase, SparksSendSession, SparksTarget,
};
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub struct SparksInteractionsStore {}

impl SparksInteractionsStore {
    const UNDO_WINDOW_SECONDS: TimeInterval = SPARKS_SEND_UNDO_WINDOW_MS as TimeInterval / 1_000.0;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {})
    }

    fn actions(cx: &mut App) -> Entity<SparksActionsStore> {
        SparksActionsStore::shared(cx)
    }

    fn service(cx: &mut App) -> Entity<SparksServicesStore> {
        SparksServicesStore::shared(cx)
    }

    fn selected_filter(cx: &mut App) -> RiseLedgerHistoryFilter {
        Self::service(cx).read(cx).selected_filter
    }

    fn update_service(cx: &mut App, update: impl FnOnce(&mut SparksServicesStore)) {
        Self::service(cx).update(cx, |service, cx| {
            update(service);
            cx.notify();
        });
    }

    fn account_id(cx: &mut App) -> Option<String> {
        RiseSparksPresentation::shared(cx).read(cx).snapshot().account_id.clone()
    }

    pub fn open_wallet(&mut self, cx: &mut Context<Self>) {
        let filter = Self::selected_filter(cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.refresh_balance_action(false, cx);
            actions.refresh_earnings_action(false, cx);
            actions.load_history_action(filter, false, cx);
        });
    }

    pub fn wallet_moved(&mut self, cx: &mut Context<Self>) {
        let filter = Self::selected_filter(cx);
        Self::actions(cx).update(cx, |actions, cx| actions.load_history_action(filter, true, cx));
    }

    pub fn select_filter(&mut self, filter: RiseLedgerHistoryFilter, cx: &mut Context<Self>) {
        if Self::selected_filter(cx) == filter {
            return;
        }
        Self::update_service(cx, |service| service.selected_filter = filter);
        Self::actions(cx).update(cx, |actions, cx| actions.load_history_action(filter, false, cx));
    }

    pub fn retry_wallet(&mut self, cx: &mut Context<Self>) {
        let filter = Self::selected_filter(cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.refresh_balance_action(true, cx);
            actions.refresh_earnings_action(true, cx);
            actions.load_history_action(filter, true, cx);
        });
    }

    pub fn pull_to_refresh_wallet(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let filter = Self::selected_filter(cx);
        Self::actions(cx).update(cx, |actions, cx| actions.reload_wallet_action(filter, cx))
    }

    pub fn withdraw_received(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let (received_total, is_transferring) = {
            let service = service.read(cx);
            (service.received_total(), service.is_transferring())
        };
        if received_total <= 0 || is_transferring {
            return;
        }

        let transfer = Self::actions(cx).update(cx, |actions, cx| actions.transfer_earned_action(cx));
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = transfer.await;
            cx.update(|cx| match result {
                Ok(result) => {
                    let moved = result.transferred;
                    if moved <= 0 {
                        Notifier::send(
                            tr("sparks_withdraw_nothing"),
                            NotifierSendOptions { position: NotifierPosition::Bottom, ..NotifierSendOptions::default() },
                            cx,
                        );
                        return;
                    }
                    notification(NotificationKind::Success);
                    Notifier::send(
                        tr("sparks_withdraw_done").replace("{amount}", &RiseFormat::compact_count(moved)),
                        NotifierSendOptions {
                            r#type: NotifierType::Success,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                    let filter = Self::selected_filter(cx);
                    Self::actions(cx).update(cx, |actions, cx| actions.load_history_action(filter, true, cx));
                }
                Err(error) if error.is::<RiseSparksTransferError>() || error.is::<CancellationError>() => {}
                Err(error) => {
                    Notifier::send(
                        RiseSparksFailure::message(&error),
                        NotifierSendOptions {
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    pub fn load_more_history(&mut self, cx: &mut Context<Self>) {
        let filter = Self::selected_filter(cx);
        Self::actions(cx).update(cx, |actions, cx| actions.load_more_history_action(filter, cx));
    }

    pub fn prefetch_history(&mut self, filters: &[RiseLedgerHistoryFilter], cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| {
            for filter in filters {
                actions.load_history_action(*filter, false, cx);
            }
        });
    }

    pub fn open_purchase_sheet(&mut self, needed: Option<i64>, cx: &mut Context<Self>) {
        Self::update_service(cx, |service| {
            service.needed_amount = needed.filter(|needed| *needed > 0);
            service.purchase_phase = SparksPurchasePhase::Idle;
            service.is_purchase_sheet_presented = true;
        });
        self.load_offers(cx);
    }

    pub fn close_purchase_sheet(&mut self, cx: &mut Context<Self>) {
        Self::update_service(cx, |service| {
            service.is_purchase_sheet_presented = false;
            service.needed_amount = None;
            if matches!(service.purchase_phase, SparksPurchasePhase::Purchasing { .. }) {
                return;
            }
            service.purchase_phase = SparksPurchasePhase::Idle;
        });
    }

    pub fn dismiss_continue_on_phone(&mut self, cx: &mut Context<Self>) {
        Self::update_service(cx, |service| {
            if matches!(service.purchase_phase, SparksPurchasePhase::ContinueOnPhone { .. }) {
                service.purchase_phase = SparksPurchasePhase::Idle;
            }
        });
    }

    pub fn load_offers(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if service.read(cx).offers_phase == SparksOffersPhase::Loading {
            return;
        }
        if !service.read(cx).offers.is_empty() {
            Self::update_service(cx, |service| service.offers_phase = SparksOffersPhase::Ready);
            return;
        }
        Self::update_service(cx, |service| service.offers_phase = SparksOffersPhase::Loading);

        let offers = Self::actions(cx).update(cx, |actions, cx| actions.load_offers_action(cx));
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = offers.await;
            cx.update(|cx| {
                let phase = match &result {
                    Ok(offers) if offers.is_empty() => SparksOffersPhase::Failed(RiseSparksPurchases::failure_text(
                        tr("sparks_purchase_store_unavailable"),
                        format!(
                            "StoreKit returned no packs of {} ({})",
                            RiseSparksPurchases::product_ids().len(),
                            RiseSparksPurchases::environment().slug()
                        ),
                    )),
                    Ok(_) => SparksOffersPhase::Ready,
                    Err(error) => SparksOffersPhase::Failed(RiseSparksPurchases::failure_text(
                        tr("sparks_purchase_store_unavailable"),
                        format!("offers: {error}"),
                    )),
                };
                Self::update_service(cx, |service| {
                    if let Ok(offers) = result {
                        service.offers = offers;
                    }
                    service.offers_phase = phase;
                });
            });
        })
        .detach();
    }

    pub fn purchase(&mut self, offer: &RiseSparksPackOffer, cx: &mut Context<Self>) {
        if matches!(Self::service(cx).read(cx).purchase_phase, SparksPurchasePhase::Purchasing { .. }) {
            return;
        }
        let product_id = offer.product_id.clone();
        let offer_amount = offer.amount;
        Self::update_service(cx, |service| {
            service.purchase_phase = SparksPurchasePhase::Purchasing { product_id: product_id.clone() };
        });
        PurchaseCelebration::Sparks.prepare(cx);

        let outcome = Self::actions(cx).update(cx, |actions, cx| actions.purchase_action(&product_id, cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = outcome.await;
            let _ = this.update(cx, |store, cx| match outcome {
                RiseSparksPurchaseOutcome::Credited { amount, .. } => {
                    store.celebrate(if amount > 0 { amount } else { offer_amount }, cx);
                }
                RiseSparksPurchaseOutcome::Pending => {
                    Self::update_service(cx, |service| service.purchase_phase = SparksPurchasePhase::Pending);
                }
                RiseSparksPurchaseOutcome::Cancelled => {
                    Self::update_service(cx, |service| service.purchase_phase = SparksPurchasePhase::Idle);
                }
                RiseSparksPurchaseOutcome::Failed { message, .. } => {
                    Self::update_service(cx, |service| service.purchase_phase = SparksPurchasePhase::Failed(message));
                }
                RiseSparksPurchaseOutcome::ContinueOnPhone { product_id } => {
                    Self::update_service(cx, |service| {
                        service.purchase_phase = SparksPurchasePhase::ContinueOnPhone { product_id };
                    });
                }
            });
        })
        .detach();
    }

    pub fn close_success_sheet(&mut self, cx: &mut Context<Self>) {
        Self::update_service(cx, |service| service.is_success_sheet_presented = false);
    }

    pub fn open_send_sheet(
        &mut self,
        target: SparksTarget,
        view_only: bool,
        recipient_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if !view_only && self.refuses_recipient(recipient_id, cx) {
            return;
        }
        let sheet_target = target.clone();
        Self::update_service(cx, |service| {
            service.send_target = Some(sheet_target);
            service.send_view_only = view_only;
            service.send_amount = service.send_amount.min(service.maximum_send_amount().max(1)).max(1);
            service.top = None;
            service.top_phase = RiseSparksLoadPhase::Loading;
            service.is_send_sheet_presented = true;
        });
        let top = Self::actions(cx).update(cx, |actions, cx| {
            actions.refresh_balance_action(false, cx);
            actions.top_senders_action(&target, cx)
        });

        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = top.await;
            cx.update(|cx| {
                Self::update_service(cx, |service| {
                    if service.send_target.as_ref() != Some(&target) {
                        return;
                    }
                    match result {
                        Ok(top) => {
                            service.top = Some(top);
                            service.top_phase = RiseSparksLoadPhase::Loaded;
                        }
                        Err(error) => {
                            service.top_phase = RiseSparksLoadPhase::Failed(RiseSparksFailure::message(&error));
                        }
                    }
                });
            });
        })
        .detach();
    }

    pub fn close_send_sheet(&mut self, cx: &mut Context<Self>) {
        Self::update_service(cx, |service| {
            service.is_send_sheet_presented = false;
            service.send_target = None;
        });
    }

    pub fn set_send_amount(&mut self, amount: i64, cx: &mut Context<Self>) {
        Self::update_service(cx, |service| {
            service.send_amount = amount.min(RiseSparksLimits::tip_max_amount().max(1)).max(1);
        });
    }

    pub fn set_send_anonymous(&mut self, anonymous: bool, cx: &mut Context<Self>) {
        Self::update_service(cx, |service| service.send_anonymous = anonymous);
    }

    pub fn send_from_sheet(&mut self, cx: &mut Context<Self>) {
        let (target, view_only, amount, anonymous) = {
            let service = Self::service(cx).read(cx);
            (service.send_target.clone(), service.send_view_only, service.send_amount, service.send_anonymous)
        };
        let Some(target) = target else {
            return;
        };
        if view_only {
            return;
        }
        self.close_send_sheet(cx);
        if !self.send(&target, amount, anonymous, None, cx) {
            return;
        }
        let Some(account_id) = Self::account_id(cx) else {
            return;
        };

        cx.spawn(async move |_, cx: &mut AsyncApp| {
            cx.background_executor().timer(SparksBurst::AFTER_SHEET).await;
            cx.update(|cx| {
                if Self::account_id(cx).as_deref() != Some(account_id.as_str()) {
                    return;
                }
                let signal = Self::service(cx).read(cx).live_burst_signal(&target);
                if let Some(signal) = signal {
                    signal.update(cx, |signal, cx| signal.fire(cx));
                }
            });
        })
        .detach();
    }

    /// one tap is one spark, taps on the same target join its toast and restart the countdown
    /// nothing hits the server until the window closes
    pub fn tap(&mut self, target: &SparksTarget, recipient_id: Option<&str>, cx: &mut Context<Self>) -> bool {
        if self.refuses_recipient(recipient_id, cx) {
            return false;
        }
        self.send(target, 1, false, recipient_id, cx)
    }

    pub fn burst(&mut self, frame: Bounds<Pixels>, cx: &mut Context<Self>) {
        let width = f32::from(frame.size.width);
        if width <= 0.0 || !SparksBurst::allows_decorative_motion() {
            return;
        }
        let center = frame.center();
        let burst = SparksBurst {
            id: Uuid::new_v4(),
            center: Point { x: f32::from(center.x), y: f32::from(center.y) },
            side: width * SparksBurst::CANVAS_PER_ICON,
        };
        let burst_id = burst.id;
        Self::update_service(cx, |service| {
            service.bursts.push(burst);
            if service.bursts.len() > SparksBurst::MAXIMUM_IN_FLIGHT {
                let overflow = service.bursts.len() - SparksBurst::MAXIMUM_IN_FLIGHT;
                service.bursts.drain(..overflow);
            }
        });
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            cx.background_executor().timer(SparksBurst::LIFETIME).await;
            cx.update(|cx| {
                Self::update_service(cx, |service| service.bursts.retain(|burst| burst.id != burst_id));
            });
        })
        .detach();
    }

    fn refuses_recipient(&mut self, recipient_id: Option<&str>, cx: &mut Context<Self>) -> bool {
        let Some(recipient_id) = recipient_id else {
            return false;
        };
        if !UserServicesStore::shared(cx).read(cx).peer_restricts_viewer(recipient_id) {
            return false;
        }
        Notifier::send(
            tr("sparks_error_target_unavailable"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
        true
    }

    fn celebrate(&mut self, amount: i64, cx: &mut Context<Self>) {
        Self::update_service(cx, |service| {
            service.success_amount = amount;
            service.needed_amount = None;
            service.purchase_phase = SparksPurchasePhase::Idle;
            service.is_purchase_sheet_presented = false;
        });
        PurchaseCelebrationDirector::of(PurchaseCelebration::Sparks, cx).update(cx, |director, cx| director.arm(cx));
        Self::update_service(cx, |service| service.is_success_sheet_presented = true);
    }

    fn send(
        &mut self,
        target: &SparksTarget,
        amount: i64,
        anonymous: bool,
        recipient_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        if amount <= 0 {
            return false;
        }
        let Some(account_id) = Self::account_id(cx) else {
            return false;
        };

        let (has_balance, spendable_now, active) = {
            let service = Self::service(cx).read(cx);
            (service.has_balance(), service.spendable_now(), service.active_session().cloned())
        };
        let shortfall = amount - spendable_now;
        if has_balance && shortfall > 0 {
            self.open_purchase_sheet(Some(shortfall), cx);
            return false;
        }

        if let Some(mut session) = active
            && session.target == *target
            && session.anonymous == anonymous
            && session.account_id == account_id
            && let Some(toast_id) = session.toast_id
            && session.amount + amount <= RiseSparksLimits::tip_max_amount()
            && Notifier::extend_undo(
                toast_id,
                Self::toast_title(session.amount + amount),
                Self::UNDO_WINDOW_SECONDS,
                cx,
            )
        {
            session.amount += amount;
            Self::update_service(cx, |service| {
                service.sessions.insert(session.id, session);
            });
            Self::actions(cx).update(cx, |actions, cx| actions.draw_sparks_action(target, amount, cx)).detach();
            return true;
        }

        let session = SparksSendSession {
            id: Uuid::new_v4(),
            account_id,
            target: target.clone(),
            amount,
            idempotency_key: Uuid::new_v4().uuid_string().to_lowercase(),
            anonymous,
            toast_id: None,
            recipient_id: recipient_id.map(str::to_owned),
        };
        let session_id = session.id;
        Self::update_service(cx, |service| {
            service.sessions.insert(session_id, session);
            service.active_session_id = Some(session_id);
        });
        Self::actions(cx).update(cx, |actions, cx| actions.draw_sparks_action(target, amount, cx)).detach();

        let toast_id = Notifier::send_undo(
            Self::toast_title(amount),
            tr("common_cancel"),
            NotifierUndoOptions {
                position: NotifierPosition::Bottom,
                duration: Self::UNDO_WINDOW_SECONDS,
                ..NotifierUndoOptions::default()
            },
            move |cx: &mut App| {
                cx.defer(move |cx| {
                    SparksInteractionsStore::shared(cx).update(cx, |store, cx| store.commit(session_id, cx));
                });
            },
            move |cx: &mut App| {
                cx.defer(move |cx| {
                    SparksInteractionsStore::shared(cx).update(cx, |store, cx| store.cancel(session_id, cx));
                });
            },
            cx,
        );
        Self::update_service(cx, |service| {
            if let Some(current) = service.sessions.get_mut(&session_id) {
                current.toast_id = Some(toast_id);
            }
        });
        true
    }

    fn commit(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let Some(session) = Self::service(cx).read(cx).sessions.get(&session_id).cloned() else {
            return;
        };
        Self::update_service(cx, |service| {
            if service.active_session_id == Some(session_id) {
                service.active_session_id = None;
            }
        });

        let send = Self::actions(cx).update(cx, |actions, cx| actions.send_sparks_action(session.clone(), cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = send.await;
            let _ = this.update(cx, |store, cx| {
                Self::update_service(cx, |service| {
                    service.sessions.remove(&session_id);
                });
                let Err(error) = result else {
                    return;
                };
                if error.is::<CancellationError>() {
                    return;
                }
                if Self::account_id(cx).as_deref() != Some(session.account_id.as_str()) {
                    return;
                }
                store.explain(&error, Some(&session.target), session.recipient_id.as_deref(), cx);
            });
        })
        .detach();
    }

    fn cancel(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let mut removed = None;
        Self::update_service(cx, |service| {
            removed = service.sessions.remove(&session_id);
            if removed.is_some() && service.active_session_id == Some(session_id) {
                service.active_session_id = None;
            }
        });
        let Some(session) = removed else {
            return;
        };

        Self::actions(cx)
            .update(cx, |actions, cx| actions.draw_sparks_action(&session.target, -session.amount, cx))
            .detach();
    }

    fn explain(
        &mut self,
        error: &anyhow::Error,
        target: Option<&SparksTarget>,
        recipient_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let code = RiseSparksFailure::code(error).as_deref().and_then(ErrorCode::from_wire);
        if code == Some(ErrorCode::SparksInsufficientBalance) {
            let needed = RiseSparksFailure::integer_detail(ErrorParam::Shortfall.wire(), error);
            self.open_purchase_sheet(needed, cx);
            return;
        }
        if code == Some(ErrorCode::SparksDisabledForTarget)
            && let Some(name) = Self::direct_peer_name(target, cx)
        {
            if let Some(SparksTarget::Message { chat_id, .. }) = target {
                Self::service(cx).update(cx, |service, cx| service.note_direct_chat_refuses_sparks(chat_id, cx));
            }
            Notifier::send(
                tr("sparks_error_disabled_for_person").replace("{name}", &name),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            return;
        }
        if code == Some(ErrorCode::ContactBlockedByUser) {
            if let Some(recipient_id) = recipient_id {
                UserServicesStore::shared(cx).update(cx, |users, cx| users.note_peer_restricts_viewer(recipient_id, cx));
            }
            Notifier::send(
                tr("sparks_error_target_unavailable"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            return;
        }
        Notifier::send(
            RiseSparksFailure::message(error),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn direct_peer_name(target: Option<&SparksTarget>, cx: &mut App) -> Option<String> {
        let Some(SparksTarget::Message { chat_id, .. }) = target else {
            return None;
        };
        let chats = ChatServicesStore::shared(cx);
        let chat = chats.read(cx).chat_info(chat_id, cx)?;
        if chat.r#type != ChatType::Direct {
            return None;
        }
        let peer = chat.direct_peer_profile_name();
        if peer.is_empty() { None } else { Some(peer) }
    }

    fn toast_title(amount: i64) -> String {
        tr("sparks_sent_toast").replace("{amount}", &RiseFormat::compact_count(amount))
    }
}
