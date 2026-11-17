use std::collections::HashMap;

use gpui::{App, Context, Entity, EntityId, Global, WeakEntity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{ACCOUNT_STATE_LEGACY_MIGRATED, ACCOUNT_STATE_OWNER};
use rise_widgets::overlay::blocking_loader_store::BlockingLoaderStore;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceRegistry;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredOutboundCoordinator;
use crate::core::lib::sai::core::websocket::sai_ws_response_debt_coordinator::SaiWsResponseDebtCoordinator;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_memory_cache::SaiFetchMemoryCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_body_deep_compare::SaiWsLastOutboundBodyRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_cache_refresher_coordinator::SaiWsCacheRefresherCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_coalescing::SaiWsCoalescingRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_coordinator::SaiWsOptimisticCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_outbound_cancellation::SaiWsOutboundCancellation;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_coordinator::SaiWsScreenRefreshCoordinator;
use crate::core::lib::sai::sai::Sai;
use crate::core::media::media_save_coordinator::MediaSaveCoordinator;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::rise_media_range_policy::RiseMediaRangePolicy;
use crate::core::media::rise_media_stream_session::RiseMediaStreamRegistry;
use crate::core::media::rise_media_transfer_hub::RiseMediaTransferHub;
use crate::core::media::url_cache::URLCache;
use crate::core::media::video_warm_start_prefetch::VideoWarmStartPrefetch;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::push_notifications::push_deep_link_store::PushDeepLinkStore;
use crate::core::restriction::restriction_store::RestrictionStore;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::storage::rise_chat_media_ledger::RiseChatMediaLedger;
use crate::core::ui::clever_image_disk_cache::{CleverImageDiskCache, CleverImageDiskPrefetch};
use crate::core::ui::custom_emoji_resource_warmup::CustomEmojiResourceWarmup;
use crate::core::ui::emoji_picker_ui::emoji_picker_recents_store::EmojiPickerRecentsStore;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::lottie_sticker::lottie_sticker_prefetch::LottieStickerPrefetch;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierStore, NotifierType,
};
use crate::core::widgets::overlay::achievement_earned_store::AchievementEarnedStore;
use crate::core::widgets::overlay::premium_gift_celebration_store::PremiumGiftCelebrationStore;
use crate::core::widgets::overlay::premium_promo_store::PremiumPromoStore;
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::core::widgets::overlay::subscription_success_store::SubscriptionSuccessStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::components::web_login_approval::web_login_approval_store::WebLoginApprovalStore;
use crate::modules::auth::services::account_state_reset_hooks::{
    account_state_reset_analytics_hooks, account_state_reset_bot_hooks, account_state_reset_chat_hooks,
    account_state_reset_chat_ui_hooks, account_state_reset_comment_hooks, account_state_reset_core_widgets_hooks,
    account_state_reset_diagnostics_hooks, account_state_reset_gifts_hooks, account_state_reset_media_picker_hooks,
    account_state_reset_moderation_hooks, account_state_reset_music_hooks, account_state_reset_notify_hooks,
    account_state_reset_post_hooks, account_state_reset_post_list_hooks, account_state_reset_presence_hooks,
    account_state_reset_rtc_hooks, account_state_reset_search_hooks, account_state_reset_session_hooks,
    account_state_reset_shared_element_hooks, account_state_reset_shorts_hooks, account_state_reset_sparks_hooks,
    account_state_reset_storage_hooks, account_state_reset_story_hooks, account_state_reset_user_hooks,
    account_state_reset_user_ui_hooks, account_state_reset_vacancy_hooks,
};
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::stickers::components::sticker_pack_add_cell::StickerAssetComposerStore;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;
use crate::modules::user::shared::mention_resolver::MentionTapResolver;

/// something that can be reset when the account state resets
pub trait AccountResettable: Sized + 'static {
    fn reset_account_state(&mut self, cx: &mut Context<Self>);
}

/// weak handle that resets the store while it lives and says whether it still does
struct WeakAccountResettable {
    value: Box<dyn Fn(&mut App) -> bool>,
}

