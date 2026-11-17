use std::collections::HashMap;
use std::time::Duration;

use gpui::{App, Context, Entity};
use rise_canon::rise_key::{bot as bot_keys, report as report_keys};
use rise_canon::{RiseEvent, RiseLimits};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_moderation::{ContentStatus, PlatformRole};
use rise_platform::haptics::{NotificationKind, notification};
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::{SaiCacheCleanLayer, SaiCacheCleaner};
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::moderation::stores::moderation_actions::moderation_actions::ModerationActionsStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    ModeratorCapability, ModeratorDetailState, ModeratorKind, ModeratorKindFilter, ModeratorPermissions,
    ModeratorScopeType, ReportTargetType, ReportTargetTypeExt,
};
use crate::modules::moderation::stores::moderation_actions::moderation_wire_types::{
    AllModeratorsGatewayDTO, ModeratorProfileGatewayDTO, ProtoModeratorProfileWithUserExt, ModeratorStatsGatewayDTO,
    ModeratorStatsWireDTO, ModeratorWithUserWireDTO, ReportQueueWireDTO, ReportsQueueGatewayDTO,
};
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

#[derive(Clone, Debug, Default)]
pub struct ModeratorUpdateRevert {
    pub roster_snapshot: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Default)]
pub struct ModeratorRemoveRevert {
    pub roster_snapshot: Option<Map<String, Value>>,
}

pub const SANCTION_TOAST_DURATION: TimeInterval = 3.2;
const SANCTION_BANNER_GAP: TimeInterval = 0.4;

pub struct ModerationServicesStore {
    pub moderator_permissions: Option<ModeratorPermissions>,
    pub moderator_kind: Option<ModeratorKind>,
    pub moderator_is_active: bool,
    pub did_load_moderator_profile: bool,

    pub moderators_error: Option<String>,
    pub kind_filter: ModeratorKindFilter,

    detail_state: HashMap<String, ModeratorDetailState>,
    history_error: HashMap<String, String>,

    pub sanction_notice_uptime: Option<f64>,
}

impl ModerationServicesStore {
    pub fn sanction_toast_remaining(&self) -> Option<f64> {
        let shown = self.sanction_notice_uptime?;
        let elapsed = RiseClock::monotonic().as_secs_f64() - shown;
        let remaining = SANCTION_TOAST_DURATION + SANCTION_BANNER_GAP - elapsed;
        (elapsed >= 0.0 && remaining > 0.0).then_some(remaining)
    }

