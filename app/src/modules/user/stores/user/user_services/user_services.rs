use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context, Entity, Task};
use rise_canon::rise_key::user as key;
use rise_canon::rise_request::user::{sync_profile_goals, sync_profile_plans};
use rise_canon::{RiseEvent, RiseGuideSignal, RiseLimits, RiseTimingCanon, rise_request};
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::RiseSubscription;
use rise_engine::CancellationError;
use rise_events::name;
use serde_json::{Map, Value};

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{SaiDebouncer, SaiDebouncerOptions};
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::engine::rise_user_domain::RiseUserDomainRegistry;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::shared::contact_device_book::{ContactDeviceAuthorization, ContactDeviceBook};
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::UserContactPhotoSuggestionWireDTO;
use crate::modules::user::stores::user::user_actions::user_types::{
    GoalItem, GoalSubgoalItem, PlanItem, PrivacyExceptionAction, PrivacyRulePatch, ProfileModel, ProfileSocialLink, UserListContinuation,
    UserListEntry, UserListKind, UserListRelationship, UserRelationshipFailureCacheContract,
    UserRelationshipListCacheRegistryContract, UserRelationshipListCacheScope, UserRelationshipListRenderContract,
    UserRelationshipSearchContract,
};
use crate::modules::user::stores::user::user_actions::user_wire_types::{
    ChatMuteOverridesGatewayDTO, FriendDtoExt, FriendListGatewayDTO, PrivacyExceptionWireDTO, PrivacySettingsWireDTO,
    RiseUserProfileRow, SyncProfileGoalsGatewayDTO, SyncProfilePlansGatewayDTO, UserGetProfileByTagGatewayDTO,
    UserGetProfileByTagGatewayDTOExt, UserGoalWireEntry, UserGoalWireEntryExt, UserPlanWireEntry,
    UserPlanWireEntryExt, UserProfileSubscriptionDTO, UserSubscriptionDtoExt, UserSubsListGatewayDTO,
};

