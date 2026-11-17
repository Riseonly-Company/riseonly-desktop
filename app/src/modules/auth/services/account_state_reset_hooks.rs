use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

macro_rules! account_state_reset_slot {
    ($hooks:ident, $slot:ident, $set:ident, $get:ident) => {
        pub(crate) static $slot: FeatureHookSlot<$hooks> = FeatureHookSlot::new(stringify!($slot));

        /// installs the slot as a whole, first installer wins and a second call is ignored
        pub fn $set(hooks: $hooks) {
            let _ = $slot.set(hooks);
        }

        pub fn $get() -> Option<&'static $hooks> {
            $slot.get()
        }
    };
}

pub struct AccountStateResetUserHooks {
    pub activate_account_state: fn(cx: &mut App),
    pub rise_user_presentation_report_theme_settings_failure:
        fn(error: &anyhow::Error, account_id: &str, account_epoch: Option<&str>, cx: &mut App),
    pub reset_account_state: fn(cx: &mut App),
    pub premium_badge_picker_popover_dismiss: fn(cx: &mut App),
}

pub struct AccountStateResetUserUiHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetChatHooks {
    pub activate_account_state: fn(cx: &mut App),
    pub fence_account_transition: fn(cx: &mut App),
    pub reset_account_state: fn(preserve_account_disk: bool, cx: &mut App),
    pub remove_persisted_account: fn(user_id: &str, cx: &mut App),
}

pub struct AccountStateResetChatUiHooks {
    pub reset_account_state: fn(cx: &mut App),
    pub hard_reset_render_caches: fn(cx: &mut App),
}