    /// sends a top banner or holds it until the sanction toast closes
    pub fn send_after_sanction_toast(title: String, options: NotifierSendOptions, cx: &mut App) {
        let Some(delay) = Self::shared(cx).read(cx).sanction_toast_remaining() else {
            Notifier::send(title, options, cx);
            return;
        };
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            let _ = cx.update(|cx| Notifier::send(title, options, cx));
        })
        .detach();
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            moderator_permissions: None,
            moderator_kind: None,
            moderator_is_active: false,
            did_load_moderator_profile: false,
            moderators_error: None,
            kind_filter: ModeratorKindFilter::All,
            detail_state: HashMap::new(),
            history_error: HashMap::new(),
            sanction_notice_uptime: None,
        }
    }

    pub fn moderators_limit() -> i64 {
        RiseLimits::client_value("moderation_services_store_m_o_d_e_r_a_t_o_r_s_l_i_m_i_t")
    }

    pub fn history_limit() -> i64 {
        RiseLimits::client_value("moderation_services_store_h_i_s_t_o_r_y_l_i_m_i_t")
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.moderator_permissions = None;
        self.moderator_kind = None;
        self.moderator_is_active = false;
        self.did_load_moderator_profile = false;
        self.moderators_error = None;
        self.kind_filter = ModeratorKindFilter::All;
        self.detail_state.clear();
        self.history_error.clear();
        cx.notify();
    }

    fn my_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .filter(|id| !id.is_empty())
    }

    pub fn moderators_cache_key(kind: ModeratorKindFilter, cx: &mut App) -> String {
        let Some(me) = Self::my_id(cx) else {
            return String::new();
        };
        SaiCache::new(["getAllModerators", me.as_str(), kind.raw_value()]).key
    }

    pub fn moderator_history_cache_key(target_user_id: &str, cx: &mut App) -> String {
        let Some(me) = Self::my_id(cx) else {
            return String::new();
        };
        SaiCache::new(["getModeratorHistory", me.as_str(), target_user_id]).key
    }

    fn cached_payload(key: &str, observed: bool, cx: &mut App) -> Option<Value> {
        if observed {
            SaiWsResponseCache::observed_payload(
                key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        } else {
            SaiWsResponseCache::current_payload(
                key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        }
    }

    fn moderators_dto(kind_filter: ModeratorKindFilter, observed: bool, cx: &mut App) -> Option<AllModeratorsGatewayDTO> {
        let key = Self::moderators_cache_key(kind_filter, cx);
        if key.is_empty() {
            return None;
        }
        let payload = Self::cached_payload(&key, observed, cx)?;
        SaiJSONCodec::decode::<AllModeratorsGatewayDTO>(Some(&payload))
    }

    fn history_dto(user_id: &str, observed: bool, cx: &mut App) -> Option<ReportsQueueGatewayDTO> {
        let key = Self::moderator_history_cache_key(user_id, cx);
        if key.is_empty() {
            return None;
        }
        let payload = Self::cached_payload(&key, observed, cx)?;
        SaiJSONCodec::decode::<ReportsQueueGatewayDTO>(Some(&payload))
    }

    fn current_kind_filter(cx: &mut App) -> ModeratorKindFilter {
        Self::shared(cx).read(cx).kind_filter
    }

    pub fn moderators(cx: &mut App) -> Vec<ModeratorWithUserWireDTO> {
        let kind_filter = Self::current_kind_filter(cx);
        Self::moderators_dto(kind_filter, true, cx).map(|dto| dto.moderators).unwrap_or_default()
    }

    pub fn moderators_has_more(cx: &mut App) -> bool {
        let kind_filter = Self::current_kind_filter(cx);
        Self::moderators_dto(kind_filter, true, cx).map(|dto| dto.is_have_more_bot_or_top).unwrap_or(false)
    }

    pub fn moderators_relative_id(cx: &mut App) -> Option<i64> {
        let kind_filter = Self::current_kind_filter(cx);
        Self::moderators_dto(kind_filter, true, cx).and_then(|dto| dto.relative_id)
    }

    pub fn moderators_total(cx: &mut App) -> i64 {
        let kind_filter = Self::current_kind_filter(cx);
        match Self::moderators_dto(kind_filter, true, cx) {
            Some(dto) => dto.total,
            None => Self::moderators(cx).len() as i64,
        }
    }

    pub fn is_caller(user_id: Option<&str>, cx: &mut App) -> bool {
        let Some(user_id) = user_id.filter(|id| !id.is_empty()) else {
            return false;
        };
        let Some(me) = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()) else {
            return false;
        };
        user_id == me
    }

    pub fn has_moderators_payload(cx: &mut App) -> bool {
        let kind_filter = Self::current_kind_filter(cx);
        Self::moderators_dto(kind_filter, false, cx).is_some()
    }

    pub fn did_load_moderators(cx: &mut App) -> bool {
        ModerationActionsStore::shared(cx).read(cx).all_moderators_sai.status() == SaiWsSliceStatus::Fulfilled
    }

    pub fn caller_role(cx: &mut App) -> String {
        let role = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.more.role.clone())
            .unwrap_or_default();
        trimming_whitespaces_and_newlines(&role).to_uppercase()
    }

    pub fn is_admin(cx: &mut App) -> bool {
        Self::caller_role(cx) == PlatformRole::Admin.wire()
    }

    pub fn is_super_moder(cx: &mut App) -> bool {
        if Self::caller_role(cx) == PlatformRole::SuperModer.wire() {
            return true;
        }
        let store = Self::shared(cx).read(cx);
        store.moderator_is_active && store.moderator_kind == Some(ModeratorKind::SuperModer)
    }

    pub fn is_moderator(cx: &mut App) -> bool {
        if Self::is_admin(cx) {
            return true;
        }
        let role = Self::caller_role(cx);
        if [PlatformRole::Moderator.wire(), PlatformRole::SuperModer.wire()].contains(&role.as_str()) {
            return true;
        }
        let store = Self::shared(cx).read(cx);
        store.moderator_is_active && store.moderator_kind.is_some()
    }

    pub fn can_enter_moderation(cx: &mut App) -> bool {
        Self::is_moderator(cx) || Self::is_admin(cx)
    }

    pub fn can_resolve_appeals(cx: &mut App) -> bool {
        if Self::is_admin(cx) {
            return true;
        }
        let store = Self::shared(cx).read(cx);
        let loaded = store.did_load_moderator_profile;
        let granted = store.moderator_is_active
            && store.moderator_permissions.as_ref().is_some_and(|permissions| permissions.can_resolve_appeals);
        if loaded {
            return granted;
        }
        granted || Self::is_super_moder(cx)
    }

    pub fn can_moderate(scope: Option<ModeratorScopeType>, cap: ModeratorCapability, cx: &mut App) -> bool {
        if Self::is_admin(cx) {
            return true;
        }
        let store = Self::shared(cx).read(cx);
        let (Some(scope), Some(perms)) = (scope, store.moderator_permissions.as_ref()) else {
            return false;
        };
        perms.can(scope, cap)
    }

    pub fn moderatable_scope_types(cx: &mut App) -> Vec<ModeratorScopeType> {
        if Self::is_admin(cx) {
            return ModeratorScopeType::moderator_scopes().to_vec();
        }
        Self::shared(cx)
            .read(cx)
            .moderator_permissions
            .as_ref()
            .map(ModeratorPermissions::reviewable_types)
            .unwrap_or_default()
    }

    pub fn moderator_granted_event_handler(&mut self, cx: &mut Context<Self>) {
        cx.defer(|cx| {
            ModerationActionsStore::shared(cx).update(cx, |actions, cx| actions.get_my_moderator_profile_action(true, cx));
            UserInteractionsStore::shared(cx).update(cx, |user, cx| user.refresh_my_profile(cx));
        });
        notification(NotificationKind::Success);
        Notifier::send(
            tr("moderator_granted_title"),
            NotifierSendOptions {
                content: Some(tr("moderator_granted_content")),
                r#type: NotifierType::Success,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("checkmark.seal.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn moderator_rights_changed_event_handler(&mut self, cx: &mut Context<Self>) {
        cx.defer(|cx| {
            ModerationInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.moderator_rights_changed(cx));
        });
    }

    pub fn moderator_removed_event_handler(&mut self, cx: &mut Context<Self>) {
        cx.defer(|cx| {
            UserInteractionsStore::shared(cx).update(cx, |user, cx| user.refresh_my_profile(cx));
            ModerationInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.moderator_rights_changed(cx));
        });
    }

    pub fn content_disabled_event_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::ContentDisabled(event)) =
            RiseEvent::decode_payload(rise_events::name::CONTENT_DISABLED, payload)
        else {
            return;
        };
        Self::apply_content_moderation(
            &event.content_type,
            event.post_id,
            event.comment_id,
            ContentStatus::Disabled.wire(),
            cx,
        );
    }

    pub fn content_restored_event_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::ContentRestored(event)) =
            RiseEvent::decode_payload(rise_events::name::CONTENT_RESTORED, payload)
        else {
            return;
        };
        Self::apply_content_moderation(
            &event.content_type,
            event.post_id,
            event.comment_id,
            ContentStatus::Active.wire(),
            cx,
        );
    }

    fn apply_content_moderation(
        content_type: &str,
        post_id: Option<i64>,
        comment_id: Option<i64>,
        status: &str,
        cx: &mut App,
    ) {
        if Some(content_type) == ReportTargetType::Comment.content_kind() {
            let Some(comment_id) = comment_id.filter(|id| *id > 0) else {
                return;
            };
            CommentInteractionsStore::shared(cx)
                .update(cx, |comments, cx| comments.apply_moderation_status(&comment_id.to_string(), status, cx));
        } else {
            let Some(post_id) = post_id.filter(|id| *id > 0) else {
                return;
            };
            PostServicesStore::shared(cx)
                .update(cx, |posts, cx| posts.apply_moderation_status(&post_id.to_string(), status, cx));
        }
    }

    pub fn user_sanctioned_event_handler(&mut self, cx: &mut Context<Self>) {
        if AuthServicesStore::shared(cx).read(cx).is_account_blocked() {
            return;
        }
        Notifier::send(
            tr("user_sanctioned_title"),
            NotifierSendOptions {
                content: Some(tr("user_sanctioned_content")),
                r#type: NotifierType::Warning,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("exclamationmark.shield.fill".to_owned()),
                duration: SANCTION_TOAST_DURATION,
                ..NotifierSendOptions::default()
            },
            cx,
        );
        self.sanction_notice_uptime = Some(RiseClock::monotonic().as_secs_f64());
    }

    pub fn get_my_moderator_profile_success_handler(
        &mut self,
        dto: Option<ModeratorProfileGatewayDTO>,
        cx: &mut Context<Self>,
    ) {
        self.did_load_moderator_profile = true;
        let Some(profile) = dto.and_then(|dto| dto.profile) else {
            self.moderator_permissions = None;
            self.moderator_kind = None;
            self.moderator_is_active = false;
            cx.notify();
            return;
        };
        self.moderator_permissions = Some(ModeratorPermissions::from_json_string(Some(&profile.permissions_json)));
        self.moderator_kind = ModeratorKind::from_wire(&profile.kind.to_uppercase());
        self.moderator_is_active = profile.is_active;
        cx.notify();
    }

    pub fn get_my_moderator_profile_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.did_load_moderator_profile = true;
        cx.notify();
        let message = error.to_string();
        Sai::log("Report", move || format!("getMyModeratorProfile error: {message}"), SaiLogKind::Network, cx);
    }

    fn moderator_row_id(row: &Map<String, Value>) -> Option<String> {
        row.get(report_keys::PROFILE)
            .and_then(Value::as_object)
            .and_then(|profile| profile.get(bot_keys::USER_ID))
            .and_then(Value::as_str)
            .or_else(|| row.get(bot_keys::USER_ID).and_then(Value::as_str))
            .map(str::to_owned)
    }

    pub fn get_all_moderators_success_handler(&mut self, cx: &mut Context<Self>) {
        self.moderators_error = None;
        cx.notify();
    }

    pub fn get_all_moderators_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let message = error.to_string();
        self.moderators_error = Some(message.clone());
        cx.notify();
        Sai::log("ModeratorAdmin", move || format!("getAllModerators error: {message}"), SaiLogKind::Network, cx);
    }

    pub fn get_moderator_profile_success_handler(
        &mut self,
        row: Option<ModeratorWithUserWireDTO>,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = row else {
            return;
        };
        self.patch_moderator_row(target_user_id, &row, cx);
    }

    pub fn get_moderator_profile_error_handler(
        &mut self,
        error: &anyhow::Error,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        let message = format!("getModeratorProfile({target_user_id}) error: {error}");
        Sai::log("ModeratorAdmin", move || message, SaiLogKind::Network, cx);
    }

    fn patch_moderator_row(&mut self, target_user_id: &str, dto: &ModeratorWithUserWireDTO, cx: &mut Context<Self>) {
        let key = Self::moderators_cache_key(self.kind_filter, cx);
        if key.is_empty() {
            return;
        }
        let Some(row) = SaiJSONCodec::encode(dto) else {
            return;
        };
        let _ = SaiUpdater::update_json_value(
            &key,
            |ctx| {
                let mut root = AnyDictionaryBridge::dictionary(ctx.payload.as_ref()).unwrap_or_default();
                let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(report_keys::MODERATORS));
                if let Some(idx) =
                    rows.iter().position(|candidate| Self::moderator_row_id(candidate).as_deref() == Some(target_user_id))
                {
                    rows[idx] = row;
                }
                root.insert(
                    report_keys::MODERATORS.to_owned(),
                    Value::Array(rows.into_iter().map(Value::Object).collect()),
                );
                Ok(Value::Object(root))
            },
            SaiUpdaterWriteScope::Cache,
            cx,
        );
    }

    fn roster_snapshot(key: &str, cx: &mut App) -> Option<Map<String, Value>> {
        let payload = SaiWsResponseCache::current_payload(
            key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        AnyDictionaryBridge::dictionary(payload.as_ref())
    }

    pub fn apply_update_optimistic(
        &mut self,
        target_user_id: &str,
        is_active: Option<bool>,
        cx: &mut Context<Self>,
    ) -> ModeratorUpdateRevert {
        let key = Self::moderators_cache_key(self.kind_filter, cx);
        let roster_snapshot = Self::roster_snapshot(&key, cx);

        if let Some(is_active) = is_active
            && !key.is_empty()
        {
            let _ = SaiUpdater::update_json_value(
                &key,
                |ctx| {
                    let mut root = AnyDictionaryBridge::dictionary(ctx.payload.as_ref()).unwrap_or_default();
                    let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(report_keys::MODERATORS));
                    if let Some(idx) = rows
                        .iter()
                        .position(|candidate| Self::moderator_row_id(candidate).as_deref() == Some(target_user_id))
                    {
                        let row = &mut rows[idx];
                        let mut profile =
                            AnyDictionaryBridge::dictionary(row.get(report_keys::PROFILE)).unwrap_or_default();
                        profile.insert(bot_keys::IS_ACTIVE.to_owned(), Value::Bool(is_active));
                        row.insert(report_keys::PROFILE.to_owned(), Value::Object(profile));
                    }
                    root.insert(
                        report_keys::MODERATORS.to_owned(),
                        Value::Array(rows.into_iter().map(Value::Object).collect()),
                    );
                    Ok(Value::Object(root))
                },
                SaiUpdaterWriteScope::Cache,
                cx,
            );
        }
        ModeratorUpdateRevert { roster_snapshot }
    }

    pub fn update_moderator_permissions_success_handler(
        &mut self,
        updated: Option<ModeratorWithUserWireDTO>,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        notification(NotificationKind::Success);
        if let Some(updated) = updated {
            self.patch_moderator_row(target_user_id, &updated, cx);
        }
        Self::invalidate_moderator_caches(target_user_id, cx);
        Notifier::send(
            tr("moderator_update_success_title"),
            NotifierSendOptions {
                content: Some(tr("moderator_update_success_content")),
                r#type: NotifierType::Success,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("checkmark.seal.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn update_moderator_permissions_error_handler(
        &mut self,
        error: &anyhow::Error,
        target_user_id: &str,
        revert: ModeratorUpdateRevert,
        cx: &mut Context<Self>,
    ) {
        let key = Self::moderators_cache_key(self.kind_filter, cx);
        if !key.is_empty()
            && let Some(snapshot) = revert.roster_snapshot
        {
            let _ = SaiUpdater::replace_root(&key, snapshot, SaiUpdaterWriteScope::Cache, cx);
        }
        Notifier::send(
            tr("moderator_update_failed_title"),
            NotifierSendOptions {
                content: Some(error.to_string()),
                r#type: NotifierType::Warning,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
        let message = format!("updateModeratorPermissions({target_user_id}) error: {error}");
        Sai::log("ModeratorAdmin", move || message, SaiLogKind::Network, cx);
    }

    pub fn apply_remove_moderator_optimistic(
        &mut self,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) -> ModeratorRemoveRevert {
        let key = Self::moderators_cache_key(self.kind_filter, cx);
        let roster_snapshot = Self::roster_snapshot(&key, cx);
        if !key.is_empty() {
            let _ = SaiUpdater::update_json_value(
                &key,
                |ctx| {
                    let mut root = AnyDictionaryBridge::dictionary(ctx.payload.as_ref()).unwrap_or_default();
                    let rows = AnyDictionaryBridge::array_of_dictionaries(root.get(report_keys::MODERATORS));
                    let kept: Vec<Value> = rows
                        .into_iter()
                        .filter(|candidate| Self::moderator_row_id(candidate).as_deref() != Some(target_user_id))
                        .map(Value::Object)
                        .collect();
                    root.insert(report_keys::MODERATORS.to_owned(), Value::Array(kept));
                    Ok(Value::Object(root))
                },
                SaiUpdaterWriteScope::Cache,
                cx,
            );
        }
        ModeratorRemoveRevert { roster_snapshot }
    }

    pub fn remove_moderator_success_handler(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        Self::invalidate_moderator_caches(target_user_id, cx);
        notification(NotificationKind::Success);
        Notifier::send(
            tr("moderator_remove_success_title"),
            NotifierSendOptions {
                content: Some(tr("moderator_remove_success_content")),
                r#type: NotifierType::Success,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("checkmark.seal.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn remove_moderator_error_handler(
        &mut self,
        error: &anyhow::Error,
        target_user_id: &str,
        revert: ModeratorRemoveRevert,
        cx: &mut Context<Self>,
    ) {
        let key = Self::moderators_cache_key(self.kind_filter, cx);
        if !key.is_empty()
            && let Some(snapshot) = revert.roster_snapshot
        {
            let _ = SaiUpdater::replace_root(&key, snapshot, SaiUpdaterWriteScope::Cache, cx);
        }
        Notifier::send(
            tr("moderator_remove_failed_title"),
            NotifierSendOptions {
                content: Some(error.to_string()),
                r#type: NotifierType::Warning,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
        let message = format!("removeModerator({target_user_id}) error: {error}");
        Sai::log("ModeratorAdmin", move || message, SaiLogKind::Network, cx);
    }

    pub fn get_moderator_stats_success_handler(
        &mut self,
        dto: Option<ModeratorStatsGatewayDTO>,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        let mut state = self.detail_state.get(target_user_id).cloned().unwrap_or_default();
        state.did_load_stats = true;
        state.stats_error = None;
        if let Some(dto) = dto {
            state.stats = dto.stats;
            state.stats_user = dto.user;
        }
        self.detail_state.insert(target_user_id.to_owned(), state);
        cx.notify();
    }

    pub fn get_moderator_stats_error_handler(
        &mut self,
        error: &anyhow::Error,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        let mut state = self.detail_state.get(target_user_id).cloned().unwrap_or_default();
        state.did_load_stats = true;
        state.stats_error = Some(error.to_string());
        self.detail_state.insert(target_user_id.to_owned(), state);
        cx.notify();
        let message = format!("getModeratorStats({target_user_id}) error: {error}");
        Sai::log("ModeratorAdmin", move || message, SaiLogKind::Network, cx);
    }

    pub fn get_moderator_history_success_handler(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        self.history_error.remove(target_user_id);
        cx.notify();
    }

    pub fn get_moderator_history_error_handler(
        &mut self,
        error: &anyhow::Error,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.history_error.insert(target_user_id.to_owned(), error.to_string());
        cx.notify();
        let message = format!("getModeratorHistory({target_user_id}) error: {error}");
        Sai::log("ModeratorAdmin", move || message, SaiLogKind::Network, cx);
    }

    pub fn invalidate_moderator_caches(target_user_id: &str, cx: &mut App) {
        let Some(me) = Self::my_id(cx) else {
            return;
        };
        let mut keys: Vec<String> = vec![
            SaiCache::new(["getModeratorProfile", me.as_str(), target_user_id]).key,
            SaiCache::new(["getModeratorStats", me.as_str(), target_user_id]).key,
        ];
        for kind in ModeratorKindFilter::ALL {
            keys.push(SaiCache::new(["getAllModerators", me.as_str(), kind.raw_value()]).key);
        }
        SaiCacheCleaner::clean(&keys, SaiCacheCleanLayer::Everywhere, cx);
    }

    pub fn moderator(user_id: &str, cx: &mut App) -> Option<ModeratorWithUserWireDTO> {
        Self::moderators(cx)
            .into_iter()
            .find(|moderator| moderator.target_user_id().as_deref() == Some(user_id))
    }

    pub fn detail(user_id: &str, cx: &mut App) -> ModeratorDetailState {
        let (mut state, history_error) = {
            let store = Self::shared(cx).read(cx);
            (
                store.detail_state.get(user_id).cloned().unwrap_or_default(),
                store.history_error.get(user_id).cloned(),
            )
        };
        let dto = Self::history_dto(user_id, true, cx);
        state.history = dto.as_ref().map(|dto| dto.reports.clone()).unwrap_or_default();
        state.history_has_more = dto.as_ref().is_some_and(|dto| dto.is_have_more_bot_or_top);
        state.history_relative_id = dto
            .as_ref()
            .and_then(|dto| dto.relative_id.or_else(|| dto.reports.last().map(|report| report.id)));
        state.history_total = dto.as_ref().map(|dto| dto.total).unwrap_or(state.history.len() as i64);
        state.history_error = history_error;
        state
    }

    pub fn stats(&self, user_id: &str) -> Option<ModeratorStatsWireDTO> {
        self.detail_state.get(user_id).and_then(|state| state.stats.clone())
    }

    pub fn did_load_stats(&self, user_id: &str) -> bool {
        self.detail_state.get(user_id).is_some_and(|state| state.did_load_stats)
    }

    pub fn history(user_id: &str, cx: &mut App) -> Vec<ReportQueueWireDTO> {
        Self::history_dto(user_id, true, cx).map(|dto| dto.reports).unwrap_or_default()
    }

    pub fn did_load_history(user_id: &str, cx: &mut App) -> bool {
        Self::history_dto(user_id, true, cx).is_some()
    }
}