impl WeakAccountResettable {
    fn new<T: AccountResettable>(value: WeakEntity<T>) -> Self {
        Self {
            value: Box::new(move |cx| value.update(cx, |store, cx| store.reset_account_state(cx)).is_ok()),
        }
    }
}

/// registry of everything that resets with the account state
#[derive(Default)]
pub struct AccountStateResetRegistry {
    stores: HashMap<EntityId, WeakAccountResettable>,
    did_register_stores: bool,
}

impl Global for AccountStateResetRegistry {}

impl AccountStateResetRegistry {
    pub fn register<T: AccountResettable>(store: &Entity<T>, cx: &mut App) {
        let id = store.entity_id();
        let weak = WeakAccountResettable::new(store.downgrade());
        cx.default_global::<Self>().stores.insert(id, weak);
    }

    pub fn reset_all(cx: &mut App) {
        let stores = std::mem::take(&mut cx.default_global::<Self>().stores);
        let mut alive = HashMap::with_capacity(stores.len());
        for (id, store) in stores {
            if (store.value)(cx) {
                alive.insert(id, store);
            }
        }
        let registry = cx.default_global::<Self>();
        for (id, store) in alive {
            registry.stores.entry(id).or_insert(store);
        }
    }
}

/// why the account state is being reset
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccountStateResetReason {
    UserLogout,
    ForcedLogout,
    AccountChanged,
    LegacyCacheMigration,
}

impl AccountStateResetReason {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::UserLogout => "userLogout",
            Self::ForcedLogout => "forcedLogout",
            Self::AccountChanged => "accountChanged",
            Self::LegacyCacheMigration => "legacyCacheMigration",
        }
    }
}

pub enum AccountStateResetCoordinator {}

impl AccountStateResetCoordinator {
    const UNKNOWN_AUTHENTICATED_OWNER: &'static str = "__unknown_authenticated_owner__";

    pub fn prepare_for_activation(raw_user_id: &str, transition_already_reset: bool, cx: &mut App) {
        let user_id = trimming_whitespaces_and_newlines(raw_user_id).to_owned();
        let defaults = UserDefaults::standard();
        if user_id.is_empty() {
            Self::hard_reset(AccountStateResetReason::AccountChanged, true, false, cx);
            defaults.set(ACCOUNT_STATE_OWNER, Self::UNKNOWN_AUTHENTICATED_OWNER);
            return;
        }

        if !defaults.bool(ACCOUNT_STATE_LEGACY_MIGRATED) {
            Self::hard_reset(AccountStateResetReason::LegacyCacheMigration, false, false, cx);
            defaults.set(ACCOUNT_STATE_LEGACY_MIGRATED, true);
        }

        let previous = defaults.string(ACCOUNT_STATE_OWNER);
        if let Some(previous) = previous
            && previous != user_id
            && !transition_already_reset
        {
            Self::hard_reset(AccountStateResetReason::AccountChanged, false, true, cx);
        }
        defaults.set(ACCOUNT_STATE_OWNER, user_id.as_str());
        SaiAccountCacheScope::activate(&user_id);
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if let Some(user) = account_state_reset_user_hooks() {
            (user.activate_account_state)(cx);
        }
        if let Some(rtc) = account_state_reset_rtc_hooks() {
            (rtc.authenticated_account_did_activate)(&user_id, cx);
        }
        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        let failed_user_id = user_id.clone();
        cx.spawn(async move |cx| {
            if let Err(error) = prepare.await {
                cx.update(|cx| {
                    if let Some(user) = account_state_reset_user_hooks() {
                        (user.rise_user_presentation_report_theme_settings_failure)(
                            &error,
                            &failed_user_id,
                            account_epoch.as_deref(),
                            cx,
                        );
                    }
                });
            }
        })
        .detach();
        if let Some(chat) = account_state_reset_chat_hooks() {
            (chat.activate_account_state)(cx);
        }
        let transfer_hub = RiseMediaTransferHub::shared();
        RiseAccountRuntime::current().spawn(async move {
            transfer_hub.prewarm().await;
        });
        if let Some(music) = account_state_reset_music_hooks() {
            (music.activate_account_state)(cx);
        }
        if let Some(vacancy) = account_state_reset_vacancy_hooks() {
            (vacancy.activate_account_state)(cx);
        }
        if let Some(media_picker) = account_state_reset_media_picker_hooks() {
            (media_picker.activate_account_state)(cx);
        }
        EmojiPickerRecentsStore::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
        if let Some(analytics) = account_state_reset_analytics_hooks() {
            (analytics.activate_account_state)(cx);
        }
    }