use super::user_guide_sandbox_hooks::user_guide_sandbox_hooks;
use super::user_services_hooks::UserServicesHooks;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserServicesStoreFollowOverlay {
    pub is_subbed: bool,
    pub is_subscriber: bool,
    pub is_friend: bool,
    pub subscribers_count: i64,
    pub friends_count: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserServicesStoreFollowBaseline {
    pub subbed: bool,
    pub friend: bool,
    pub t_follows_me: bool,
    pub their_followers: i64,
    pub their_friends: i64,
    pub my_following: i64,
    pub my_friends: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UserServicesStoreContactForegroundRefresh {
    JoinEvents,
    PhotoSuggestions,
    SavedContactFolders,
    AddressBookSync,
}

impl UserServicesStoreContactForegroundRefresh {
    pub const ALL_CASES: [Self; 4] =
        [Self::JoinEvents, Self::PhotoSuggestions, Self::SavedContactFolders, Self::AddressBookSync];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::JoinEvents => "contact_join_events",
            Self::PhotoSuggestions => "contact_photo_suggestions",
            Self::SavedContactFolders => "saved_contacts_folders",
            Self::AddressBookSync => "address_book_sync",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|kind| kind.raw_value() == raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FollowMyCountsBase {
    following: i64,
    friends: i64,
}

const GOALS_SYNC_DEBOUNCE_ID: &str = "syncProfileGoals";
const GOALS_SYNC_GROUP: &str = "profileGoals";
const PLANS_SYNC_DEBOUNCE_ID: &str = "syncProfilePlans";
const PLANS_SYNC_GROUP: &str = "profilePlans";
const PROFILE_SYNC_DEBOUNCE_MILLISECONDS: i64 = 450;

pub struct UserServicesStore {
    pub follow_overlays: HashMap<String, UserServicesStoreFollowOverlay>,
    pub follow_baselines: HashMap<String, UserServicesStoreFollowBaseline>,
    pub follow_pending: HashSet<String>,
    failed_list_cache_keys: HashSet<String>,
    registered_list_cache_scopes: Vec<UserRelationshipListCacheScope>,
    follow_profile_tags: HashMap<String, String>,
    follow_cache_snapshots: HashMap<String, HashMap<String, Map<String, Value>>>,
    follow_my_counts_base: Option<FollowMyCountsBase>,
    committed_follow_overlays: HashMap<String, (UserServicesStoreFollowOverlay, Option<String>)>,
    foreign_profile_fetched_at: HashMap<String, TimeInterval>,
    list_fetched_at: HashMap<String, TimeInterval>,
    peers_restricting_viewer: HashSet<String>,
    peers_refusing_viewer_messages: HashSet<String>,

    optimistic_privacy: Option<PrivacySettingsWireDTO>,
    privacy_mutation_snapshot: Option<PrivacySettingsWireDTO>,
    social_links_mutation_snapshot: Option<Vec<ProfileSocialLink>>,
    quick_reaction_mutation_snapshot: Option<String>,
    quick_reaction_refresh_revision: u64,
    pub quick_reaction_mutation_pending: bool,
    pub quick_reaction_mutation_failed: bool,
    premium_badge_mutation_snapshot: Option<Option<String>>,
    pub premium_badge_mutation_pending: bool,
    pub premium_badge_mutation_failed: bool,
    name_color_mutation_snapshot: Option<Option<String>>,
    pub name_color_mutation_pending: bool,

    phone_public_mutation_snapshot: Option<bool>,
    pub phone_public_mutation_pending: bool,
    phone_public_mutation_revision: u64,
    my_profile_read_at: Option<TimeInterval>,

    mute_all_chats_enabled: bool,
    chat_mute_override_ids: HashSet<String>,
    pub chat_mute_override_pending: HashSet<String>,
    chat_mute_overrides_did_load: bool,

    pub contact_authorization: ContactDeviceAuthorization,
    pub should_present_contact_sync_onboarding: bool,
    pub contact_sync_enable_after_system_settings: bool,
    pub contact_sync_settings_did_load: bool,
    pub contact_sync_settings_error: Option<String>,
    pub contact_sync_in_progress: bool,
    pub contact_sync_retry_attempt: i64,
    pub contact_sync_error: Option<String>,
    pub contact_sync_last_completed_at: Option<Date>,
    pub saved_contact_failed_targets: HashSet<String>,
    pub saved_contact_pending_targets: HashSet<String>,
    pub contact_block_pending_targets: HashSet<String>,
    pub contact_photo_pending_targets: HashSet<String>,
    pub contact_photo_suggestion_pending_targets: HashSet<String>,
    pub contact_photo_resolution_request_ids: HashMap<String, String>,
    pub active_contact_photo_suggestion: Option<UserContactPhotoSuggestionWireDTO>,
    pub presented_contact_join_event_ids: HashSet<String>,
    pub contact_join_visible_names: HashMap<String, String>,
    pub contact_join_alias_requested_targets: HashSet<String>,
    pub saved_contact_folder_bootstrap_complete: bool,
    pub contact_sync_task: Option<Task<()>>,
    pub contact_sync_run_id: Option<Uuid>,
    pub contact_sync_rerun_requested: bool,
    pub contact_store_observer: Option<rise_platform::contacts::ContactStoreChangeWatch>,
    pub contact_socket_observer: Option<RiseSubscription>,
    pub contact_socket_drop_observer: Option<RiseSubscription>,
    pub saved_contact_folder_hydration_queue: Vec<String>,
    pub saved_contact_folder_hydration_in_flight: Option<String>,
    pub saved_contact_folder_hydration_attempted_targets: HashSet<String>,
    pub contact_foreground_refreshed_at: HashMap<UserServicesStoreContactForegroundRefresh, Date>,

    goals: Vec<GoalItem>,
    plans: Vec<PlanItem>,
    goals_sync_snapshot: Option<Vec<GoalItem>>,
    suppress_goals_sync: bool,
    plans_sync_snapshot: Option<Vec<PlanItem>>,
    suppress_plans_sync: bool,
}

impl UserServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            follow_overlays: HashMap::new(),
            follow_baselines: HashMap::new(),
            follow_pending: HashSet::new(),
            failed_list_cache_keys: HashSet::new(),
            registered_list_cache_scopes: Vec::new(),
            follow_profile_tags: HashMap::new(),
            follow_cache_snapshots: HashMap::new(),
            follow_my_counts_base: None,
            committed_follow_overlays: HashMap::new(),
            foreign_profile_fetched_at: HashMap::new(),
            list_fetched_at: HashMap::new(),
            peers_restricting_viewer: HashSet::new(),
            peers_refusing_viewer_messages: HashSet::new(),
            optimistic_privacy: None,
            privacy_mutation_snapshot: None,
            social_links_mutation_snapshot: None,
            quick_reaction_mutation_snapshot: None,
            quick_reaction_refresh_revision: 0,
            quick_reaction_mutation_pending: false,
            quick_reaction_mutation_failed: false,
            premium_badge_mutation_snapshot: None,
            premium_badge_mutation_pending: false,
            premium_badge_mutation_failed: false,
            name_color_mutation_snapshot: None,
            name_color_mutation_pending: false,
            phone_public_mutation_snapshot: None,
            phone_public_mutation_pending: false,
            phone_public_mutation_revision: 0,
            my_profile_read_at: None,
            mute_all_chats_enabled: false,
            chat_mute_override_ids: HashSet::new(),
            chat_mute_override_pending: HashSet::new(),
            chat_mute_overrides_did_load: false,
            contact_authorization: ContactDeviceBook::authorization(),
            should_present_contact_sync_onboarding: false,
            contact_sync_enable_after_system_settings: false,
            contact_sync_settings_did_load: false,
            contact_sync_settings_error: None,
            contact_sync_in_progress: false,
            contact_sync_retry_attempt: 0,
            contact_sync_error: None,
            contact_sync_last_completed_at: None,
            saved_contact_failed_targets: HashSet::new(),
            saved_contact_pending_targets: HashSet::new(),
            contact_block_pending_targets: HashSet::new(),
            contact_photo_pending_targets: HashSet::new(),
            contact_photo_suggestion_pending_targets: HashSet::new(),
            contact_photo_resolution_request_ids: HashMap::new(),
            active_contact_photo_suggestion: None,
            presented_contact_join_event_ids: HashSet::new(),
            contact_join_visible_names: HashMap::new(),
            contact_join_alias_requested_targets: HashSet::new(),
            saved_contact_folder_bootstrap_complete: false,
            contact_sync_task: None,
            contact_sync_run_id: None,
            contact_sync_rerun_requested: false,
            contact_store_observer: None,
            contact_socket_observer: None,
            contact_socket_drop_observer: None,
            saved_contact_folder_hydration_queue: Vec::new(),
            saved_contact_folder_hydration_in_flight: None,
            saved_contact_folder_hydration_attempted_targets: HashSet::new(),
            contact_foreground_refreshed_at: HashMap::new(),
            goals: Vec::new(),
            plans: Vec::new(),
            goals_sync_snapshot: None,
            suppress_goals_sync: false,
            plans_sync_snapshot: None,
            suppress_plans_sync: false,
        }
    }

    fn own_profile_freshness() -> TimeInterval {
        RiseTimingCanon::freshness_seconds("user_profile_own")
    }

    fn foreign_profile_freshness() -> TimeInterval {
        RiseTimingCanon::freshness_seconds("user_profile_foreign")
    }

    fn list_freshness() -> TimeInterval {
        RiseTimingCanon::freshness_seconds("user_profile_list")
    }

    fn system_uptime() -> TimeInterval {
        RiseClock::monotonic().as_secs_f64()
    }

    pub fn peers_restricting_viewer(&self) -> &HashSet<String> {
        &self.peers_restricting_viewer
    }

    pub fn peers_refusing_viewer_messages(&self) -> &HashSet<String> {
        &self.peers_refusing_viewer_messages
    }

    pub fn optimistic_privacy(&self) -> Option<&PrivacySettingsWireDTO> {
        self.optimistic_privacy.as_ref()
    }

    pub fn set_optimistic_privacy(&mut self, value: Option<PrivacySettingsWireDTO>, cx: &mut Context<Self>) {
        self.optimistic_privacy = value;
        self.report_message_requests_fact(cx);
        cx.notify();
    }

    pub fn privacy_load_failed(&self, cx: &mut App) -> bool {
        UserActionsStore::slices(cx).get_privacy_sai.status() == SaiWsSliceStatus::Rejected
    }

    pub fn should_present_contact_sync_onboarding(&self) -> bool {
        self.should_present_contact_sync_onboarding
    }

    pub fn quick_reaction_refresh_revision(&self) -> u64 {
        self.quick_reaction_refresh_revision
    }

    pub fn phone_public_mutation_revision(&self) -> u64 {
        self.phone_public_mutation_revision
    }

    pub fn is_my_profile_fresh(&self) -> bool {
        let Some(read_at) = self.my_profile_read_at else {
            return false;
        };
        Self::system_uptime() - read_at < Self::own_profile_freshness()
    }

    pub fn has_read_my_profile(&self) -> bool {
        self.my_profile_read_at.is_some()
    }

    pub fn mute_all_chats_enabled(&self) -> bool {
        self.mute_all_chats_enabled
    }

    pub fn set_mute_all_chats_enabled(&mut self, value: bool, cx: &mut Context<Self>) {
        self.mute_all_chats_enabled = value;
        GuideFacts::set(RiseGuideSignal::MuteAllChatsEnabled, value, cx);
        cx.notify();
    }

    pub fn chat_mute_override_ids(&self) -> &HashSet<String> {
        &self.chat_mute_override_ids
    }

    pub fn set_chat_mute_override_ids(&mut self, value: HashSet<String>, cx: &mut Context<Self>) {
        self.chat_mute_override_ids = value;
        self.report_chat_mute_exceptions_fact(cx);
        cx.notify();
    }

    fn report_chat_mute_exceptions_fact(&self, cx: &mut App) {
        GuideFacts::set(RiseGuideSignal::ChatMuteExceptionsPresent, !self.chat_mute_override_ids.is_empty(), cx);
    }

    pub fn chat_mute_overrides_did_load(&self) -> bool {
        self.chat_mute_overrides_did_load
    }

    pub fn goals(&self) -> &[GoalItem] {
        &self.goals
    }

    pub fn plans(&self) -> &[PlanItem] {
        &self.plans
    }

    pub fn claim_contact_foreground_refresh(
        &mut self,
        kind: UserServicesStoreContactForegroundRefresh,
        now: Date,
        cx: &mut Context<Self>,
    ) -> bool {
        let window = RiseTimingCanon::freshness_seconds(kind.raw_value());
        if let Some(last) = self.contact_foreground_refreshed_at.get(&kind)
            && now.time_interval_since(last) < window
        {
            return false;
        }
        self.contact_foreground_refreshed_at.insert(kind, now);
        cx.notify();
        true
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.my_profile_read_at = None;
        self.contact_foreground_refreshed_at.clear();
        self.foreign_profile_fetched_at.clear();
        self.list_fetched_at.clear();
        self.peers_restricting_viewer.clear();
        self.peers_refusing_viewer_messages.clear();
        self.follow_overlays.clear();
        self.follow_baselines.clear();
        self.follow_pending.clear();
        self.failed_list_cache_keys.clear();
        self.registered_list_cache_scopes.clear();
        self.follow_profile_tags.clear();
        self.follow_cache_snapshots.clear();
        self.follow_my_counts_base = None;
        self.committed_follow_overlays.clear();
        self.optimistic_privacy = None;
        self.report_message_requests_fact(cx);
        self.privacy_mutation_snapshot = None;
        self.social_links_mutation_snapshot = None;
        self.quick_reaction_mutation_snapshot = None;
        self.quick_reaction_mutation_pending = false;
        self.quick_reaction_mutation_failed = false;
        self.premium_badge_mutation_snapshot = None;
        self.premium_badge_mutation_pending = false;
        self.premium_badge_mutation_failed = false;
        self.phone_public_mutation_snapshot = None;
        self.phone_public_mutation_pending = false;
        self.phone_public_mutation_revision = self.phone_public_mutation_revision.wrapping_add(1);
        self.mute_all_chats_enabled = false;
        GuideFacts::set(RiseGuideSignal::MuteAllChatsEnabled, false, cx);
        self.chat_mute_override_ids.clear();
        self.report_chat_mute_exceptions_fact(cx);
        self.chat_mute_override_pending.clear();
        self.chat_mute_overrides_did_load = false;
        self.contact_sync_task = None;
        self.contact_sync_run_id = None;
        self.contact_authorization = ContactDeviceBook::authorization();
        self.should_present_contact_sync_onboarding = false;
        self.contact_sync_enable_after_system_settings = false;
        self.contact_sync_settings_did_load = false;
        self.contact_sync_settings_error = None;
        self.contact_sync_in_progress = false;
        self.contact_sync_retry_attempt = 0;
        self.contact_sync_error = None;
        self.contact_sync_last_completed_at = None;
        self.saved_contact_failed_targets.clear();
        self.saved_contact_pending_targets.clear();
        self.contact_block_pending_targets.clear();
        self.contact_photo_pending_targets.clear();
        self.contact_photo_suggestion_pending_targets.clear();
        self.contact_photo_resolution_request_ids.clear();
        self.active_contact_photo_suggestion = None;
        self.presented_contact_join_event_ids.clear();
        self.saved_contact_folder_bootstrap_complete = false;
        self.saved_contact_folder_hydration_queue.clear();
        self.saved_contact_folder_hydration_in_flight = None;
        self.saved_contact_folder_hydration_attempted_targets.clear();
        self.goals.clear();
        self.plans.clear();
        self.goals_sync_snapshot = None;
        self.plans_sync_snapshot = None;
        self.suppress_goals_sync = false;
        self.suppress_plans_sync = false;
        cx.notify();
    }

    pub fn profile_by_tag_cache_key(tag: &str) -> String {
        let normalized_tag = Self::normalized_foreign_identity(Some(tag)).map(|tag| tag.to_lowercase()).unwrap_or_default();
        SaiCache::new(["getProfileByTag", normalized_tag.as_str()]).key
    }

    pub fn profile_by_id_cache_key(user_id: &str, viewer_id: &str) -> String {
        let normalized_user_id = Self::normalized_foreign_identity(Some(user_id)).unwrap_or_default();
        let normalized_viewer_id = Self::normalized_foreign_identity(Some(viewer_id)).unwrap_or_default();
        SaiCache::new(["getProfileById", normalized_user_id.as_str(), normalized_viewer_id.as_str()]).key
    }

    fn session_profile(cx: &mut App) -> Option<ProfileModel> {
        AuthServicesStore::shared(cx).read(cx).profile().cloned()
    }

    fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn apply_session_profile(profile: ProfileModel, cx: &mut App) {
        AuthServicesStore::shared(cx).update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
    }

    pub fn cached_foreign_profile_tag(&self, raw_tag: &str, cx: &mut App) -> Option<UserGetProfileByTagGatewayDTO> {
        let tag = Self::normalized_foreign_identity(Some(raw_tag))?;
        let response = Self::foreign_profile_dto(&Self::profile_by_tag_cache_key(&tag), cx)?;
        Self::foreign_profile_answers(&response, &tag).then_some(response)
    }

    pub fn foreign_profile_answers(response: &UserGetProfileByTagGatewayDTO, requested_tag: &str) -> bool {
        let Some(model) = response.as_profile_model() else {
            return false;
        };
        let tag = trimming_whitespaces_and_newlines(&model.tag);
        if tag.is_empty() {
            return true;
        }
        std::iter::once(tag).chain(model.alias_tags.iter().map(String::as_str)).any(|handle| {
            compare_case_insensitive(trimming_whitespaces_and_newlines(handle), requested_tag) == Ordering::Equal
        })
    }

    pub fn cached_foreign_profile_user_id(
        &self,
        raw_user_id: &str,
        raw_viewer_id: Option<&str>,
        cx: &mut App,
    ) -> Option<UserGetProfileByTagGatewayDTO> {
        let user_id = Self::normalized_foreign_identity(Some(raw_user_id))?;
        let viewer_id = match Self::normalized_foreign_identity(raw_viewer_id) {
            Some(viewer_id) => viewer_id,
            None => Self::normalized_foreign_identity(Self::effective_user_id_for_wire(cx).as_deref())?,
        };
        let response = Self::foreign_profile_dto(&Self::profile_by_id_cache_key(&user_id, &viewer_id), cx)?;
        (response.as_profile_model()?.id == user_id).then_some(response)
    }

    fn answers_user(response: &UserGetProfileByTagGatewayDTO, user_id: &str) -> bool {
        response.as_profile_model().is_some_and(|model| model.id == user_id)
    }

    pub fn foreign_profile_payload(&self, raw_user_id: &str, cx: &mut App) -> Option<UserGetProfileByTagGatewayDTO> {
        let user_id = Self::normalized_foreign_identity(Some(raw_user_id))?;
        let actions = UserActionsStore::slices(cx);
        let by_tag_slice = actions.get_profile_by_tag_sai.data();
        let by_id_slice = actions.get_profile_by_id_sai.data();
        let by_id = self.cached_foreign_profile_user_id(&user_id, None, cx);
        let last_tag_read = by_tag_slice
            .as_ref()
            .and_then(|response| response.as_profile_model())
            .filter(|model| model.id == user_id)
            .map(|model| model.tag);
        let tag = Self::normalized_foreign_identity(self.follow_profile_tags.get(&user_id).map(String::as_str))
            .or_else(|| Self::normalized_foreign_identity(last_tag_read.as_deref()));
        let by_tag = tag
            .as_deref()
            .and_then(|tag| self.cached_foreign_profile_tag(tag, cx))
            .filter(|response| Self::answers_user(response, &user_id));
        if let (Some(by_id), Some(by_tag), Some(tag)) = (by_id.as_ref(), by_tag.as_ref(), tag.as_deref())
            && let Some(viewer_id) = Self::normalized_foreign_identity(Self::effective_user_id_for_wire(cx).as_deref())
        {
            let id_read_at = self
                .foreign_profile_fetched_at
                .get(&Self::profile_by_id_cache_key(&user_id, &viewer_id))
                .copied()
                .unwrap_or(f64::NEG_INFINITY);
            let tag_read_at = self
                .foreign_profile_fetched_at
                .get(&Self::profile_by_tag_cache_key(tag))
                .copied()
                .unwrap_or(f64::NEG_INFINITY);
            return Some(if tag_read_at > id_read_at { by_tag.clone() } else { by_id.clone() });
        }
        if let Some(response) = by_id.or(by_tag) {
            return Some(response);
        }
        if let Some(response) = by_tag_slice.filter(|response| Self::answers_user(response, &user_id)) {
            return Some(response);
        }
        by_id_slice.filter(|response| Self::answers_user(response, &user_id))
    }

    pub fn foreign_profile_tag(&self, raw_user_id: &str, cx: &mut App) -> Option<String> {
        let user_id = Self::normalized_foreign_identity(Some(raw_user_id))?;
        if let Some(tag) = Self::normalized_foreign_identity(self.follow_profile_tags.get(&user_id).map(String::as_str)) {
            return Some(tag);
        }
        let tag = self.foreign_profile_payload(&user_id, cx)?.as_profile_model()?.tag;
        Self::normalized_foreign_identity(Some(&tag))
    }

    pub fn profile_updated_event_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) -> Option<String> {
        let Some(RiseEvent::UserProfileUpdated(event)) = RiseEvent::decode_payload(name::USER_PROFILE_UPDATED, payload)
        else {
            return None;
        };
        if event.user_id.is_empty() {
            return None;
        }
        let user_id = event.user_id.clone();
        let display_name = event.user_info.as_ref().map(|info| trimming_whitespaces_and_newlines(&info.name).to_owned());
        let logo = event.user_info.as_ref().map(|info| info.more.logo.clone());
        let tag = event.user_info.as_ref().map(|info| info.tag.clone());

        let my_id = Self::session_profile(cx).map(|profile| profile.id).unwrap_or_default();
        if user_id == my_id {
            if let Some(mut current) = Self::session_profile(cx) {
                if let Some(display_name) = display_name.filter(|value| !value.is_empty()) {
                    current.name = display_name;
                }
                if let Some(logo) = logo {
                    current.avatar_url = (!logo.is_empty()).then(|| logo.clone());
                    current.more.logo = logo;
                }
                Self::apply_session_profile(current, cx);
            }
            SaiWsResponseCache::remove(
                &SaiCache::new(["getMyProfile", user_id.as_str()]).key,
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
            if event.updated_fields.iter().any(|field| field.as_str() == key::QUICK_REACTION)
                && let Some(domain) = RiseUserDomainRegistry::shared(cx).read(cx).active().cloned()
            {
                domain.refresh_quick_reaction();
            }
            return None;
        }

        if !my_id.is_empty() {
            SaiWsResponseCache::remove(
                &Self::profile_by_id_cache_key(&user_id, &my_id),
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
        }
        if let Some(tag) = tag.filter(|tag| !tag.is_empty()) {
            SaiWsResponseCache::remove(&Self::profile_by_tag_cache_key(&tag), SaiCacheCleanLayer::Everywhere, cx);
        }

        let fields = &event.updated_fields;
        if !fields.is_empty() && !fields.iter().any(|field| Self::PEER_ROW_PROFILE_FIELDS.contains(&field.as_str())) {
            return None;
        }
        Some(user_id)
    }

    const PEER_ROW_PROFILE_FIELDS: [&'static str; 4] = [key::NAME, key::LOGO, key::AVATAR_URL, key::IS_PREMIUM];

    fn has_in_flight_request(cache_key: &str, cx: &mut App) -> bool {
        SaiWsPendingRegistry::shared(cx).read(cx).in_flight_request_id(cache_key).is_some()
    }

    pub fn is_foreign_profile_pending_tag(&self, raw_tag: &str, cx: &mut App) -> bool {
        let Some(tag) = Self::normalized_foreign_identity(Some(raw_tag)) else {
            return false;
        };
        Self::has_in_flight_request(&Self::profile_by_tag_cache_key(&tag), cx)
    }

    pub fn is_foreign_profile_pending_user_id(&self, raw_user_id: &str, cx: &mut App) -> bool {
        let Some(user_id) = Self::normalized_foreign_identity(Some(raw_user_id)) else {
            return false;
        };
        let Some(viewer_id) = Self::normalized_foreign_identity(Self::effective_user_id_for_wire(cx).as_deref()) else {
            return false;
        };
        Self::has_in_flight_request(&Self::profile_by_id_cache_key(&user_id, &viewer_id), cx)
    }

    pub fn note_foreign_profile_fetched(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        let now = Self::system_uptime();
        let freshness = Self::foreign_profile_freshness();
        self.foreign_profile_fetched_at.retain(|_, fetched_at| now - *fetched_at < freshness);
        self.foreign_profile_fetched_at.insert(cache_key.to_owned(), now);
        cx.notify();
    }

    pub fn note_list_fetched(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        let now = Self::system_uptime();
        let freshness = Self::list_freshness();
        self.list_fetched_at.retain(|_, fetched_at| now - *fetched_at < freshness);
        self.list_fetched_at.insert(cache_key.to_owned(), now);
        cx.notify();
    }

    pub fn forget_list_freshness(&mut self, user_id: &str) {
        if user_id.is_empty() {
            return;
        }
        for kind in [UserListKind::Friends, UserListKind::Subscribers, UserListKind::Subscriptions] {
            let unfiltered = Self::list_cache_key(kind, user_id, None);
            self.list_fetched_at.remove(&unfiltered);
        }
    }

    pub fn is_list_fresh(&self, cache_key: &str) -> bool {
        self.list_fetched_at
            .get(cache_key)
            .is_some_and(|fetched_at| Self::system_uptime() - fetched_at < Self::list_freshness())
    }

    pub fn note_peer_restriction(
        &mut self,
        user_id: &str,
        response: &UserGetProfileByTagGatewayDTO,
        cx: &mut Context<Self>,
    ) {
        let scope =
            response.resolved_owner_block_scope().map(|scope| trimming_whitespaces_and_newlines(scope).to_lowercase());
        let is_friend = response.profile.as_ref().and_then(|profile| profile.is_friend) == Some(true);
        let blocks = scope.as_deref().is_some_and(|scope| !scope.is_empty());
        if (is_friend || !blocks) && self.peers_refusing_viewer_messages.remove(user_id) {
            cx.notify();
        }
        let restricts = scope.as_deref() == Some(RiseBlockScope::Total.raw_value());
        if restricts == self.peers_restricting_viewer.contains(user_id) {
            return;
        }
        if restricts {
            self.peers_restricting_viewer.insert(user_id.to_owned());
        } else {
            self.peers_restricting_viewer.remove(user_id);
        }
        cx.notify();
    }

    pub fn note_peer_restricts_viewer(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let target = trimming_whitespaces_and_newlines(user_id);
        if target.is_empty() || self.peers_restricting_viewer.contains(target) {
            return;
        }
        self.peers_restricting_viewer.insert(target.to_owned());
        cx.notify();
    }

    pub fn peer_restricts_viewer(&self, user_id: &str) -> bool {
        let target = trimming_whitespaces_and_newlines(user_id);
        !target.is_empty() && self.peers_restricting_viewer.contains(target)
    }

    pub fn note_peer_refuses_viewer_messages(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let target = trimming_whitespaces_and_newlines(user_id);
        if target.is_empty() || self.peers_refusing_viewer_messages.contains(target) {
            return;
        }
        self.peers_refusing_viewer_messages.insert(target.to_owned());
        cx.notify();
    }

    pub fn peer_refuses_viewer_messages(&self, user_id: &str) -> bool {
        let target = trimming_whitespaces_and_newlines(user_id);
        !target.is_empty() && self.peers_refusing_viewer_messages.contains(target)
    }

    pub fn is_foreign_profile_fresh_tag(&self, raw_tag: &str, cx: &mut App) -> bool {
        let Some(tag) = Self::normalized_foreign_identity(Some(raw_tag)) else {
            return false;
        };
        self.is_foreign_profile_fresh_cache_key(&Self::profile_by_tag_cache_key(&tag), cx)
    }

    pub fn is_foreign_profile_fresh_user_id(&self, raw_user_id: &str, cx: &mut App) -> bool {
        let Some(user_id) = Self::normalized_foreign_identity(Some(raw_user_id)) else {
            return false;
        };
        let Some(viewer_id) = Self::normalized_foreign_identity(Self::effective_user_id_for_wire(cx).as_deref()) else {
            return false;
        };
        self.is_foreign_profile_fresh_cache_key(&Self::profile_by_id_cache_key(&user_id, &viewer_id), cx)
    }

    fn is_foreign_profile_fresh_cache_key(&self, cache_key: &str, cx: &mut App) -> bool {
        let Some(fetched_at) = self.foreign_profile_fetched_at.get(cache_key).copied() else {
            return false;
        };
        if Self::system_uptime() - fetched_at >= Self::foreign_profile_freshness() {
            return false;
        }
        Self::foreign_profile_dto(cache_key, cx)
            .and_then(|response| {
                response.resolved_owner_block_scope().map(|scope| trimming_whitespaces_and_newlines(scope).to_owned())
            })
            .unwrap_or_default()
            .is_empty()
    }

    pub fn validated_foreign_profile_user_id(
        &self,
        response: &UserGetProfileByTagGatewayDTO,
        expected_user_id: Option<&str>,
        expected_tag: Option<&str>,
        _cx: &mut App,
    ) -> Option<String> {
        let model = response.as_profile_model()?;
        let user_id = Self::normalized_foreign_identity(Some(&model.id))?;
        if let Some(expected_user_id) = Self::normalized_foreign_identity(expected_user_id)
            && expected_user_id != user_id
        {
            return None;
        }
        if let Some(expected_tag) = Self::normalized_foreign_identity(expected_tag)
            && compare_case_insensitive(&model.tag, &expected_tag) != Ordering::Equal
        {
            return None;
        }
        Some(user_id)
    }

    fn foreign_profile_dto(cache_key: &str, cx: &mut App) -> Option<UserGetProfileByTagGatewayDTO> {
        let payload = SaiWsResponseCache::observed_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )?;
        SaiJSONCodec::decode::<UserGetProfileByTagGatewayDTO>(Some(&payload))
    }

    fn normalized_foreign_identity(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        (!value.is_empty()).then(|| value.to_owned())
    }

    pub fn handle_chat_mute_overrides_success(
        &mut self,
        dto: Option<ChatMuteOverridesGatewayDTO>,
        cx: &mut Context<Self>,
    ) {
        let Some(dto) = dto else {
            return;
        };
        self.chat_mute_overrides_did_load = true;
        cx.notify();
        if (user_guide_sandbox_hooks().absorb_mute_overrides)(Some(dto.mute_all), Some(dto.chat_ids.as_slice()), cx) {
            return;
        }
        self.set_mute_all_chats_enabled(dto.mute_all, cx);
        if !self.chat_mute_override_pending.is_empty() {
            return;
        }
        self.set_chat_mute_override_ids(dto.chat_ids.into_iter().collect(), cx);
    }

    pub fn apply_chat_mute_override_optimistic(&mut self, chat_id: &str, add: bool, cx: &mut Context<Self>) {
        if add {
            self.chat_mute_override_ids.insert(chat_id.to_owned());
        } else {
            self.chat_mute_override_ids.remove(chat_id);
        }
        self.report_chat_mute_exceptions_fact(cx);
        cx.notify();
    }

    pub fn mark_chat_mute_override_pending(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.chat_mute_override_pending.insert(chat_id.to_owned());
        cx.notify();
    }

    pub fn clear_chat_mute_override_pending(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.chat_mute_override_pending.remove(chat_id);
        cx.notify();
    }

    pub fn handle_my_profile_success(
        &mut self,
        profile: Option<UserProfileSubscriptionDTO>,
        expected_phone_public_revision: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = profile else {
            tracing::warn!(target: "riseonly::X453", "my_profile success without profile");
            return;
        };
        let mut next = p.as_profile_model();
        tracing::warn!(
            target: "riseonly::X453",
            "my_profile success id={} badge={:?} level={:?} streak={:?} rating={:?}",
            p.id,
            p.premium_badge_emoji,
            next.level,
            next.streak,
            next.rating
        );
        if (self.phone_public_mutation_pending || self.phone_public_mutation_revision != expected_phone_public_revision)
            && let Some(current) = Self::session_profile(cx)
            && current.id == next.id
        {
            next.is_phone_public = current.is_phone_public;
        }
        next.posts_count += UserServicesHooks::in_flight_own_publication_count(&next.id, cx);
        Self::apply_session_profile(next, cx);
        self.my_profile_read_at = Some(Self::system_uptime());

        if let Some(more) = p.more.as_ref() {
            if !self.is_goals_sync_pending() {
                self.replace_goals_from_wire(&more.goals, cx);
            }
            if !self.is_plans_sync_pending() {
                self.replace_plans_from_wire(&more.plans_struct, cx);
            }
        }
        cx.notify();
    }

    pub fn apply_social_links_optimistic(&mut self, links: Vec<ProfileSocialLink>, cx: &mut Context<Self>) {
        let Some(mut profile) = Self::session_profile(cx) else {
            return;
        };
        self.social_links_mutation_snapshot = Some(std::mem::replace(&mut profile.social_links, links));
        Self::apply_session_profile(profile, cx);
        cx.notify();
    }

    pub fn revert_social_links_optimistic(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.social_links_mutation_snapshot.clone() else {
            return;
        };
        let Some(mut profile) = Self::session_profile(cx) else {
            return;
        };
        profile.social_links = snapshot;
        Self::apply_session_profile(profile, cx);
        self.social_links_mutation_snapshot = None;
        cx.notify();
    }

    pub fn commit_social_links_optimistic(&mut self, cx: &mut Context<Self>) {
        self.social_links_mutation_snapshot = None;
        cx.notify();
    }

    pub fn apply_quick_reaction_optimistic(&mut self, emoji: &str, cx: &mut Context<Self>) {
        let Some(mut profile) = Self::session_profile(cx) else {
            return;
        };
        self.quick_reaction_refresh_revision = self.quick_reaction_refresh_revision.wrapping_add(1);
        self.quick_reaction_mutation_snapshot = Some(profile.quick_reaction.clone());
        self.quick_reaction_mutation_pending = true;
        self.quick_reaction_mutation_failed = false;
        profile.quick_reaction = emoji.to_owned();
        Self::apply_session_profile(profile, cx);
        cx.notify();
    }

    pub fn commit_quick_reaction_optimistic(&mut self, cx: &mut Context<Self>) {
        self.quick_reaction_refresh_revision = self.quick_reaction_refresh_revision.wrapping_add(1);
        self.quick_reaction_mutation_snapshot = None;
        self.quick_reaction_mutation_pending = false;
        self.quick_reaction_mutation_failed = false;
        cx.notify();
    }

    pub fn revert_quick_reaction_optimistic(&mut self, cx: &mut Context<Self>) {
        self.quick_reaction_refresh_revision = self.quick_reaction_refresh_revision.wrapping_add(1);
        let Some(snapshot) = self.quick_reaction_mutation_snapshot.take() else {
            self.quick_reaction_mutation_pending = false;
            cx.notify();
            return;
        };
        if let Some(mut profile) = Self::session_profile(cx) {
            profile.quick_reaction = snapshot;
            Self::apply_session_profile(profile, cx);
        }
        self.quick_reaction_mutation_pending = false;
        self.quick_reaction_mutation_failed = true;
        cx.notify();
    }

    pub fn apply_refreshed_quick_reaction(&mut self, emoji: &str, expected_revision: u64, cx: &mut Context<Self>) -> bool {
        if expected_revision != self.quick_reaction_refresh_revision || self.quick_reaction_mutation_pending {
            return false;
        }
        let Some(value) = EmojiPickerCatalog::normalized_selectable_emoji_or_custom_ref(Some(emoji)) else {
            return true;
        };
        let Some(mut profile) = Self::session_profile(cx) else {
            return true;
        };
        if profile.quick_reaction == value {
            return true;
        }
        self.quick_reaction_refresh_revision = self.quick_reaction_refresh_revision.wrapping_add(1);
        profile.quick_reaction = value;
        Self::apply_session_profile(profile, cx);
        cx.notify();
        true
    }

    pub fn apply_name_color_optimistic(&mut self, color: Option<String>, cx: &mut Context<Self>) {
        let Some(mut profile) = Self::session_profile(cx) else {
            return;
        };
        self.name_color_mutation_snapshot = Some(std::mem::replace(&mut profile.more.name_color, color));
        self.name_color_mutation_pending = true;
        Self::apply_session_profile(profile, cx);
        cx.notify();
    }

    pub fn commit_name_color_optimistic(&mut self, cx: &mut Context<Self>) {
        self.name_color_mutation_snapshot = None;
        self.name_color_mutation_pending = false;
        cx.notify();
    }

    pub fn revert_name_color_optimistic(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.name_color_mutation_snapshot.take() else {
            self.name_color_mutation_pending = false;
            cx.notify();
            return;
        };
        if let Some(mut profile) = Self::session_profile(cx) {
            profile.more.name_color = snapshot;
            Self::apply_session_profile(profile, cx);
        }
        self.name_color_mutation_pending = false;
        cx.notify();
    }

    pub fn apply_premium_badge_optimistic(&mut self, emoji: Option<String>, cx: &mut Context<Self>) {
        let Some(mut profile) = Self::session_profile(cx) else {
            return;
        };
        self.premium_badge_mutation_snapshot = Some(std::mem::replace(&mut profile.more.premium_badge_emoji, emoji));
        self.premium_badge_mutation_pending = true;
        self.premium_badge_mutation_failed = false;
        Self::apply_session_profile(profile, cx);
        cx.notify();
    }

    pub fn commit_premium_badge_optimistic(&mut self, cx: &mut Context<Self>) {
        self.premium_badge_mutation_snapshot = None;
        self.premium_badge_mutation_pending = false;
        self.premium_badge_mutation_failed = false;
        cx.notify();
    }

    pub fn clear_premium_badge_failure(&mut self, cx: &mut Context<Self>) {
        if !self.premium_badge_mutation_failed {
            return;
        }
        self.premium_badge_mutation_failed = false;
        cx.notify();
    }

    pub fn revert_premium_badge_optimistic(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.premium_badge_mutation_snapshot.take() else {
            self.premium_badge_mutation_pending = false;
            cx.notify();
            return;
        };
        if let Some(mut profile) = Self::session_profile(cx) {
            profile.more.premium_badge_emoji = snapshot;
            Self::apply_session_profile(profile, cx);
        }
        self.premium_badge_mutation_pending = false;
        self.premium_badge_mutation_failed = true;
        cx.notify();
    }

    pub fn apply_phone_public_optimistic(&mut self, is_public: bool, cx: &mut Context<Self>) {
        let Some(mut profile) = Self::session_profile(cx) else {
            return;
        };
        self.phone_public_mutation_revision = self.phone_public_mutation_revision.wrapping_add(1);
        self.phone_public_mutation_snapshot = Some(profile.is_phone_public);
        self.phone_public_mutation_pending = true;
        profile.is_phone_public = is_public;
        Self::apply_session_profile(profile, cx);
        cx.notify();
    }

    pub fn commit_phone_public_optimistic(&mut self, data: Option<UserProfileSubscriptionDTO>, cx: &mut Context<Self>) {
        self.phone_public_mutation_revision = self.phone_public_mutation_revision.wrapping_add(1);
        self.phone_public_mutation_snapshot = None;
        self.phone_public_mutation_pending = false;
        if let Some(data) = data {
            Self::apply_session_profile(data.as_profile_model(), cx);
        }
        cx.notify();
    }

    pub fn revert_phone_public_optimistic(&mut self, cx: &mut Context<Self>) {
        self.phone_public_mutation_revision = self.phone_public_mutation_revision.wrapping_add(1);
        if let Some(snapshot) = self.phone_public_mutation_snapshot
            && let Some(mut profile) = Self::session_profile(cx)
        {
            profile.is_phone_public = snapshot;
            Self::apply_session_profile(profile, cx);
        }
        self.phone_public_mutation_snapshot = None;
        self.phone_public_mutation_pending = false;
        cx.notify();
    }

    pub fn build_sync_profile_goals_body(&mut self, _cx: &mut Context<Self>) -> rise_request::user::SyncProfileGoals {
        let goals = self
            .goals
            .iter()
            .map(|item| sync_profile_goals::GoalsElement {
                description: Some(item.description.clone()),
                id: Some(item.id.clone()),
                progress: Some((item.progress * 100.0).round().clamp(0.0, 100.0) as i64),
                steps: Some(if item.is_checklist {
                    item.subgoals
                        .iter()
                        .map(|subgoal| sync_profile_goals::goals_element::StepsElement {
                            done: Some(subgoal.done),
                            id: Some(subgoal.id.clone()),
                            title: Some(subgoal.title.clone()),
                        })
                        .collect()
                } else {
                    Vec::new()
                }),
                title: Some(item.title.clone()),
                ..Default::default()
            })
            .collect();
        rise_request::user::SyncProfileGoals { goals: Some(goals) }
    }

    pub fn build_sync_profile_plans_body(&mut self, _cx: &mut Context<Self>) -> rise_request::user::SyncProfilePlans {
        let plans = self
            .plans
            .iter()
            .map(|item| sync_profile_plans::PlansElement {
                date_ms: Some(item.date_ms),
                description: Some(item.description.clone()),
                id: Some(item.id.clone()),
                title: Some(item.title.clone()),
                ..Default::default()
            })
            .collect();
        rise_request::user::SyncProfilePlans { plans: Some(plans) }
    }

    pub fn handle_sync_profile_goals_success(&mut self, data: Option<SyncProfileGoalsGatewayDTO>, cx: &mut Context<Self>) {
        if let Some(data) = data {
            self.replace_goals_from_wire(&data.goals, cx);
        }
        self.clear_goals_sync_snapshot(cx);
    }

    pub fn handle_sync_profile_goals_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.revert_goals_if_pending_sync(cx);
        Self::report_sync_error("sync_profile_goals", error, cx);
    }

    pub fn handle_sync_profile_plans_success(&mut self, data: Option<SyncProfilePlansGatewayDTO>, cx: &mut Context<Self>) {
        if let Some(data) = data {
            self.replace_plans_from_wire(&data.plans, cx);
        }
        self.clear_plans_sync_snapshot(cx);
    }

    pub fn handle_sync_profile_plans_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.revert_plans_if_pending_sync(cx);
        Self::report_sync_error("sync_profile_plans", error, cx);
    }

    fn report_sync_error(method: &'static str, error: &anyhow::Error, cx: &mut App) {
        let text = format!("{method} error: {error}");
        Sai::log("User", move || text, SaiLogKind::Network, cx);
        if error.downcast_ref::<CancellationError>().is_none() {
            Self::notify_error(CanonErrors::refusal_text(error), cx);
        }
    }

    fn notify_error(title: String, cx: &mut App) {
        Notifier::send(title, NotifierSendOptions { r#type: NotifierType::Error, ..Default::default() }, cx);
    }

    pub fn patch_profile_by_tag_friend_request_id(&mut self, request_id: Option<i64>, tag: &str, cx: &mut Context<Self>) {
        let cache_key = Self::profile_by_tag_cache_key(tag);
        let mut fields = Map::new();
        fields.insert(key::FRIEND_REQUEST_ID.to_owned(), request_id.map_or(Value::Null, Value::from));
        let _ = SaiUpdater::patch_root(&cache_key, &fields, SaiUpdaterWriteScope::All, cx);
    }

    pub fn report_message_requests_fact(&self, cx: &mut App) {
        let enabled = match self.optimistic_privacy.as_ref() {
            Some(privacy) => privacy.message_requests_enabled,
            None => UserActionsStore::slices(cx)
                .get_privacy_sai
                .data()
                .is_some_and(|privacy| privacy.message_requests_enabled),
        };
        GuideFacts::set(RiseGuideSignal::MessageRequestsEnabled, enabled, cx);
    }

    fn privacy_source(&self, cx: &mut App) -> Option<PrivacySettingsWireDTO> {
        self.optimistic_privacy.clone().or_else(|| UserActionsStore::slices(cx).get_privacy_sai.data())
    }

    pub fn apply_privacy_patch_optimistic(
        &mut self,
        patch: &PrivacyRulePatch,
        server: Option<PrivacySettingsWireDTO>,
        cx: &mut Context<Self>,
    ) {
        let Some(base) = self.optimistic_privacy.clone().or(server) else {
            return;
        };
        self.privacy_mutation_snapshot = Some(base.clone());
        let mut next = base;
        let rules: [(&Option<String>, &mut String); 23] = [
            (&patch.plan_rule, &mut next.plan_rule),
            (&patch.goal_rule, &mut next.goal_rule),
            (&patch.friend_rule, &mut next.friend_rule),
            (&patch.hb_rule, &mut next.hb_rule),
            (&patch.phone_rule, &mut next.phone_rule),
            (&patch.description_rule, &mut next.description_rule),
            (&patch.last_seen_rule, &mut next.last_seen_rule),
            (&patch.online_status_rule, &mut next.online_status_rule),
            (&patch.profile_photo_rule, &mut next.profile_photo_rule),
            (&patch.calls_rule, &mut next.calls_rule),
            (&patch.forwards_rule, &mut next.forwards_rule),
            (&patch.groups_rule, &mut next.groups_rule),
            (&patch.subs_rule, &mut next.subs_rule),
            (&patch.comments_rule, &mut next.comments_rule),
            (&patch.reposts_rule, &mut next.reposts_rule),
            (&patch.achievements_rule, &mut next.achievements_rule),
            (&patch.stories_rule, &mut next.stories_rule),
            (&patch.messages_rule, &mut next.messages_rule),
            (&patch.coauthor_rule, &mut next.coauthor_rule),
            (&patch.tag_rule, &mut next.tag_rule),
            (&patch.sparks_rule, &mut next.sparks_rule),
            (&patch.gifts_rule, &mut next.gifts_rule),
            (&patch.gift_send_rule, &mut next.gift_send_rule),
        ];
        for (value, target) in rules {
            if let Some(value) = value {
                *target = value.clone();
            }
        }
        if let Some(value) = patch.subs_count_hidden {
            next.subs_count_hidden = value;
        }
        if let Some(value) = patch.message_requests_enabled {
            next.message_requests_enabled = value;
        }
        self.set_optimistic_privacy(Some(next), cx);
    }

    pub fn privacy_exceptions(
        &self,
        privacy_type: &str,
        action: PrivacyExceptionAction,
        cx: &mut App,
    ) -> Vec<PrivacyExceptionWireDTO> {
        let Some(source) = self.privacy_source(cx) else {
            return Vec::new();
        };
        let list = match action {
            PrivacyExceptionAction::Allow => source.allow_exceptions,
            PrivacyExceptionAction::Deny => source.deny_exceptions,
        };
        list.into_iter().filter(|row| row.privacy_type == privacy_type).collect()
    }

    pub fn apply_privacy_exceptions_optimistic(
        &mut self,
        privacy_type: &str,
        action: PrivacyExceptionAction,
        users: Vec<PrivacyExceptionWireDTO>,
        cx: &mut Context<Self>,
    ) {
        let Some(base) = self.privacy_source(cx) else {
            return;
        };
        self.privacy_mutation_snapshot = Some(base.clone());
        let mut next = base;
        let list = match action {
            PrivacyExceptionAction::Allow => &mut next.allow_exceptions,
            PrivacyExceptionAction::Deny => &mut next.deny_exceptions,
        };
        list.retain(|row| row.privacy_type != privacy_type);
        list.extend(users);
        self.set_optimistic_privacy(Some(next), cx);
    }

    pub fn rollback_privacy_optimistic(&mut self, cx: &mut Context<Self>) {
        self.privacy_mutation_snapshot = None;
        self.set_optimistic_privacy(None, cx);
    }

    pub fn commit_privacy_optimistic(&mut self, cx: &mut Context<Self>) {
        self.privacy_mutation_snapshot = None;
        self.set_optimistic_privacy(None, cx);
    }

    pub fn handle_update_privacy_success(&mut self, settings: Option<PrivacySettingsWireDTO>, cx: &mut Context<Self>) {
        self.commit_privacy_optimistic(cx);
        let Some(settings) = settings else {
            return;
        };
        let Some(user_id) = Self::session_profile(cx).map(|profile| profile.id) else {
            return;
        };
        let Some(root) = SaiJSONCodec::encode(&settings) else {
            return;
        };
        let cache_key = SaiCache::new(["getPrivacySettings", user_id.as_str()]).key;
        let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx);
    }

    pub fn current_follow(&self, user_id: &str, cx: &mut App) -> UserListRelationship {
        if let Some(overlay) = self.follow_overlays.get(user_id) {
            return Self::overlay_relationship(overlay);
        }
        if let Some(baseline) = self.follow_baselines.get(user_id) {
            return Self::baseline_relationship(baseline);
        }
        if let Some(model) = self.fetched_foreign_profile(user_id, cx) {
            return UserListRelationship {
                is_subbed: model.is_subbed,
                is_subscriber: model.is_subscriber,
                is_friend: model.is_friend,
            };
        }
        UserListRelationship { is_subbed: false, is_subscriber: false, is_friend: false }
    }

    fn overlay_relationship(overlay: &UserServicesStoreFollowOverlay) -> UserListRelationship {
        UserListRelationship {
            is_subbed: overlay.is_subbed,
            is_subscriber: overlay.is_subscriber,
            is_friend: overlay.is_friend,
        }
    }

    fn baseline_relationship(baseline: &UserServicesStoreFollowBaseline) -> UserListRelationship {
        UserListRelationship {
            is_subbed: baseline.subbed,
            is_subscriber: baseline.t_follows_me,
            is_friend: baseline.friend,
        }
    }

    pub fn resolved_follow_relationship(
        &self,
        user_id: &str,
        fallback: UserListRelationship,
        cx: &mut App,
    ) -> UserListRelationship {
        if let Some(relationship) = self.overlay_or_baseline_relationship(user_id) {
            return relationship;
        }
        if let Some(model) = self.fetched_foreign_profile(user_id, cx) {
            return UserListRelationship {
                is_subbed: model.is_subbed,
                is_subscriber: model.is_subscriber,
                is_friend: model.is_friend,
            };
        }
        fallback
    }

    fn overlay_or_baseline_relationship(&self, user_id: &str) -> Option<UserListRelationship> {
        self.follow_overlays
            .get(user_id)
            .map(Self::overlay_relationship)
            .or_else(|| self.follow_baselines.get(user_id).map(Self::baseline_relationship))
    }

    pub fn optimistic_follow_relationship(&self, user_id: &str, fallback: UserListRelationship) -> UserListRelationship {
        self.overlay_or_baseline_relationship(user_id).unwrap_or(fallback)
    }

    pub fn capture_follow_baseline_if_needed(
        &mut self,
        user_id: &str,
        fallback: Option<UserListRelationship>,
        cx: &mut Context<Self>,
    ) {
        if self.follow_baselines.contains_key(user_id) {
            return;
        }
        let model = self.fetched_foreign_profile(user_id, cx);
        let me = Self::session_profile(cx);
        if self.follow_my_counts_base.is_none()
            && let Some(me) = me.as_ref()
        {
            self.follow_my_counts_base = Some(FollowMyCountsBase { following: me.subs_count, friends: me.friends_count });
        }
        let baseline = UserServicesStoreFollowBaseline {
            subbed: fallback.map(|f| f.is_subbed).or(model.as_ref().map(|m| m.is_subbed)).unwrap_or(false),
            friend: fallback.map(|f| f.is_friend).or(model.as_ref().map(|m| m.is_friend)).unwrap_or(false),
            t_follows_me: fallback.map(|f| f.is_subscriber).or(model.as_ref().map(|m| m.is_subscriber)).unwrap_or(false),
            their_followers: model.as_ref().map_or(0, |m| m.subscribers_count),
            their_friends: model.as_ref().map_or(0, |m| m.friends_count),
            my_following: me.as_ref().map_or(0, |me| me.subs_count),
            my_friends: me.as_ref().map_or(0, |me| me.friends_count),
        };
        self.follow_baselines.insert(user_id.to_owned(), baseline);
        cx.notify();
    }

    pub fn note_follow_profile_tag(&mut self, user_id: &str, tag: &str, cx: &mut Context<Self>) {
        let normalized = trimming_whitespaces_and_newlines(tag);
        if !normalized.is_empty() {
            self.follow_profile_tags.insert(user_id.to_owned(), normalized.to_owned());
            cx.notify();
        }
    }

    fn fetched_foreign_profile(&self, user_id: &str, cx: &mut App) -> Option<ProfileModel> {
        if let Some(model) = self.cached_foreign_profile_user_id(user_id, None, cx).and_then(|r| r.as_profile_model()) {
            return Some(model);
        }
        let actions = UserActionsStore::slices(cx);
        let by_tag = actions.get_profile_by_tag_sai.data().and_then(|r| r.as_profile_model());
        if let Some(model) = by_tag.filter(|model| model.id == user_id) {
            return Some(model);
        }
        actions
            .get_profile_by_id_sai
            .data()
            .and_then(|r| r.as_profile_model())
            .filter(|model| model.id == user_id)
    }

    pub fn apply_follow_optimistic(&mut self, user_id: &str, following: bool, cx: &mut Context<Self>) {
        let Some(baseline) = self.follow_baselines.get(user_id).copied() else {
            return;
        };
        let follow_delta = i64::from(following) - i64::from(baseline.subbed);
        let new_friend = following && baseline.t_follows_me;
        let friend_delta = i64::from(new_friend) - i64::from(baseline.friend);
        let overlay = UserServicesStoreFollowOverlay {
            is_subbed: following,
            is_subscriber: baseline.t_follows_me,
            is_friend: new_friend,
            subscribers_count: (baseline.their_followers + follow_delta).max(0),
            friends_count: (baseline.their_friends + friend_delta).max(0),
        };
        self.follow_overlays.insert(user_id.to_owned(), overlay);
        self.follow_pending.insert(user_id.to_owned());
        self.recompute_my_follow_counts(cx);
        self.apply_follow_caches(user_id, overlay, cx);
        cx.notify();
    }

    fn recompute_my_follow_counts(&self, cx: &mut App) {
        let Some(base) = self.follow_my_counts_base else {
            return;
        };
        let Some(mut me) = Self::session_profile(cx) else {
            return;
        };
        let mut following = base.following;
        let mut friends = base.friends;
        for (user_id, overlay) in &self.follow_overlays {
            let Some(baseline) = self.follow_baselines.get(user_id) else {
                continue;
            };
            following += i64::from(overlay.is_subbed) - i64::from(baseline.subbed);
            friends += i64::from(overlay.is_friend) - i64::from(baseline.friend);
        }
        me.subs_count = following.max(0);
        me.friends_count = friends.max(0);
        Self::apply_session_profile(me, cx);
    }

    fn affected_follow_scopes(&self, user_id: &str, cx: &mut App) -> Vec<UserRelationshipListCacheScope> {
        let Some(viewer_id) = Self::effective_user_id_for_wire(cx).filter(|id| !id.is_empty()) else {
            return self.registered_list_cache_scopes.clone();
        };
        let mut scopes = self.registered_list_cache_scopes.clone();
        let defaults = [
            UserRelationshipListCacheScope::new(UserListKind::Subscriptions, viewer_id.clone(), None),
            UserRelationshipListCacheScope::new(UserListKind::Friends, viewer_id, None),
            UserRelationshipListCacheScope::new(UserListKind::Subscribers, user_id.to_owned(), None),
            UserRelationshipListCacheScope::new(UserListKind::Friends, user_id.to_owned(), None),
        ];
        for scope in defaults {
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        scopes
    }

    fn scope_cache_key(scope: &UserRelationshipListCacheScope) -> String {
        Self::list_cache_key_normalized(scope.kind, &scope.owner_id, scope.query.as_deref())
    }

    fn apply_follow_caches(&mut self, user_id: &str, overlay: UserServicesStoreFollowOverlay, cx: &mut App) {
        let Some(viewer_id) = Self::effective_user_id_for_wire(cx).filter(|id| !id.is_empty()) else {
            return;
        };
        let cache_scopes: HashMap<String, UserRelationshipListCacheScope> = self
            .affected_follow_scopes(user_id, cx)
            .into_iter()
            .map(|scope| (Self::scope_cache_key(&scope), scope))
            .collect();

        let mut profile_keys = vec![Self::profile_by_id_cache_key(user_id, &viewer_id)];
        if let Some(tag) = self.follow_profile_tags.get(user_id) {
            profile_keys.push(Self::profile_by_tag_cache_key(tag));
        }
        if !self.follow_cache_snapshots.contains_key(user_id) {
            let all_keys: HashSet<String> = cache_scopes.keys().cloned().chain(profile_keys).collect();
            let mut snapshot = HashMap::new();
            for cache_key in all_keys {
                if let Some(root) = Self::current_root(&cache_key, cx) {
                    snapshot.insert(cache_key, root);
                }
            }
            self.follow_cache_snapshots.insert(user_id.to_owned(), snapshot);
        }
        let Some(snapshot) = self.follow_cache_snapshots.get(user_id).cloned() else {
            return;
        };
        for (cache_key, original) in snapshot {
            let patched = match cache_scopes.get(&cache_key) {
                Some(scope) => self.patch_follow_list_root(original, scope, user_id, &viewer_id, overlay, cx),
                None => Self::patch_follow_profile_root(original, overlay),
            };
            let _ = SaiUpdater::replace_root(&cache_key, patched, SaiUpdaterWriteScope::All, cx);
        }
        Self::mirror_follow_to_author_rows(user_id, overlay, cx);
    }

    fn current_root(cache_key: &str, cx: &mut App) -> Option<Map<String, Value>> {
        let payload = SaiWsResponseCache::current_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        AnyDictionaryBridge::dictionary(payload.as_ref())
    }

    fn mirror_follow_to_author_rows(user_id: &str, overlay: UserServicesStoreFollowOverlay, cx: &mut App) {
        Self::write_author_relationship(user_id, Self::overlay_relationship(&overlay), cx);
    }

    fn write_author_relationship(user_id: &str, relationship: UserListRelationship, cx: &mut App) {
        UserServicesHooks::set_author_relationship(user_id, relationship, cx);
        UserServicesHooks::set_author_relationship_on_held_posts(user_id, relationship, cx);
        UserServicesHooks::set_like_user_relationship(user_id, relationship, cx);
        UserServicesHooks::set_author_relationship_action(user_id, relationship, cx);
    }

    fn patch_follow_profile_root(original: Map<String, Value>, overlay: UserServicesStoreFollowOverlay) -> Map<String, Value> {
        let mut root = original;
        root.insert(key::IS_SUBSCRIBED.to_owned(), Value::Bool(overlay.is_subbed));
        root.insert(key::IS_SUBBED.to_owned(), Value::Bool(overlay.is_subbed));
        root.insert(key::IS_SUBSCRIBER.to_owned(), Value::Bool(overlay.is_subscriber));
        root.insert(key::IS_FRIEND.to_owned(), Value::Bool(overlay.is_friend));
        if let Some(mut profile) = AnyDictionaryBridge::dictionary(root.get(key::PROFILE)) {
            profile.insert(key::IS_SUBBED.to_owned(), Value::Bool(overlay.is_subbed));
            profile.insert(key::IS_SUBSCRIBER.to_owned(), Value::Bool(overlay.is_subscriber));
            profile.insert(key::IS_FRIEND.to_owned(), Value::Bool(overlay.is_friend));
            profile.insert(key::SUBSCRIBERS_COUNT.to_owned(), Value::from(overlay.subscribers_count));
            profile.insert(key::FRIENDS_COUNT.to_owned(), Value::from(overlay.friends_count));
            root.insert(key::PROFILE.to_owned(), Value::Object(profile));
        }
        if let Some(mut user) = AnyDictionaryBridge::dictionary(root.get(key::USER)) {
            user.insert(key::SUBSCRIBERS_COUNT.to_owned(), Value::from(overlay.subscribers_count));
            user.insert(key::FRIENDS_COUNT.to_owned(), Value::from(overlay.friends_count));
            root.insert(key::USER.to_owned(), Value::Object(user));
        }
        if let Some(mut more) = AnyDictionaryBridge::dictionary(root.get(key::MORE)) {
            more.insert(key::SUBSCRIBERS.to_owned(), Value::from(overlay.subscribers_count));
            more.insert(key::FRIENDS.to_owned(), Value::from(overlay.friends_count));
            root.insert(key::MORE.to_owned(), Value::Object(more));
        }
        root
    }

    fn patch_follow_list_root(
        &self,
        original: Map<String, Value>,
        scope: &UserRelationshipListCacheScope,
        target_user_id: &str,
        viewer_id: &str,
        overlay: UserServicesStoreFollowOverlay,
        cx: &mut App,
    ) -> Map<String, Value> {
        let mut root = original;
        let mut items = AnyDictionaryBridge::array_of_dictionaries(root.get(key::ITEMS));

        for item in items.iter_mut() {
            if Self::follow_list_row_user_id(item).as_deref() != Some(target_user_id) {
                continue;
            }
            item.insert(key::VIEWER_IS_SUBBED.to_owned(), Value::Bool(overlay.is_subbed));
            item.insert(key::VIEWER_IS_SUBSCRIBER.to_owned(), Value::Bool(overlay.is_subscriber));
            item.insert(key::VIEWER_IS_FRIEND.to_owned(), Value::Bool(overlay.is_friend));
            item.insert(key::IS_SUBBED.to_owned(), Value::Bool(overlay.is_subbed));
            item.insert(key::IS_SUBSCRIBED.to_owned(), Value::Bool(overlay.is_subbed));
            item.insert(key::IS_SUBSCRIBER.to_owned(), Value::Bool(overlay.is_subscriber));
            item.insert(key::IS_FRIEND.to_owned(), Value::Bool(overlay.is_friend));
            if let Some(mut user) = AnyDictionaryBridge::dictionary(item.get(key::USER)) {
                user.insert(key::VIEWER_IS_SUBBED.to_owned(), Value::Bool(overlay.is_subbed));
                user.insert(key::VIEWER_IS_SUBSCRIBER.to_owned(), Value::Bool(overlay.is_subscriber));
                user.insert(key::VIEWER_IS_FRIEND.to_owned(), Value::Bool(overlay.is_friend));
                user.insert(key::IS_SUBBED.to_owned(), Value::Bool(overlay.is_subbed));
                user.insert(key::IS_SUBSCRIBED.to_owned(), Value::Bool(overlay.is_subbed));
                user.insert(key::IS_SUBSCRIBER.to_owned(), Value::Bool(overlay.is_subscriber));
                item.insert(key::USER.to_owned(), Value::Object(user));
            }
        }

        let membership = if scope.owner_id == viewer_id && scope.kind == UserListKind::Subscriptions {
            Some((target_user_id.to_owned(), overlay.is_subbed))
        } else if scope.owner_id == viewer_id && scope.kind == UserListKind::Friends {
            Some((target_user_id.to_owned(), overlay.is_friend))
        } else if scope.owner_id == target_user_id && scope.kind == UserListKind::Subscribers {
            Some((viewer_id.to_owned(), overlay.is_subbed))
        } else if scope.owner_id == target_user_id && scope.kind == UserListKind::Friends {
            Some((viewer_id.to_owned(), overlay.is_friend))
        } else {
            None
        };

        if let Some((membership_user_id, should_contain)) = membership {
            let existing_index =
                items.iter().position(|row| Self::follow_list_row_user_id(row).as_deref() == Some(membership_user_id.as_str()));
            let total = root.get(key::TOTAL).and_then(Value::as_i64);
            if should_contain && existing_index.is_none() && scope.query.is_none() {
                if let Some(row) = self.follow_list_row(&membership_user_id, target_user_id, overlay, cx) {
                    items.insert(0, row);
                    let previous = total.unwrap_or(items.len() as i64 - 1);
                    root.insert(key::TOTAL.to_owned(), Value::from(previous + 1));
                }
            } else if !should_contain && let Some(existing_index) = existing_index {
                items.remove(existing_index);
                let previous = total.unwrap_or(items.len() as i64 + 1);
                root.insert(key::TOTAL.to_owned(), Value::from((previous - 1).max(0)));
            }
        }

        root.insert(key::ITEMS.to_owned(), Value::Array(items.into_iter().map(Value::Object).collect()));
        root
    }

    fn follow_list_row_user_id(row: &Map<String, Value>) -> Option<String> {
        AnyDictionaryBridge::dictionary(row.get(key::USER))
            .and_then(|user| user.get(key::ID).and_then(Value::as_str).map(str::to_owned))
            .or_else(|| row.get(key::USER_ID).and_then(Value::as_str).map(str::to_owned))
    }

    fn follow_list_row(
        &self,
        user_id: &str,
        target_user_id: &str,
        overlay: UserServicesStoreFollowOverlay,
        cx: &mut App,
    ) -> Option<Map<String, Value>> {
        let profile = if user_id == target_user_id {
            self.fetched_foreign_profile(target_user_id, cx)
        } else if Self::effective_user_id_for_wire(cx).as_deref() == Some(user_id) {
            Self::session_profile(cx)
        } else {
            None
        }?;
        let is_target = user_id == target_user_id;
        let mut more = Map::new();
        more.insert(
            key::LOGO.to_owned(),
            Value::String(profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone())),
        );
        more.insert(key::WHO.to_owned(), Value::String(profile.more.who.clone()));
        more.insert(
            key::P_LANG.to_owned(),
            Value::Array(profile.more.p_lang.iter().cloned().map(Value::String).collect()),
        );
        let mut user = Map::new();
        user.insert(key::ID.to_owned(), Value::String(profile.id.clone()));
        user.insert(key::NAME.to_owned(), Value::String(profile.name.clone()));
        user.insert(key::TAG.to_owned(), Value::String(profile.tag.clone()));
        user.insert(key::IS_PREMIUM.to_owned(), Value::Bool(profile.more.is_premium));
        user.insert(key::IS_OFFICIAL.to_owned(), Value::Bool(profile.more.is_official));
        user.insert(key::MORE.to_owned(), Value::Object(more));
        let mut row = Map::new();
        row.insert(key::ENTRY_KEY.to_owned(), Value::String(format!("optimistic:{user_id}")));
        row.insert(key::IS_FRIEND.to_owned(), Value::Bool(if is_target { overlay.is_friend } else { true }));
        row.insert(key::VIEWER_IS_SUBBED.to_owned(), Value::Bool(is_target && overlay.is_subbed));
        row.insert(key::VIEWER_IS_SUBSCRIBER.to_owned(), Value::Bool(is_target && overlay.is_subscriber));
        row.insert(key::VIEWER_IS_FRIEND.to_owned(), Value::Bool(if is_target { overlay.is_friend } else { true }));
        row.insert(key::USER.to_owned(), Value::Object(user));
        Some(row)
    }

    pub fn apply_follow_overlay(&self, model: ProfileModel) -> ProfileModel {
        let Some(overlay) = self.follow_overlays.get(&model.id) else {
            return model;
        };
        let mut model = model;
        model.is_subbed = overlay.is_subbed;
        model.is_subscriber = overlay.is_subscriber;
        model.is_friend = overlay.is_friend;
        model.subscribers_count = overlay.subscribers_count;
        model.friends_count = overlay.friends_count;
        model
    }

    pub fn sync_follow(
        &mut self,
        user_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let (Some(baseline), Some(overlay)) =
            (self.follow_baselines.get(user_id).copied(), self.follow_overlays.get(user_id).copied())
        else {
            return;
        };

        if overlay.is_subbed == baseline.subbed {
            self.restore_follow_caches(user_id, cx);
            self.clear_follow_optimistic(user_id, false, cx);
            if let Some(on_success) = on_success {
                on_success(cx);
            }
            return;
        }

        let ack_user_id = user_id.to_owned();
        let ack: Rc<dyn Fn(Option<UserListRelationship>, &mut App)> = Rc::new(move |authoritative, cx| {
            UserServicesStore::shared(cx)
                .update(cx, |services, cx| services.on_follow_ack(&ack_user_id, authoritative, cx));
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure_user_id = user_id.to_owned();
        let failure: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(move |error, cx| {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.revert_follow(&failure_user_id, cx));
            Self::notify_follow_failure(error, cx);
        });
        let target_id = user_id.to_owned();
        let actions = UserActionsStore::shared(cx);
        if overlay.is_subbed {
            actions.update(cx, |actions, cx| actions.follow_user_action(&target_id, Some(ack), Some(failure), cx));
        } else {
            actions.update(cx, |actions, cx| actions.unfollow_user_action(&target_id, Some(ack), Some(failure), cx));
        }
    }

    fn on_follow_ack(&mut self, user_id: &str, authoritative: Option<UserListRelationship>, cx: &mut Context<Self>) {
        let previous_baseline = self.follow_baselines.get(user_id).copied();
        if let Some(mut overlay) = self.follow_overlays.get(user_id).copied()
            && let Some(authoritative) = authoritative
        {
            overlay.is_subbed = authoritative.is_subbed;
            overlay.is_subscriber = authoritative.is_subscriber;
            overlay.is_friend = authoritative.is_friend;
            if let Some(baseline) = previous_baseline {
                let follow_delta = i64::from(overlay.is_subbed) - i64::from(baseline.subbed);
                let friend_delta = i64::from(overlay.is_friend) - i64::from(baseline.friend);
                overlay.subscribers_count = (baseline.their_followers + follow_delta).max(0);
                overlay.friends_count = (baseline.their_friends + friend_delta).max(0);
            }
            self.follow_overlays.insert(user_id.to_owned(), overlay);
            self.apply_follow_caches(user_id, overlay, cx);
        }
        if let Some(baseline) = previous_baseline
            && let Some(overlay) = self.follow_overlays.get(user_id)
            && let Some(mut base) = self.follow_my_counts_base
        {
            base.following += i64::from(overlay.is_subbed) - i64::from(baseline.subbed);
            base.friends += i64::from(overlay.is_friend) - i64::from(baseline.friend);
            self.follow_my_counts_base = Some(base);
        }
        self.follow_cache_snapshots.remove(user_id);
        self.follow_pending.remove(user_id);
        if !self.follow_pending.is_empty()
            && let Some(overlay) = self.follow_overlays.get(user_id).copied()
        {
            let tag = self.follow_profile_tags.get(user_id).cloned();
            self.committed_follow_overlays.insert(user_id.to_owned(), (overlay, tag));
        }
        self.follow_overlays.remove(user_id);
        self.follow_baselines.remove(user_id);
        self.follow_profile_tags.remove(user_id);
        self.recompute_my_follow_counts(cx);
        self.reapply_concurrent_follow_cache_patches(cx);
        self.finish_follow_batch_if_idle();
        cx.notify();
    }

    fn finish_follow_batch_if_idle(&mut self) {
        if self.follow_pending.is_empty() {
            self.committed_follow_overlays.clear();
            self.follow_my_counts_base = None;
        }
    }

    fn notify_follow_failure(error: &anyhow::Error, cx: &mut App) {
        if error.downcast_ref::<CancellationError>().is_some() {
            return;
        }
        if CanonErrors::failure(error).is_some_and(|failure| failure.subscription_lifts_limit()) {
            return;
        }
        Self::notify_error(CanonErrors::refusal_text(error), cx);
    }

    fn revert_follow(&mut self, user_id: &str, cx: &mut Context<Self>) {
        self.restore_follow_caches(user_id, cx);
        self.follow_pending.remove(user_id);
        self.follow_overlays.remove(user_id);
        self.follow_baselines.remove(user_id);
        self.follow_profile_tags.remove(user_id);
        self.recompute_my_follow_counts(cx);
        self.reapply_concurrent_follow_cache_patches(cx);
        self.finish_follow_batch_if_idle();
        cx.notify();
    }

    fn restore_follow_caches(&mut self, user_id: &str, cx: &mut App) {
        if let Some(baseline) = self.follow_baselines.get(user_id) {
            Self::write_author_relationship(user_id, Self::baseline_relationship(baseline), cx);
        }
        let Some(snapshot) = self.follow_cache_snapshots.remove(user_id) else {
            return;
        };
        for (cache_key, root) in snapshot {
            let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx);
        }
    }

    pub fn clear_follow_optimistic(&mut self, user_id: &str, restore_my_counts: bool, cx: &mut Context<Self>) {
        if restore_my_counts {
            self.restore_follow_caches(user_id, cx);
        }
        self.follow_overlays.remove(user_id);
        self.follow_baselines.remove(user_id);
        self.follow_pending.remove(user_id);
        self.follow_profile_tags.remove(user_id);
        self.recompute_my_follow_counts(cx);
        self.reapply_concurrent_follow_cache_patches(cx);
        self.finish_follow_batch_if_idle();
        cx.notify();
    }

    fn reapply_concurrent_follow_cache_patches(&self, cx: &mut App) {
        for (user_id, (overlay, tag)) in &self.committed_follow_overlays {
            self.apply_follow_caches_on_current_root(user_id, *overlay, tag.as_deref(), cx);
        }
        for (user_id, overlay) in &self.follow_overlays {
            if self.follow_pending.contains(user_id) {
                let tag = self.follow_profile_tags.get(user_id).map(String::as_str);
                self.apply_follow_caches_on_current_root(user_id, *overlay, tag, cx);
            }
        }
    }

    fn apply_follow_caches_on_current_root(
        &self,
        user_id: &str,
        overlay: UserServicesStoreFollowOverlay,
        tag: Option<&str>,
        cx: &mut App,
    ) {
        let Some(viewer_id) = Self::effective_user_id_for_wire(cx).filter(|id| !id.is_empty()) else {
            return;
        };
        let scopes_by_key: HashMap<String, UserRelationshipListCacheScope> = self
            .affected_follow_scopes(user_id, cx)
            .into_iter()
            .map(|scope| (Self::scope_cache_key(&scope), scope))
            .collect();
        let mut profile_keys = vec![Self::profile_by_id_cache_key(user_id, &viewer_id)];
        if let Some(tag) = tag.filter(|tag| !tag.is_empty()) {
            profile_keys.push(Self::profile_by_tag_cache_key(tag));
        }

        for (cache_key, scope) in &scopes_by_key {
            let Some(root) = Self::current_root(cache_key, cx) else {
                continue;
            };
            let patched = self.patch_follow_list_root(root, scope, user_id, &viewer_id, overlay, cx);
            let _ = SaiUpdater::replace_root(cache_key, patched, SaiUpdaterWriteScope::All, cx);
        }
        for cache_key in profile_keys {
            let Some(root) = Self::current_root(&cache_key, cx) else {
                continue;
            };
            let _ =
                SaiUpdater::replace_root(&cache_key, Self::patch_follow_profile_root(root, overlay), SaiUpdaterWriteScope::All, cx);
        }
        Self::mirror_follow_to_author_rows(user_id, overlay, cx);
    }

    pub fn clear_follow_overlay_on_fetch(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if self.follow_pending.contains(user_id) {
            return;
        }
        self.follow_overlays.remove(user_id);
        self.follow_baselines.remove(user_id);
        cx.notify();
    }

    pub fn list_cache_key(kind: UserListKind, user_id: &str, search: Option<&str>) -> String {
        Self::list_cache_key_normalized(kind, user_id, Self::normalized_list_search(search).as_deref())
    }

    pub fn list_cache_key_normalized(kind: UserListKind, user_id: &str, normalized_query: Option<&str>) -> String {
        let method = match kind {
            UserListKind::Friends => "getUserFriends",
            UserListKind::Subscribers => "getUserSubs",
            UserListKind::Subscriptions => "getUserSubscriptions",
        };
        match normalized_query {
            Some(query) => SaiCache::new([method, user_id, "q", query]).key,
            None => SaiCache::new([method, user_id]).key,
        }
    }

    pub fn register_list_cache_scope(
        &mut self,
        kind: UserListKind,
        user_id: &str,
        search: Option<&str>,
        cx: &mut Context<Self>,
    ) -> String {
        let scope = UserRelationshipListCacheScope::new(kind, user_id.to_owned(), search);
        let evicted = UserRelationshipListCacheRegistryContract::record(&scope, &mut self.registered_list_cache_scopes);
        let evicted_any = !evicted.is_empty();
        for evicted_scope in evicted {
            let cache_key = Self::scope_cache_key(&evicted_scope);
            SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
            self.failed_list_cache_keys.remove(&cache_key);
        }
        if evicted_any {
            cx.notify();
        }
        Self::scope_cache_key(&scope)
    }

    pub fn normalized_list_query(search: Option<&str>) -> Option<String> {
        UserRelationshipSearchContract::outbound_query(search)
    }

    pub fn list_entries(&self, kind: UserListKind, user_id: &str, search: Option<&str>, cx: &mut App) -> Vec<UserListEntry> {
        match kind {
            UserListKind::Friends => self
                .friend_list_dto(user_id, search, true, cx)
                .map(|dto| dto.items.iter().filter_map(FriendDtoExt::as_user_list_entry).collect())
                .unwrap_or_default(),
            UserListKind::Subscribers | UserListKind::Subscriptions => self
                .subs_list_dto(kind, user_id, search, true, cx)
                .map(|dto| dto.items.iter().filter_map(UserSubscriptionDtoExt::as_user_list_entry).collect())
                .unwrap_or_default(),
        }
    }

    pub fn resolved_list_relationship(
        &self,
        entry: &UserListEntry,
        kind: UserListKind,
        owner_id: &str,
        cx: &mut App,
    ) -> UserListRelationship {
        if let Some(overlay) = entry.author.id.as_ref().and_then(|user_id| self.follow_overlays.get(user_id)) {
            return Self::overlay_relationship(overlay);
        }
        let viewer_id = Self::session_profile(cx).map(|profile| profile.id).unwrap_or_default();
        entry.resolved_relationship(kind, !viewer_id.is_empty() && viewer_id == owner_id)
    }

    pub fn list_render_status(
        &self,
        kind: UserListKind,
        user_id: &str,
        search: Option<&str>,
        cx: &mut App,
    ) -> SaiWsSliceStatus {
        let cache_key = Self::list_cache_key(kind, user_id, search);
        let slice_status = Self::list_slice_status(kind, cx);
        let has_payload = SaiWsResponseCache::observed_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
        .is_some();
        let is_pending = Self::has_in_flight_request(&cache_key, cx);
        UserRelationshipListRenderContract::status(
            has_payload,
            is_pending,
            self.failed_list_cache_keys.contains(&cache_key),
            slice_status,
        )
    }

    pub fn clear_list_failure(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        if self.failed_list_cache_keys.remove(cache_key) {
            cx.notify();
        }
    }

    pub fn mark_list_failure(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        UserRelationshipFailureCacheContract::record(cache_key, &mut self.failed_list_cache_keys);
        cx.notify();
    }

    pub fn is_follow_interaction_pending(&self, user_id: &str) -> bool {
        self.follow_pending.contains(user_id)
    }

    pub fn list_has_more(&self, kind: UserListKind, user_id: &str, search: Option<&str>, cx: &mut App) -> bool {
        match kind {
            UserListKind::Friends => self.friend_list_dto(user_id, search, true, cx).is_some_and(|dto| dto.has_more),
            UserListKind::Subscribers | UserListKind::Subscriptions => {
                self.subs_list_dto(kind, user_id, search, true, cx).is_some_and(|dto| dto.has_more)
            }
        }
    }

    pub fn list_continuation(
        &self,
        kind: UserListKind,
        user_id: &str,
        search: Option<&str>,
        cx: &mut App,
    ) -> Option<UserListContinuation> {
        match kind {
            UserListKind::Friends => {
                let dto = self.friend_list_dto(user_id, search, true, cx);
                let legacy = dto.as_ref().and_then(|dto| dto.relative_id.or_else(|| dto.items.last().map(|item| item.id)));
                UserListContinuation::resolve(dto.as_ref().and_then(|dto| dto.cursor.as_deref()), legacy)
            }
            UserListKind::Subscribers | UserListKind::Subscriptions => {
                let dto = self.subs_list_dto(kind, user_id, search, true, cx);
                let legacy = dto.as_ref().and_then(|dto| {
                    dto.relative_id.or_else(|| dto.items.last().and_then(|item| item.id.to_string().parse::<i64>().ok()))
                });
                UserListContinuation::resolve(dto.as_ref().and_then(|dto| dto.cursor.as_deref()), legacy)
            }
        }
    }

    pub fn list_did_load(&self, kind: UserListKind, user_id: &str, search: Option<&str>, cx: &mut App) -> bool {
        Self::list_payload(kind, user_id, search, false, cx).is_some()
    }

    pub fn list_slice(kind: UserListKind, cx: &mut App) -> Rc<dyn SaiSliceState> {
        let actions = UserActionsStore::slices(cx);
        match kind {
            UserListKind::Friends => actions.get_user_friends_sai.clone() as Rc<dyn SaiSliceState>,
            UserListKind::Subscribers => actions.get_user_subs_sai.clone() as Rc<dyn SaiSliceState>,
            UserListKind::Subscriptions => actions.get_user_subscriptions_sai.clone() as Rc<dyn SaiSliceState>,
        }
    }

    fn list_slice_status(kind: UserListKind, cx: &mut App) -> SaiWsSliceStatus {
        Self::list_slice(kind, cx).status()
    }

    fn friend_list_dto(
        &self,
        user_id: &str,
        search: Option<&str>,
        observed: bool,
        cx: &mut App,
    ) -> Option<FriendListGatewayDTO> {
        let payload = Self::list_payload(UserListKind::Friends, user_id, search, observed, cx)?;
        SaiJSONCodec::decode::<FriendListGatewayDTO>(Some(&payload))
    }

    fn subs_list_dto(
        &self,
        kind: UserListKind,
        user_id: &str,
        search: Option<&str>,
        observed: bool,
        cx: &mut App,
    ) -> Option<UserSubsListGatewayDTO> {
        let payload = Self::list_payload(kind, user_id, search, observed, cx)?;
        SaiJSONCodec::decode::<UserSubsListGatewayDTO>(Some(&payload))
    }

    fn list_payload(kind: UserListKind, user_id: &str, search: Option<&str>, observed: bool, cx: &mut App) -> Option<Value> {
        let cache_key = Self::list_cache_key(kind, user_id, search);
        if observed {
            return SaiWsResponseCache::observed_payload(
                &cache_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            );
        }
        SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
    }

    fn normalized_list_search(search: Option<&str>) -> Option<String> {
        UserRelationshipSearchContract::cache_query(search)
    }

    pub fn goal(&self, id: &str) -> Option<&GoalItem> {
        self.goals.iter().find(|goal| goal.id == id)
    }

    pub fn replace_goals_from_wire(&mut self, entries: &[UserGoalWireEntry], cx: &mut Context<Self>) {
        let empty_checklist_ids: HashSet<String> = self
            .goals
            .iter()
            .filter(|goal| goal.is_checklist && goal.subgoals.is_empty())
            .map(|goal| goal.id.clone())
            .collect();
        self.suppress_goals_sync = true;
        self.goals = entries
            .iter()
            .map(|entry| {
                let mut item = entry.as_goal_item();
                if !item.is_checklist && empty_checklist_ids.contains(&item.id) {
                    item.is_checklist = true;
                    item.progress = item.checklist_progress();
                }
                item
            })
            .collect();
        self.goals_sync_snapshot = None;
        self.suppress_goals_sync = false;
        cx.notify();
    }

    pub fn add_goal(&mut self, title: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(title).to_owned();
        if trimmed.is_empty() {
            return;
        }
        self.mutate_goals(cx, |goals| {
            goals.push(GoalItem {
                id: Uuid::new_v4().uuid_string(),
                title: trimmed,
                progress: 0.0,
                description: String::new(),
                is_checklist: false,
                subgoals: Vec::new(),
            });
        });
        GuideSignalBus::emit(RiseGuideSignal::GoalAdded);
    }

    pub fn update_goal(&mut self, id: &str, title: Option<String>, progress: Option<f64>, cx: &mut Context<Self>) {
        let Some(index) = self.goals.iter().position(|goal| goal.id == id) else {
            return;
        };
        let progress_changed = progress.is_some();
        self.mutate_goals(cx, |goals| {
            if let Some(title) = title {
                goals[index].title = title;
            }
            if let Some(progress) = progress {
                goals[index].progress = progress.clamp(0.0, 1.0);
            }
        });
        if progress_changed {
            GuideSignalBus::emit(RiseGuideSignal::GoalProgressChanged);
        }
    }

    pub fn delete_goal(&mut self, id: &str, cx: &mut Context<Self>) {
        self.mutate_goals(cx, |goals| goals.retain(|goal| goal.id != id));
    }

    pub fn update_goal_description(&mut self, id: &str, description: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(description).to_owned();
        self.mutate_goal(id, cx, |goal| goal.description = trimmed);
    }

    pub fn set_goal_checklist(&mut self, id: &str, is_checklist: bool, cx: &mut Context<Self>) {
        self.mutate_goal(id, cx, |goal| {
            goal.is_checklist = is_checklist;
            if !is_checklist {
                goal.subgoals.clear();
            }
        });
    }

    pub fn add_subgoal(&mut self, goal_id: &str, title: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(title).to_owned();
        if trimmed.is_empty() || self.goal(goal_id).map_or(0, |goal| goal.subgoals.len()) >= Self::max_goal_subgoals() {
            return;
        }

        self.mutate_goal(goal_id, cx, |goal| {
            goal.subgoals.push(GoalSubgoalItem { id: Uuid::new_v4().uuid_string(), title: trimmed, done: false });
        });
    }

    pub fn rename_subgoal(&mut self, goal_id: &str, subgoal_id: &str, title: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(title).to_owned();
        if trimmed.is_empty() {
            return;
        }

        self.mutate_goal(goal_id, cx, |goal| {
            if let Some(subgoal) = goal.subgoals.iter_mut().find(|subgoal| subgoal.id == subgoal_id) {
                subgoal.title = trimmed;
            }
        });
    }

    pub fn toggle_subgoal(&mut self, goal_id: &str, subgoal_id: &str, cx: &mut Context<Self>) {
        self.mutate_goal(goal_id, cx, |goal| {
            if let Some(subgoal) = goal.subgoals.iter_mut().find(|subgoal| subgoal.id == subgoal_id) {
                subgoal.done = !subgoal.done;
            }
        });
        GuideSignalBus::emit(RiseGuideSignal::GoalProgressChanged);
    }

    pub fn delete_subgoal(&mut self, goal_id: &str, subgoal_id: &str, cx: &mut Context<Self>) {
        self.mutate_goal(goal_id, cx, |goal| goal.subgoals.retain(|subgoal| subgoal.id != subgoal_id));
    }

    /// puts the dragged subgoal where the target one stands
    pub fn move_subgoal(&mut self, goal_id: &str, subgoal_id: &str, before_target_id: &str, cx: &mut Context<Self>) {
        if subgoal_id == before_target_id {
            return;
        }

        self.mutate_goal(goal_id, cx, |goal| {
            let from = goal.subgoals.iter().position(|subgoal| subgoal.id == subgoal_id);
            let to = goal.subgoals.iter().position(|subgoal| subgoal.id == before_target_id);
            let (Some(from), Some(to)) = (from, to) else {
                return;
            };
            let item = goal.subgoals.remove(from);
            goal.subgoals.insert(to, item);
        });
    }

    /// server ceiling on subgoals per goal
    pub fn max_goal_subgoals() -> usize {
        usize::try_from(RiseLimits::service_value("max_goal_steps")).unwrap_or(0)
    }

    fn mutate_goal(&mut self, id: &str, cx: &mut Context<Self>, change: impl FnOnce(&mut GoalItem)) {
        let Some(index) = self.goals.iter().position(|goal| goal.id == id) else {
            return;
        };

        self.mutate_goals(cx, |goals| {
            change(&mut goals[index]);
            if goals[index].is_checklist {
                goals[index].progress = goals[index].checklist_progress();
            }
        });
    }

    pub fn restore_goals_and_plans(&mut self, goals: Vec<GoalItem>, plans: Vec<PlanItem>, cx: &mut Context<Self>) {
        self.suppress_goals_sync = true;
        self.suppress_plans_sync = true;
        self.goals = goals;
        self.plans = plans;
        self.suppress_goals_sync = false;
        self.suppress_plans_sync = false;
        cx.notify();
    }

    pub fn revert_goals_if_pending_sync(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.goals_sync_snapshot.take() else {
            return;
        };
        self.suppress_goals_sync = true;
        self.goals = snapshot;
        self.suppress_goals_sync = false;
        cx.notify();
    }

    pub fn clear_goals_sync_snapshot(&mut self, cx: &mut Context<Self>) {
        self.goals_sync_snapshot = None;
        cx.notify();
    }

    fn mutate_goals(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Vec<GoalItem>)) {
        if self.suppress_goals_sync || (user_guide_sandbox_hooks().owns_goals_scene)(cx) {
            change(&mut self.goals);
            cx.notify();
            return;
        }
        if self.goals_sync_snapshot.is_none() {
            self.goals_sync_snapshot = Some(self.goals.clone());
        }
        change(&mut self.goals);
        cx.notify();
        Self::schedule_profile_sync(GOALS_SYNC_DEBOUNCE_ID, GOALS_SYNC_GROUP, ProfileSync::Goals, cx);
    }

    fn schedule_profile_sync(id: &'static str, group: &'static str, sync: ProfileSync, cx: &mut Context<Self>) {
        cx.defer(move |cx| {
            let options = SaiDebouncerOptions {
                delay_milliseconds: PROFILE_SYNC_DEBOUNCE_MILLISECONDS,
                group_key: group.to_owned(),
                ..SaiDebouncerOptions::default()
            };
            let action = Box::new(move |cx: &mut App| {
                UserActionsStore::shared(cx).update(cx, |actions, cx| match sync {
                    ProfileSync::Goals => actions.sync_profile_goals_action(cx),
                    ProfileSync::Plans => actions.sync_profile_plans_action(cx),
                });
            });
            let _ = SaiDebouncer::debounced_action_id(id, action, options, cx);
        });
    }

    pub fn is_plans_sync_pending(&self) -> bool {
        self.plans_sync_snapshot.is_some()
    }

    pub fn is_goals_sync_pending(&self) -> bool {
        self.goals_sync_snapshot.is_some()
    }

    pub fn plan(&self, id: &str) -> Option<&PlanItem> {
        self.plans.iter().find(|plan| plan.id == id)
    }

    pub fn replace_plans_from_wire(&mut self, entries: &[UserPlanWireEntry], cx: &mut Context<Self>) {
        self.suppress_plans_sync = true;
        self.plans = entries.iter().map(UserPlanWireEntryExt::as_plan_item).collect();
        self.plans_sync_snapshot = None;
        self.suppress_plans_sync = false;
        cx.notify();
    }

    pub fn add_plan(&mut self, title: &str, date_ms: i64, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(title).to_owned();
        if trimmed.is_empty() {
            return;
        }
        self.mutate_plans(cx, |plans| {
            plans.push(PlanItem { id: Uuid::new_v4().uuid_string(), title: trimmed, date_ms, description: String::new() });
        });
        GuideSignalBus::emit(RiseGuideSignal::PlanAdded);
    }

    pub fn update_plan(&mut self, id: &str, title: Option<String>, date_ms: Option<i64>, cx: &mut Context<Self>) {
        let Some(index) = self.plans.iter().position(|plan| plan.id == id) else {
            return;
        };
        self.mutate_plans(cx, |plans| {
            if let Some(title) = title {
                plans[index].title = title;
            }
            if let Some(date_ms) = date_ms {
                plans[index].date_ms = date_ms;
            }
        });
    }

    pub fn update_plan_description(&mut self, id: &str, description: &str, cx: &mut Context<Self>) {
        let Some(index) = self.plans.iter().position(|plan| plan.id == id) else {
            return;
        };
        let trimmed = trimming_whitespaces_and_newlines(description).to_owned();
        if self.plans[index].description == trimmed {
            return;
        }

        self.mutate_plans(cx, |plans| plans[index].description = trimmed);
    }

    pub fn delete_plan(&mut self, id: &str, cx: &mut Context<Self>) {
        self.mutate_plans(cx, |plans| plans.retain(|plan| plan.id != id));
    }

    pub fn revert_plans_if_pending_sync(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.plans_sync_snapshot.take() else {
            return;
        };
        self.suppress_plans_sync = true;
        self.plans = snapshot;
        self.suppress_plans_sync = false;
        cx.notify();
    }

    pub fn clear_plans_sync_snapshot(&mut self, cx: &mut Context<Self>) {
        self.plans_sync_snapshot = None;
        cx.notify();
    }

    fn mutate_plans(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Vec<PlanItem>)) {
        if self.suppress_plans_sync || (user_guide_sandbox_hooks().owns_goals_scene)(cx) {
            change(&mut self.plans);
            cx.notify();
            return;
        }
        if self.plans_sync_snapshot.is_none() {
            self.plans_sync_snapshot = Some(self.plans.clone());
        }
        change(&mut self.plans);
        cx.notify();
        Self::schedule_profile_sync(PLANS_SYNC_DEBOUNCE_ID, PLANS_SYNC_GROUP, ProfileSync::Plans, cx);
    }
}

#[derive(Clone, Copy)]
enum ProfileSync {
    Goals,
    Plans,
}
