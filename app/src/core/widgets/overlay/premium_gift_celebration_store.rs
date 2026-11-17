use std::collections::HashSet;
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::foundation::foundation_date::RiseClock;
use rise_widgets::overlay::premium_gift_celebration_hooks::PremiumGiftCelebrationHooks;

use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::ui::purchase_celebration::{PurchaseCelebration, PurchaseCelebrationDirector};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_actions::SubscriptionActionsStore;
use crate::modules::user::stores::subscription::subscription_actions::subscription_wire_types::PremiumGiftNoticeDTO;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// a gift that turned premium on for this account, told as a gift once
/// server keeps the notice until the app acknowledges it so someone away sees it next open
pub struct PremiumGiftCelebrationStore {
    gift: Option<PremiumGiftCelebrationStoreGift>,
    check_task: Option<Task<()>>,
    celebrated: HashSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PremiumGiftCelebrationStoreGift {
    pub transaction_id: String,
    pub payer_user_id: String,
    pub payer_name: Option<String>,
    pub months: i64,
    pub tag_apology: Option<PremiumGiftCelebrationStoreTagApology>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PremiumGiftCelebrationStoreTagApology {
    pub old_tag: String,
    pub new_tag: String,
}

impl PremiumGiftCelebrationStore {
    const SETTLE: Duration = Duration::from_secs(2);
    const CONNECTION_WAIT: Duration = Duration::from_secs(20);
    const CONNECTION_POLL: Duration = Duration::from_millis(250);
    const NAME_POLL: Duration = Duration::from_millis(250);
    const NAME_POLL_ATTEMPTS: usize = 8;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            gift: None,
            check_task: None,
            celebrated: HashSet::new(),
        }
    }

    pub fn gift(&self) -> Option<&PremiumGiftCelebrationStoreGift> {
        self.gift.as_ref()
    }

    pub fn is_visible(&self) -> bool {
        self.gift.is_some()
    }

    /// asks the server once the socket is up, after a pause
    /// called on screen, on every foreground and when premium turns on
    pub fn schedule_check(&mut self, cx: &mut Context<Self>) {
        if self.gift.is_some() {
            return;
        }
        self.check_task = Some(cx.spawn(async move |_, cx| {
            let deadline = RiseClock::monotonic() + Self::CONNECTION_WAIT;
            while !cx.update(|cx| SaiWebSocketManager::shared(cx).read(cx).is_connected()) {
                if RiseClock::monotonic() >= deadline {
                    return;
                }
                cx.background_executor().timer(Self::CONNECTION_POLL).await;
            }
            cx.background_executor().timer(Self::SETTLE).await;
            cx.update(|cx| {
                SubscriptionActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.get_gift_notice_action(
                        |notice, cx| {
                            Self::shared(cx).update(cx, |store, cx| store.celebrate(notice, cx));
                        },
                        cx,
                    );
                });
            });
        }));
    }

    fn celebrate(&mut self, notice: Option<PremiumGiftNoticeDTO>, cx: &mut Context<Self>) {
        if self.gift.is_some() {
            return;
        }
        let Some(notice) = notice else {
            return;
        };
        if !notice.found {
            return;
        }
        let transaction_id = notice.transaction_id;
        let payer = notice.payer_user_id;
        let is_tag_apology = !notice.old_tag.is_empty() && !notice.new_tag.is_empty();
        if transaction_id.is_empty() || (payer.is_empty() && !is_tag_apology) || self.celebrated.contains(&transaction_id)
        {
            return;
        }
        self.celebrated.insert(transaction_id.clone());
        PurchaseCelebration::Premium.prepare(cx);
        let months = notice.months.max(1);
        if is_tag_apology {
            self.gift = Some(PremiumGiftCelebrationStoreGift {
                transaction_id,
                payer_user_id: payer,
                payer_name: None,
                months,
                tag_apology: Some(PremiumGiftCelebrationStoreTagApology {
                    old_tag: notice.old_tag.clone(),
                    new_tag: notice.new_tag.clone(),
                }),
            });
            cx.notify();
            Self::adopt_platform_tag(&notice.old_tag, &notice.new_tag, cx);
            PurchaseCelebrationDirector::premium(cx).update(cx, |director, cx| director.arm(cx));
            return;
        }
        cx.spawn(async move |this, cx| {
            let name = Self::payer_name(&payer, cx).await;
            let _ = this.update(cx, |store, cx| {
                if store.gift.is_some() {
                    return;
                }
                store.gift = Some(PremiumGiftCelebrationStoreGift {
                    transaction_id,
                    payer_user_id: payer,
                    payer_name: name,
                    months,
                    tag_apology: None,
                });
                cx.notify();
                PurchaseCelebrationDirector::premium(cx).update(cx, |director, cx| director.arm(cx));
            });
        })
        .detach();
    }

    fn adopt_platform_tag(old_tag: &str, new_tag: &str, cx: &mut App) {
        let auth = AuthServicesStore::shared(cx);
        let Some(mut profile) = auth.read(cx).profile().cloned() else {
            return;
        };
        let my_id = profile.id.clone();
        if profile.tag != new_tag {
            profile.tag = new_tag.to_owned();
            auth.update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
        }
        SaiWsResponseCache::remove(
            &SaiCache::new(["getMyProfile", my_id.as_str()]).key,
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        SaiWsResponseCache::remove(
            &UserServicesStore::profile_by_id_cache_key(&my_id, &my_id),
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        SaiWsResponseCache::remove(
            &UserServicesStore::profile_by_tag_cache_key(old_tag),
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_my_profile(cx));
    }

    /// who gave it, from what the device already knows
    /// else a profile read fills it in, without a name the sheet names only the period
    async fn payer_name(payer_id: &str, cx: &mut AsyncApp) -> Option<String> {
        if let Some(known) = cx.update(|cx| Self::known_name(payer_id, cx)) {
            return Some(known);
        }
        cx.update(|cx| {
            UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.refresh_foreign_profile_user_id(payer_id.to_owned(), cx);
            });
        });
        for _ in 0..Self::NAME_POLL_ATTEMPTS {
            cx.background_executor().timer(Self::NAME_POLL).await;
            if let Some(known) = cx.update(|cx| Self::known_name(payer_id, cx)) {
                return Some(known);
            }
        }
        None
    }

    fn known_name(payer_id: &str, cx: &mut App) -> Option<String> {
        PremiumGiftCelebrationHooks::person_name_or_default(payer_id, cx).filter(|name| !name.is_empty())
    }

    /// closing the sheet acknowledges every notice of the account
    /// the ones behind were gifts on top of this one
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.gift.is_none() {
            return;
        }
        self.gift = None;
        cx.notify();
        SubscriptionActionsStore::shared(cx).update(cx, |actions, cx| actions.ack_gift_notices_action(cx));
    }

    pub fn chat_with_giver(&self, cx: &mut App) -> Option<String> {
        let gift = self.gift.as_ref().filter(|gift| gift.tag_apology.is_none())?;
        let giver = User {
            id: gift.payer_user_id.clone(),
            name: gift.payer_name.clone().unwrap_or_default(),
            ..User::default()
        };
        let chat_id = ChatInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.direct_conversation_id(&giver, cx));
        (!chat_id.is_empty()).then_some(chat_id)
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.check_task = None;
        self.gift = None;
        self.celebrated.clear();
        cx.notify();
    }
}