pub struct AccountStateResetPostHooks {
    pub fence_account_transition: fn(cx: &mut App),
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetPostListHooks {
    pub purge_all: fn(cx: &mut App),
}

pub struct AccountStateResetMusicHooks {
    pub activate_account_state: fn(cx: &mut App),
    pub music_local_track_draft_store_interrupt_for_account_change: fn(cx: &mut App) -> bool,
    pub reset_account_state: fn(cx: &mut App),
    pub remove_persisted_account: fn(user_id: &str, cx: &mut App),
}

pub struct AccountStateResetRtcHooks {
    pub authenticated_account_did_activate: fn(user_id: &str, cx: &mut App),
    pub fence_account_transition: fn(reason: &str, cx: &mut App),
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetNotifyHooks {
    pub fence_account_transition: fn(cx: &mut App),
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetCommentHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetSearchHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetSessionHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetPresenceHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetAnalyticsHooks {
    pub activate_account_state: fn(cx: &mut App),
    pub remove_persisted_account: fn(user_id: &str, cx: &mut App),
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetVacancyHooks {
    pub activate_account_state: fn(cx: &mut App),
    pub reset_account_state: fn(cx: &mut App),
    pub remove_persisted_account: fn(user_id: &str, cx: &mut App),
}

pub struct AccountStateResetSparksHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetGiftsHooks {
    pub remove_persisted_account: fn(user_id: &str, cx: &mut App),
}

pub struct AccountStateResetStorageHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetSharedElementHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetShortsHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetStoryHooks {
    pub reset_account_state: fn(cx: &mut App),
    pub hard_reset_media_caches: fn(cx: &mut App),
}

pub struct AccountStateResetBotHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetModerationHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetDiagnosticsHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub struct AccountStateResetMediaPickerHooks {
    pub activate_account_state: fn(cx: &mut App),
    pub reset_account_state: fn(cx: &mut App),
    pub remove_persisted_account: fn(user_id: &str, cx: &mut App),
}

pub struct AccountStateResetCoreWidgetsHooks {
    pub reset_account_state: fn(cx: &mut App),
}

account_state_reset_slot!(AccountStateResetUserHooks, USER, set_account_state_reset_user_hooks, account_state_reset_user_hooks);
account_state_reset_slot!(AccountStateResetUserUiHooks, USER_UI, set_account_state_reset_user_ui_hooks, account_state_reset_user_ui_hooks);
account_state_reset_slot!(AccountStateResetChatHooks, CHAT, set_account_state_reset_chat_hooks, account_state_reset_chat_hooks);
account_state_reset_slot!(AccountStateResetChatUiHooks, CHAT_UI, set_account_state_reset_chat_ui_hooks, account_state_reset_chat_ui_hooks);
account_state_reset_slot!(AccountStateResetPostHooks, POST, set_account_state_reset_post_hooks, account_state_reset_post_hooks);
account_state_reset_slot!(AccountStateResetPostListHooks, POST_LIST, set_account_state_reset_post_list_hooks, account_state_reset_post_list_hooks);
account_state_reset_slot!(AccountStateResetMusicHooks, MUSIC, set_account_state_reset_music_hooks, account_state_reset_music_hooks);
account_state_reset_slot!(AccountStateResetRtcHooks, RTC, set_account_state_reset_rtc_hooks, account_state_reset_rtc_hooks);
account_state_reset_slot!(AccountStateResetNotifyHooks, NOTIFY, set_account_state_reset_notify_hooks, account_state_reset_notify_hooks);
account_state_reset_slot!(AccountStateResetCommentHooks, COMMENT, set_account_state_reset_comment_hooks, account_state_reset_comment_hooks);
account_state_reset_slot!(AccountStateResetSearchHooks, SEARCH, set_account_state_reset_search_hooks, account_state_reset_search_hooks);
account_state_reset_slot!(AccountStateResetSessionHooks, SESSION, set_account_state_reset_session_hooks, account_state_reset_session_hooks);
account_state_reset_slot!(AccountStateResetPresenceHooks, PRESENCE, set_account_state_reset_presence_hooks, account_state_reset_presence_hooks);
account_state_reset_slot!(AccountStateResetAnalyticsHooks, ANALYTICS, set_account_state_reset_analytics_hooks, account_state_reset_analytics_hooks);
account_state_reset_slot!(AccountStateResetVacancyHooks, VACANCY, set_account_state_reset_vacancy_hooks, account_state_reset_vacancy_hooks);
account_state_reset_slot!(AccountStateResetSparksHooks, SPARKS, set_account_state_reset_sparks_hooks, account_state_reset_sparks_hooks);
account_state_reset_slot!(AccountStateResetGiftsHooks, GIFTS, set_account_state_reset_gifts_hooks, account_state_reset_gifts_hooks);
account_state_reset_slot!(AccountStateResetStorageHooks, STORAGE, set_account_state_reset_storage_hooks, account_state_reset_storage_hooks);
account_state_reset_slot!(
    AccountStateResetSharedElementHooks,
    SHARED_ELEMENT,
    set_account_state_reset_shared_element_hooks,
    account_state_reset_shared_element_hooks
);
account_state_reset_slot!(AccountStateResetShortsHooks, SHORTS, set_account_state_reset_shorts_hooks, account_state_reset_shorts_hooks);
account_state_reset_slot!(AccountStateResetStoryHooks, STORY, set_account_state_reset_story_hooks, account_state_reset_story_hooks);
account_state_reset_slot!(AccountStateResetBotHooks, BOT, set_account_state_reset_bot_hooks, account_state_reset_bot_hooks);
account_state_reset_slot!(
    AccountStateResetModerationHooks,
    MODERATION,
    set_account_state_reset_moderation_hooks,
    account_state_reset_moderation_hooks
);
account_state_reset_slot!(
    AccountStateResetDiagnosticsHooks,
    DIAGNOSTICS,
    set_account_state_reset_diagnostics_hooks,
    account_state_reset_diagnostics_hooks
);
account_state_reset_slot!(
    AccountStateResetMediaPickerHooks,
    MEDIA_PICKER,
    set_account_state_reset_media_picker_hooks,
    account_state_reset_media_picker_hooks
);
account_state_reset_slot!(
    AccountStateResetCoreWidgetsHooks,
    CORE_WIDGETS,
    set_account_state_reset_core_widgets_hooks,
    account_state_reset_core_widgets_hooks
);
