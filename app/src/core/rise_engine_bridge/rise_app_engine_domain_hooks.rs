use std::any::Any;
use std::sync::{Arc, Weak};

use futures::future::BoxFuture;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineBootstrapContext, RiseAppEngineDomainLifetime,
};

pub type RiseAppEngineDomainPrepare = Box<
    dyn Fn(RiseAppEngineBootstrapContext) -> BoxFuture<'static, anyhow::Result<RiseAppEngineDomainLifetime>>
        + Send
        + Sync,
>;

pub type RiseAppEngineSetCallHistorySink = Box<
    dyn Fn(Arc<dyn Any + Send + Sync>, Weak<dyn Any + Send + Sync>) -> BoxFuture<'static, ()>
        + Send
        + Sync,
>;

macro_rules! rise_app_engine_domain_hooks {
    ($(#[$doc:meta])* $name:ident, $slot:ident) => {
        $(#[$doc])*
        pub(crate) struct $name {
            pub prepare: RiseAppEngineDomainPrepare,
        }

        pub(crate) static $slot: FeatureHookSlot<$name> = FeatureHookSlot::new(stringify!($slot));

        impl $name {
            pub fn install(hooks: Self) {
                let _ = $slot.set(hooks);
            }

            pub fn installed() -> Option<&'static Self> {
                $slot.get()
            }
        }
    };
}

rise_app_engine_domain_hooks!(
    RiseAppEngineUserDomainHooks,
    USER_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineMusicDomainHooks,
    MUSIC_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEnginePostDomainHooks,
    POST_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineSearchDomainHooks,
    SEARCH_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineConversationDomainHooks,
    CONVERSATION_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineNotifyDomainHooks,
    NOTIFY_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineGuidesDomainHooks,
    GUIDES_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineThemeDomainHooks,
    THEME_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineModerationDomainHooks,
    MODERATION_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineCommentDomainHooks,
    COMMENT_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineMapDomainHooks,
    MAP_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineSparksDomainHooks,
    SPARKS_DOMAIN_HOOKS
);
rise_app_engine_domain_hooks!(
    RiseAppEngineGiftsDomainHooks,
    GIFTS_DOMAIN_HOOKS
);

/// rtc domain hooks, the lifetime carries the domain
/// so the bridge can hand it the call history sink
/// socket connected maps to application did become active
pub(crate) struct RiseAppEngineRtcDomainHooks {
    pub prepare: RiseAppEngineDomainPrepare,
    pub set_call_history_sink: RiseAppEngineSetCallHistorySink,
}

pub(crate) static RTC_DOMAIN_HOOKS: FeatureHookSlot<RiseAppEngineRtcDomainHooks> = FeatureHookSlot::new("RTC_DOMAIN_HOOKS");

impl RiseAppEngineRtcDomainHooks {
    pub fn install(hooks: Self) {
        let _ = RTC_DOMAIN_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        RTC_DOMAIN_HOOKS.get()
    }
}