    pub fn hard_reset(
        reason: AccountStateResetReason,
        clear_owner: bool,
        preserve_account_disk: bool,
        cx: &mut App,
    ) {
        Self::register_stores_if_needed(cx);
        NotifierStore::shared(cx).update(cx, |store, cx| store.commit_pending_undo_actions(cx));
        if let Some(rtc) = account_state_reset_rtc_hooks() {
            (rtc.fence_account_transition)(reason.raw_value(), cx);
        }
        if let Some(chat) = account_state_reset_chat_hooks() {
            (chat.fence_account_transition)(cx);
        }
        if let Some(post) = account_state_reset_post_hooks() {
            (post.fence_account_transition)(cx);
        }
        if let Some(notify) = account_state_reset_notify_hooks() {
            (notify.fence_account_transition)(cx);
        }
        SaiDebouncer::cancel_all(cx);
        let interrupted_local_music_uploads = reason == AccountStateResetReason::AccountChanged
            && account_state_reset_music_hooks()
                .is_some_and(|music| (music.music_local_track_draft_store_interrupt_for_account_change)(cx));
        Self::reset_registered_stores(preserve_account_disk, cx);

        if interrupted_local_music_uploads {
            Notifier::send(
                tr("music_message_import_account_changed"),
                NotifierSendOptions {
                    r#type: NotifierType::Info,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }

        SaiWsResponseDebtCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.purge_for_account_transition(cx));
        SaiWsDeferredOutboundCoordinator::shared(cx)
            .update(cx, |coordinator, cx| coordinator.purge_for_account_transition(cx));
        SaiWsPendingRegistry::shared(cx).update(cx, |registry, cx| registry.reset(cx));
        SaiWsOptimisticCoordinator::reset(cx);
        SaiWsCoalescingRegistry::clear_all(cx);
        SaiWsLastOutboundBodyRegistry::clear_all();
        SaiWsOutboundCancellation::clear_all();
        SaiWebSocketManager::shared(cx).update(cx, |manager, cx| manager.disconnect(cx));
        RiseEngineAppRuntime::shared(cx)
            .update(cx, |runtime, cx| runtime.close_current_engine(cx))
            .detach();

        if preserve_account_disk {
            SaiWsResponseCache::clear_memory(cx);
        } else {
            SaiWsResponseCache::clear_all(cx);
        }
        SaiFetchMemoryCache::clear_all(cx);
        SaiSliceRegistry::reset_all(cx);
        SaiWsScreenRefreshCoordinator::reset_account_state(cx);
        SaiWsCacheRefresherCoordinator::reset_account_state(cx);
        CleverImageDiskPrefetch::reset_account_state();
        LottieStickerPrefetch::reset_account_state();
        CustomEmojiResourceWarmup::shared(cx).update(cx, |warmup, cx| warmup.reset_account_state(cx));
        if let Some(user) = account_state_reset_user_hooks() {
            (user.premium_badge_picker_popover_dismiss)(cx);
        }
        VideoWarmStartPrefetch::reset_account_state();
        if let Some(story) = account_state_reset_story_hooks() {
            (story.hard_reset_media_caches)(cx);
        }
        RemoteMediaDiskCache::reset_account_state();
        RiseMediaRangePolicy::reset_for_account_change();
        let stream_registry = RiseMediaStreamRegistry::shared();
        let transfer_hub = RiseMediaTransferHub::shared();
        RiseAccountRuntime::current().spawn(async move {
            stream_registry.reset_account_state().await;
            transfer_hub.reset_account_state().await;
        });
        MediaViewerImageMemoryCache::clear_all();
        if let Some(chat_ui) = account_state_reset_chat_ui_hooks() {
            (chat_ui.hard_reset_render_caches)(cx);
        }
        if let Some(post_list) = account_state_reset_post_list_hooks() {
            (post_list.purge_all)(cx);
        }
        URLCache::shared().remove_all_cached_responses();

        if clear_owner {
            UserDefaults::standard().remove_object(ACCOUNT_STATE_OWNER);
            SaiAccountCacheScope::clear_active_account();
        }
        let reason = reason.raw_value();
        Sai::log("Auth", move || format!("account state reset reason={reason}"), SaiLogKind::Cache, cx);
    }

    pub fn remove_persisted_cache(user_id: &str, cx: &mut App) {
        RiseEngineAppRuntime::shared(cx)
            .update(cx, |runtime, cx| runtime.remove_persisted_account(user_id, cx))
            .detach();
        SaiWsResponseCache::remove_persisted_account(user_id, cx);
        if let Some(chat) = account_state_reset_chat_hooks() {
            (chat.remove_persisted_account)(user_id, cx);
        }
        if let Some(music) = account_state_reset_music_hooks() {
            (music.remove_persisted_account)(user_id, cx);
        }
        MusicPlaybackDiskCache::remove_persisted_account(user_id);
        if let Some(vacancy) = account_state_reset_vacancy_hooks() {
            (vacancy.remove_persisted_account)(user_id, cx);
        }
        CleverImageDiskCache::remove_persisted_account(user_id);
        RiseChatMediaLedger::remove_persisted_account(user_id);
        RemoteMediaDiskCache::remove_persisted_account(user_id);
        if let Some(media_picker) = account_state_reset_media_picker_hooks() {
            (media_picker.remove_persisted_account)(user_id, cx);
        }
        EmojiPickerRecentsStore::shared(cx).read(cx).remove_persisted_account(user_id);
        if let Some(gifts) = account_state_reset_gifts_hooks() {
            (gifts.remove_persisted_account)(user_id, cx);
        }
        if let Some(analytics) = account_state_reset_analytics_hooks() {
            (analytics.remove_persisted_account)(user_id, cx);
        }
    }

    fn reset_registered_stores(preserve_account_disk: bool, cx: &mut App) {
        AccountStateResetRegistry::reset_all(cx);
        HoldContextMenuPresenter::reset_account_state(cx);
        if let Some(chat) = account_state_reset_chat_hooks() {
            (chat.reset_account_state)(preserve_account_disk, cx);
        }
        if let Some(chat_ui) = account_state_reset_chat_ui_hooks() {
            (chat_ui.reset_account_state)(cx);
        }
        if let Some(post) = account_state_reset_post_hooks() {
            (post.reset_account_state)(cx);
        }
        if let Some(shorts) = account_state_reset_shorts_hooks() {
            (shorts.reset_account_state)(cx);
        }
        if let Some(comment) = account_state_reset_comment_hooks() {
            (comment.reset_account_state)(cx);
        }
        if let Some(user) = account_state_reset_user_hooks() {
            (user.reset_account_state)(cx);
        }
        if let Some(user_ui) = account_state_reset_user_ui_hooks() {
            (user_ui.reset_account_state)(cx);
        }
        if let Some(session) = account_state_reset_session_hooks() {
            (session.reset_account_state)(cx);
        }
        if let Some(notify) = account_state_reset_notify_hooks() {
            (notify.reset_account_state)(cx);
        }
        if let Some(presence) = account_state_reset_presence_hooks() {
            (presence.reset_account_state)(cx);
        }
        if let Some(story) = account_state_reset_story_hooks() {
            (story.reset_account_state)(cx);
        }
        if let Some(vacancy) = account_state_reset_vacancy_hooks() {
            (vacancy.reset_account_state)(cx);
        }
        if let Some(moderation) = account_state_reset_moderation_hooks() {
            (moderation.reset_account_state)(cx);
        }
        if let Some(analytics) = account_state_reset_analytics_hooks() {
            (analytics.reset_account_state)(cx);
        }
        if let Some(bot) = account_state_reset_bot_hooks() {
            (bot.reset_account_state)(cx);
        }
        if let Some(search) = account_state_reset_search_hooks() {
            (search.reset_account_state)(cx);
        }
        if let Some(core_widgets) = account_state_reset_core_widgets_hooks() {
            (core_widgets.reset_account_state)(cx);
        }
        if let Some(music) = account_state_reset_music_hooks() {
            (music.reset_account_state)(cx);
        }
        if let Some(media_picker) = account_state_reset_media_picker_hooks() {
            (media_picker.reset_account_state)(cx);
        }
        if let Some(diagnostics) = account_state_reset_diagnostics_hooks() {
            (diagnostics.reset_account_state)(cx);
        }
        if let Some(sparks) = account_state_reset_sparks_hooks() {
            (sparks.reset_account_state)(cx);
        }
        if let Some(rtc) = account_state_reset_rtc_hooks() {
            (rtc.reset_account_state)(cx);
        }
        if let Some(storage) = account_state_reset_storage_hooks() {
            (storage.reset_account_state)(cx);
        }
        if let Some(shared_element) = account_state_reset_shared_element_hooks() {
            (shared_element.reset_account_state)(cx);
        }
    }

    fn register_stores_if_needed(cx: &mut App) {
        let registry = cx.default_global::<AccountStateResetRegistry>();
        if registry.did_register_stores {
            return;
        }
        registry.did_register_stores = true;
        AccountStateResetRegistry::register(&StickersServicesStore::shared(cx), cx);
        AccountStateResetRegistry::register(&ShareSheetStore::shared(cx), cx);
        AccountStateResetRegistry::register(&StickerPackSheetStore::shared(cx), cx);
        AccountStateResetRegistry::register(&StickerAssetComposerStore::shared(cx), cx);
        AccountStateResetRegistry::register(&WebLoginApprovalStore::shared(cx), cx);
        AccountStateResetRegistry::register(&MentionTapResolver::shared(cx), cx);
        AccountStateResetRegistry::register(&MediaSaveCoordinator::shared(cx), cx);
        AccountStateResetRegistry::register(&BlockingLoaderStore::shared(cx), cx);
        AccountStateResetRegistry::register(&NotifierStore::shared(cx), cx);
        AccountStateResetRegistry::register(&SubscriptionSuccessStore::shared(cx), cx);
        AccountStateResetRegistry::register(&AchievementEarnedStore::shared(cx), cx);
        AccountStateResetRegistry::register(&PremiumUpsellStore::shared(cx), cx);
        AccountStateResetRegistry::register(&RestrictionStore::shared(cx), cx);
        AccountStateResetRegistry::register(&PremiumGiftCelebrationStore::shared(cx), cx);
        AccountStateResetRegistry::register(&PremiumPromoStore::shared(cx), cx);
        AccountStateResetRegistry::register(&EmojiPickerRecentsStore::shared(cx), cx);
        AccountStateResetRegistry::register(&PushDeepLinkStore::shared(cx), cx);
    }
}

macro_rules! account_resettable {
    ($($store:ty),* $(,)?) => {
        $(
            impl AccountResettable for $store {
                fn reset_account_state(&mut self, cx: &mut Context<Self>) {
                    <$store>::reset_account_state(self, cx);
                }
            }
        )*
    };
}

account_resettable!(
    StickersServicesStore,
    ShareSheetStore,
    StickerPackSheetStore,
    StickerAssetComposerStore,
    WebLoginApprovalStore,
    MentionTapResolver,
    MediaSaveCoordinator,
    BlockingLoaderStore,
    NotifierStore,
    SubscriptionSuccessStore,
    AchievementEarnedStore,
    PremiumUpsellStore,
    RestrictionStore,
    PremiumGiftCelebrationStore,
    PremiumPromoStore,
    EmojiPickerRecentsStore,
    PushDeepLinkStore,
);
