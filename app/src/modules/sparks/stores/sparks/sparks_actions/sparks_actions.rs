use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::foundation::foundation_date::Date;
use rise_engine::CancellationError;
use rise_i18n::tr;
use rise_canon::rise_ledger::HistoryFilter as RiseLedgerHistoryFilter;

use crate::core::lib::store_price::StorePriceBook;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::post::engine::rise_post_domain::RisePostActionExecutor;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::sparks::engine::rise_sparks_domain::RiseSparksDomainRegistry;
use crate::modules::sparks::engine::rise_sparks_engine_models::{
    RiseSparksLimits, RiseSparksTransferResponseDTO, RiseStoredSparksSendResult, RiseStoredSparksTop,
};
use crate::modules::sparks::engine::rise_sparks_purchases::{RiseSparksPackOffer, RiseSparksPurchaseOutcome};
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::{SparksSendSession, SparksTarget};

pub struct SparksActionsStore {}

impl SparksActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {})
    }

    pub fn refresh_balance_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(domain) = RiseSparksDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return;
        };

        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.repository.refresh_balance(force).await;
        })
        .detach();
    }

    pub fn refresh_earnings_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(domain) = RiseSparksDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return;
        };

        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.repository.refresh_earnings(force).await;
        })
        .detach();
    }

    pub fn reload_wallet_action(&mut self, filter: RiseLedgerHistoryFilter, cx: &mut Context<Self>) -> Task<()> {
        let Some(domain) = RiseSparksDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return Task::ready(());
        };
        let repository = domain.repository.clone();

        cx.spawn(async move |_, _: &mut AsyncApp| {
            futures::join!(
                repository.refresh_balance(true),
                repository.refresh_earnings(true),
                repository.load_history(filter, true),
            );
        })
    }

    pub fn transfer_earned_action(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseSparksTransferResponseDTO>> {
        let domain = RiseSparksDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx));
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = domain.await?;
            domain.repository.transfer_earned().await
        })
    }

    pub fn load_history_action(&mut self, filter: RiseLedgerHistoryFilter, force: bool, cx: &mut Context<Self>) {
        let Some(domain) = RiseSparksDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return;
        };

        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.repository.load_history(filter, force).await;
        })
        .detach();
    }

    pub fn load_more_history_action(&mut self, filter: RiseLedgerHistoryFilter, cx: &mut Context<Self>) {
        let Some(domain) = RiseSparksDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return;
        };

        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.repository.load_more_history(filter).await;
        })
        .detach();
    }

    pub fn load_offers_action(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Vec<RiseSparksPackOffer>>> {
        let domain = RiseSparksDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx));
        let book = StorePriceBook::shared(cx);
        StorePriceBook::refresh(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let domain = domain.await?;
            let mut offers = domain.purchases.offers().await?;
            cx.update(|cx| {
                let book = book.read(cx);
                for offer in &mut offers {
                    offer.display_price = book.text(&offer.product_id);
                }
            });
            Ok(offers)
        })
    }

    pub fn purchase_action(&mut self, product_id: &str, cx: &mut Context<Self>) -> Task<RiseSparksPurchaseOutcome> {
        let domain = RiseSparksDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx));
        let product_id = product_id.to_owned();
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseSparksPurchaseOutcome::Failed { message: tr("sparks_purchase_failed"), code: None };
            };

            let outcome = domain.purchases.purchase(&product_id).await;
            if let RiseSparksPurchaseOutcome::Credited { spendable_after: Some(spendable), .. } = &outcome {
                domain.repository.apply_spendable_after(*spendable, Self::now_milliseconds()).await;
                domain.repository.invalidate_history().await;
            }
            outcome
        })
    }

    pub fn draw_sparks_action(&mut self, target: &SparksTarget, delta: i64, cx: &mut Context<Self>) -> Task<()> {
        match target {
            SparksTarget::Message { message_id, .. } => {
                let message_id = message_id.clone();
                cx.spawn(async move |_, cx: &mut AsyncApp| {
                    let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                        return;
                    };
                    let _ = domain.messages.apply_sparks_delta(&message_id, delta).await;
                })
            }
            SparksTarget::Post { post_id } => {
                PostServicesStore::shared(cx).update(cx, |posts, cx| posts.apply_sparks_delta(post_id, delta, cx));
                let domain = RisePostActionExecutor::require_domain(cx);
                let post_id = post_id.clone();
                cx.spawn(async move |_, _: &mut AsyncApp| {
                    let domain = match domain.await {
                        Ok(domain) => domain,
                        Err(error) => {
                            x398_mark(|| format!("x398 draw require_domain err post={post_id} {error:#}"));
                            return;
                        }
                    };
                    let outcome = domain.apply_post_sparks_delta(&post_id, delta).await;
                    x398_mark(|| format!("x398 draw post={post_id} delta={delta} ok={:?}", outcome.as_ref().err()));
                })
            }
        }
    }

    pub fn send_sparks_action(
        &mut self,
        session: SparksSendSession,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseStoredSparksSendResult>> {
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match &session.target {
                SparksTarget::Message { message_id, .. } => {
                    let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                    if domain.account_id != session.account_id {
                        return Err(CancellationError.into());
                    }
                    domain
                        .messages
                        .send_sparks(message_id, session.amount, session.anonymous, &session.idempotency_key)
                        .await?
                }
                SparksTarget::Post { post_id } => {
                    let domain = cx.update(RisePostActionExecutor::require_domain).await?;
                    if domain.account_id != session.account_id {
                        return Err(CancellationError.into());
                    }
                    let result = match domain
                        .send_post_sparks(post_id, session.amount, session.anonymous, &session.idempotency_key)
                        .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            cx.update(|cx| {
                                PostServicesStore::shared(cx)
                                    .update(cx, |posts, cx| posts.apply_sparks_delta(post_id, -session.amount, cx));
                            });
                            return Err(error);
                        }
                    };
                    cx.update(|cx| {
                        PostServicesStore::shared(cx).update(cx, |posts, cx| {
                            posts.apply_sparks_sent(
                                post_id,
                                session.amount,
                                Some(result.sparks_total),
                                Some(result.my_sparks),
                                cx,
                            )
                        });
                    });
                    result
                }
            };

            if let Some(spendable) = result.spendable_after {
                let wallet = cx.update(|cx| RiseSparksDomainRegistry::shared(cx).read(cx).active().cloned());
                if let Some(wallet) = wallet.filter(|wallet| wallet.account_id == session.account_id) {
                    wallet.repository.apply_spendable_after(spendable, Self::now_milliseconds()).await;
                    wallet.repository.invalidate_history().await;
                }
            }
            Ok(result)
        })
    }

    pub fn top_senders_action(
        &mut self,
        target: &SparksTarget,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseStoredSparksTop>> {
        match target {
            SparksTarget::Message { message_id, .. } => {
                let message_id = message_id.clone();
                cx.spawn(async move |_, cx: &mut AsyncApp| {
                    let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                    domain.messages.sparks_top(&message_id, RiseSparksLimits::top_senders_page_size()).await
                })
            }
            SparksTarget::Post { post_id } => {
                let domain = RisePostActionExecutor::require_domain(cx);
                let post_id = post_id.clone();
                cx.spawn(async move |_, _: &mut AsyncApp| {
                    let domain = domain.await?;
                    domain.post_sparks_top(&post_id, RiseSparksLimits::top_senders_page_size()).await
                })
            }
        }
    }

    fn now_milliseconds() -> i64 {
        (Date::now().time_interval_since_1970() * 1_000.0) as i64
    }
}

fn x398_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X251", "{}", stage());
}
