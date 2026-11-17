use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{AbortHandle, Abortable};
use gpui::{App, AppContext, AsyncApp, Context, Entity, Pixels, Point, Task, WeakEntity};
use rise_canon::{RiseGuideSignal, RiseLimits, RiseTimingCanon};
use rise_canon::rise_request::map::UpdatePhotoGrid;
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_media::map_view::LatLng;
use rise_platform::pasteboard::UIPasteboard;

use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierStore, NotifierType,
};
use crate::core::widgets::sheets::people_picker_sheet::{PeoplePickerHostContext, PeoplePickerMode, PeoplePickerStore};
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::map::components::globe::map_globe_camera_link::MapGlobeCameraLink;
use crate::modules::map::components::globe::map_globe_layer::{MapCameraRequest, MapGlobeEntry};
use crate::modules::map::components::map_editing_session::MapEditingSession;
use crate::modules::map::components::reactions::map_reaction_stage::MapReactionStage;
use crate::modules::map::components::rise_map_view::RiseMapCamera;
use crate::modules::map::components::tracks::map_track_layer::MapTrackLayer;
use crate::modules::map::components::tracks::map_tracks_panel::MapPanelMode;
use crate::modules::map::engine::core::rise_map_photo_upload::{
    MapPhotoMetadata, MapPhotoMetadataReading, MapPhotoUploadError, MapPhotoUploadResult, RiseMapPhotoUpload,
    RiseMapPhotoUploadProgress,
};
use crate::modules::map::engine::core::rise_map_resource_repository::RiseMapFailure;
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapFriendRequestResult, RiseMapInvitePreviewResult, RiseMapMergePreviewResult, RiseMapMutationOutcome,
};
use crate::modules::map::engine::rise_map_friends_repository::RiseMapFriendsRepositoryList;
use crate::modules::map::engine::rise_map_journeys_repository::{MapJourneySelection, MapJourneySnapshot};
use crate::modules::map::engine::rise_map_photos_repository::RiseMapPhotosSnapshot;
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::engine::rise_map_profiles_repository::{MapProfile, MapProfileSnapshot, MapProfileTab};
use crate::modules::map::recorder::map_recorder_journal::MapRecorderJournal;
use crate::modules::map::recorder::map_self_location_feed::MapSelfLocationFeed;
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::recorder::rise_map_recorder::RiseMapRecorder;
use crate::modules::map::shared::map_directions::MapDirections;
use crate::modules::map::shared::map_gallery_export::{MapDraftLayout, MapGalleryExport};
use crate::modules::map::shared::map_gallery_scout::{MapGalleryDraft, MapGalleryScout};
use crate::modules::map::shared::map_grid_uploads::{MapGridPreview, MapGridUpload, MapGridUploadCenter};
use crate::modules::map::shared::map_invite_link::MapInviteLink;
use crate::modules::map::stores::map_actions::map_actions::MapActionsStore;
use crate::modules::map::stores::map_actions::map_photos_types::{MapPhotoDraft, MapPhotoGrid};
use crate::modules::map::stores::map_actions::map_place_types::MapPlaceSheetSession;
use crate::modules::map::stores::map_actions::map_privacy_types::{MapPrivacyAction, MapPrivacyKind, MapPrivacyRule};
use crate::modules::map::stores::map_actions::map_tracks_types::{MapTileKey, MapTrackPeriod};
use crate::modules::map::stores::map_actions::map_types::{
    MapCoordinate, MapFocusRequest, MapFriendRequestOutcome, MapInvite, MapInviteRelation, MapInviteSheetState,
    MapInviteSheetStatePhase, MapLiveFriend, MapMergePreview, MapMergePreviewRequest, MapMergePreviewSource,
    MapMergePreviewState, MapSettingsPatch, MapUser,
};
use crate::modules::map::stores::map_services::map_services::MapServicesStore;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};
use crate::modules::media_picker::permissions::photo_library_permissions::{
    PhotoLibraryAccessLevel, PhotoLibraryPermissions,
};
use crate::modules::post::stores::post_actions::post_types::AuthorInfoMore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MapOnboardingStep {
    Intro,
    Photos,
    Ready,
}

impl MapOnboardingStep {
    pub const ALL: [MapOnboardingStep; 3] = [
        MapOnboardingStep::Intro,
        MapOnboardingStep::Photos,
        MapOnboardingStep::Ready,
    ];

    pub const fn raw_value(self) -> i64 {
        match self {
            MapOnboardingStep::Intro => 0,
            MapOnboardingStep::Photos => 1,
            MapOnboardingStep::Ready => 2,
        }
    }

    pub fn from_raw_value(raw: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|step| step.raw_value() == raw)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum MapInteractionsStoreInviteDestination {
    Map,
    Onboarding(Option<String>),
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum MapInteractionsStoreInviteSubmission {
    Success(String),
    Failure(Option<String>),
}

pub struct MapInteractionsStore {
    pub is_onboarding_presented: bool,
    pub onboarding_step: MapOnboardingStep,
    pub is_joining: bool,
    pending_invite_token: Option<String>,
    pub selected_friend_id: Option<String>,
    pub focus_request: Option<MapFocusRequest>,
    pub invite_sheet: Option<MapInviteSheetState>,
    pub panel_mode: MapPanelMode,
    pub is_setting_home: bool,
    pub is_deleting_tracks: bool,
    pub track_tail: Vec<Vec<LatLng>>,
    pub selected_grid_id: Option<String>,
    pub focused_profile_grid_id: Option<String>,
    pub grid_to_edit_id: Option<String>,
    pub place_sheet: Option<MapPlaceSheetSession>,
    pub routes_owner: Option<MapUser>,
    pub photo_picker_visible: bool,
    pub photo_caption: String,
    pub photo_upload_progress: Option<f64>,
    pub grid_uploads: Vec<MapGridUpload>,
    pub globe_entry: Option<MapGlobeEntry>,
    pub camera_request: Option<MapCameraRequest>,
    pub merge_preview: MapMergePreviewState,
    pub merge_preview_source: MapMergePreviewSource,

    invite_destination: Option<MapInteractionsStoreInviteDestination>,
    invite_generation: u64,
    onboarding_generation: u64,

    visible_screens: i64,
    photo_place: Option<LatLng>,
    draft_layout_key: Vec<u64>,
    draft_layout_memo: MapDraftLayout,
    draft_queue: Vec<(MapGalleryDraft, i64, String)>,
    upload_tasks: HashMap<String, AbortHandle>,
    cancelled_slots: HashSet<String>,
    is_draining_drafts: bool,

    media_generation: u64,
    track_tail_generation: u64,
    self_pin_wanted: bool,
    visibility_sequence: u64,
    reaction_playback_task: Option<Task<()>>,
    reaction_playback_generation: u64,
    busy_request_ids: HashSet<String>,
    lines_toggle_generation: u64,
    merge_preview_task: Option<Task<()>>,
    merge_preview_generation: u64,
    pub merge_preview_signal: Entity<MapMergePreviewSignal>,
}

pub struct MapMergePreviewSignal;

impl MapInteractionsStore {
    const LOWEST_ZOOM_AFTER_GLOBE: f64 = 1.9;
    const SELF_ZOOM_LEVEL: f64 = 15.5;
    const DRAFT_ATTEMPTS: i64 = 6;
    const DRAFT_RETRY_MS: u64 = 20_000;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        AccountStateResetRegistry::register(&cx.entity(), cx);
        Self {
            is_onboarding_presented: false,
            onboarding_step: MapOnboardingStep::Intro,
            is_joining: false,
            pending_invite_token: None,
            selected_friend_id: None,
            focus_request: None,
            invite_sheet: None,
            panel_mode: MapPanelMode::Friends,
            is_setting_home: false,
            is_deleting_tracks: false,
            track_tail: Vec::new(),
            selected_grid_id: None,
            focused_profile_grid_id: None,
            grid_to_edit_id: None,
            place_sheet: None,
            routes_owner: None,
            photo_picker_visible: false,
            photo_caption: String::new(),
            photo_upload_progress: None,
            grid_uploads: Vec::new(),
            globe_entry: None,
            camera_request: None,
            merge_preview: MapMergePreviewState::Idle,
            merge_preview_source: MapMergePreviewSource::Sample,
            invite_destination: None,
            invite_generation: 0,
            onboarding_generation: 0,
            visible_screens: 0,
            photo_place: None,
            draft_layout_key: Vec::new(),
            draft_layout_memo: MapDraftLayout::default(),
            draft_queue: Vec::new(),
            upload_tasks: HashMap::new(),
            cancelled_slots: HashSet::new(),
            is_draining_drafts: false,
            media_generation: 0,
            track_tail_generation: 0,
            self_pin_wanted: false,
            visibility_sequence: 0,
            reaction_playback_task: None,
            reaction_playback_generation: 0,
            busy_request_ids: HashSet::new(),
            lines_toggle_generation: 0,
            merge_preview_task: None,
            merge_preview_generation: 0,
            merge_preview_signal: cx.new(|_| MapMergePreviewSignal),
        }
    }

    pub fn reaction_max_graphemes() -> i64 {
        RiseLimits::service_value("map_max_reaction_graphemes")
    }

    pub fn reaction_max_bytes() -> i64 {
        RiseLimits::service_value("map_max_reaction_bytes")
    }

    pub fn photo_caption_max_characters() -> i64 {
        RiseLimits::service_value("map_max_photo_caption_chars")
    }

    fn grid_radius_meters() -> f64 {
        RiseLimits::service_value("map_photo_grid_radius_m") as f64
    }

    fn merge_preview_debounce_ms() -> u64 {
        u64::try_from(RiseTimingCanon::debounce("map_track_merge_preview").client_debounce_ms).unwrap_or(0)
    }

    fn merge_preview_window_meters() -> f64 {
        RiseLimits::service_value("map_track_merge_preview_window_m") as f64
    }

    fn actions<R>(cx: &mut App, work: impl FnOnce(&mut MapActionsStore, &mut Context<MapActionsStore>) -> R) -> R {
        MapActionsStore::shared(cx).update(cx, work)
    }

    fn owns_map_scene(cx: &mut App) -> bool {
        GuideSandbox::shared(cx).read(cx).owns_map_scene()
    }

    fn owns_map_user(user_id: &str, cx: &mut App) -> bool {
        GuideSandbox::shared(cx).read(cx).owns_map_user(Some(user_id))
    }

    fn owns_map_grid(grid_id: Option<&str>, cx: &mut App) -> bool {
        GuideSandbox::shared(cx).read(cx).owns_map_grid(grid_id)
    }

    fn self_location(cx: &mut App) -> Option<LatLng> {
        MapSelfLocationFeed::shared(cx).read(cx).location().map(|location| location.coordinate())
    }

    fn real_photos(cx: &mut App) -> RiseMapPhotosSnapshot {
        let presentation = RiseMapPresentation::shared(cx);
        presentation.read(cx).photos.read(cx).snapshot().clone()
    }

    fn is_current_account(account: &Option<String>, epoch: &Option<String>) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(account.as_deref(), epoch.as_deref())
    }

    fn is_app_active(cx: &App) -> bool {
        RiseAppLifecycle::scene_phase(cx) != ScenePhase::Background
    }

    fn is_media_generation(this: &WeakEntity<Self>, generation: u64, cx: &mut AsyncApp) -> bool {
        this.read_with(cx, |store, _| store.media_generation == generation).unwrap_or(false)
    }

    fn take_cancelled_slot(this: &WeakEntity<Self>, slot_id: &str, cx: &mut AsyncApp) -> bool {
        this.update(cx, |store, _| store.cancelled_slots.remove(slot_id)).unwrap_or(false)
    }

    fn is_slot_cancelled(this: &WeakEntity<Self>, slot_id: &str, cx: &mut AsyncApp) -> bool {
        this.read_with(cx, |store, _| store.cancelled_slots.contains(slot_id)).unwrap_or(false)
    }

    fn send_notice(title: String, content: Option<String>, kind: NotifierType, cx: &mut App) {
        let options = NotifierSendOptions {
            content,
            r#type: kind,
            position: NotifierPosition::Top,
            ..NotifierSendOptions::default()
        };
        NotifierStore::shared(cx).update(cx, |notifier, cx| {
            notifier.send(title, options, cx);
        });
    }

    fn assign_selected_friend_id(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.selected_friend_id = value;
        let is_open = self.selected_friend_id.is_some()
            && self.selected_friend_id != SaiAccountCacheScope::active_user_id();
        GuideFacts::set(RiseGuideSignal::MapFriendCardOpen, is_open, cx);
        cx.notify();
    }

    fn assign_panel_mode(&mut self, value: MapPanelMode, cx: &mut Context<Self>) {
        self.panel_mode = value;
        GuideFacts::set(RiseGuideSignal::MapStatisticsOpen, self.panel_mode == MapPanelMode::Tracks, cx);
        cx.notify();
    }

    fn assign_selected_grid_id(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.selected_grid_id = value;
        GuideFacts::set(RiseGuideSignal::MapGridOpen, self.selected_grid_id.is_some(), cx);
        cx.notify();
    }

    fn assign_photo_picker_visible(&mut self, value: bool, cx: &mut Context<Self>) {
        self.photo_picker_visible = value;
        GuideFacts::set(RiseGuideSignal::MapPhotoPickerOpen, self.photo_picker_visible, cx);
        cx.notify();
    }

    pub fn selected_friend_id(&self) -> Option<&str> {
        self.selected_friend_id.as_deref()
    }

    pub fn selected_grid_id(&self) -> Option<&str> {
        self.selected_grid_id.as_deref()
    }

    pub fn grid_to_edit_id(&self) -> Option<&str> {
        self.grid_to_edit_id.as_deref()
    }

    pub fn set_grid_to_edit_id(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.grid_to_edit_id = value;
        GuideFacts::set(RiseGuideSignal::MapGridEditorOpen, self.grid_to_edit_id.is_some(), cx);
        cx.notify();
    }

    pub fn place_sheet(&self) -> Option<&MapPlaceSheetSession> {
        self.place_sheet.as_ref()
    }

    pub fn set_place_sheet(&mut self, value: Option<MapPlaceSheetSession>, cx: &mut Context<Self>) {
        self.place_sheet = value;
        GuideFacts::set(RiseGuideSignal::MapPlaceSheetOpen, self.place_sheet.is_some(), cx);
        cx.notify();
    }

    pub fn routes_owner(&self) -> Option<&MapUser> {
        self.routes_owner.as_ref()
    }

    pub fn set_photo_picker_visible(&mut self, value: bool, cx: &mut Context<Self>) {
        self.assign_photo_picker_visible(value, cx);
    }

    pub fn set_camera_request(&mut self, value: Option<MapCameraRequest>, cx: &mut Context<Self>) {
        self.camera_request = value;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.set_place_sheet(None, cx);
        self.lines_toggle_generation = self.lines_toggle_generation.wrapping_add(1);
        self.assign_panel_mode(MapPanelMode::Friends, cx);
        self.invite_generation = self.invite_generation.wrapping_add(1);
        self.onboarding_generation = self.onboarding_generation.wrapping_add(1);
        self.invite_destination = None;
        self.invite_sheet = None;
        self.pending_invite_token = None;
        self.is_onboarding_presented = false;
        self.onboarding_step = MapOnboardingStep::Intro;
        self.is_joining = false;
        self.media_generation = self.media_generation.wrapping_add(1);
        self.track_tail_generation = self.track_tail_generation.wrapping_add(1);
        for task in self.upload_tasks.values() {
            task.abort();
        }
        self.upload_tasks = HashMap::new();
        self.draft_queue = Vec::new();
        self.cancelled_slots = HashSet::new();
        self.is_draining_drafts = false;
        self.grid_uploads = Vec::new();
        self.photo_upload_progress = None;
        self.assign_photo_picker_visible(false, cx);
        self.photo_place = None;
        self.photo_caption = String::new();
        self.assign_selected_grid_id(None, cx);
        self.focused_profile_grid_id = None;
        self.set_grid_to_edit_id(None, cx);
        self.assign_selected_friend_id(None, cx);
        self.routes_owner = None;
        self.track_tail = Vec::new();
        self.draft_layout_key = Vec::new();
        self.draft_layout_memo = MapDraftLayout::default();
        self.reaction_selection_changed(None, cx);
        MapGridUploadCenter::shared(cx).update(cx, |center, _| center.reset());
        self.merge_preview_task = None;
        self.merge_preview_generation = self.merge_preview_generation.wrapping_add(1);
        self.merge_preview = MapMergePreviewState::Idle;
        self.merge_preview_source = MapMergePreviewSource::Sample;
        cx.notify();
    }

    pub fn map_appeared(&mut self, cx: &mut Context<Self>) {
        self.visible_screens += 1;
        Self::actions(cx, |actions, cx| actions.open_action(false, cx));
        if self.visible_screens != 1 {
            return;
        }

        self.refresh_track_tail(cx);
        self.seed_own_track_cooling(cx);
        self.sync_self_pin_feed(cx);
        self.scan_gallery(cx);
        self.visibility_sequence += 1;
        let sequence = self.visibility_sequence;
        Self::actions(cx, |actions, cx| actions.set_map_visible_action(true, sequence, cx));
    }

    fn seed_own_track_cooling(&mut self, cx: &mut Context<Self>) {
        if self.routes_owner.is_some() {
            return;
        }
        let Some(settings) = MapServicesStore::state(cx).state.map(|state| state.settings) else {
            return;
        };
        Self::actions(cx, |actions, cx| {
            actions.apply_own_track_colouring_action(settings.track_cooling_days, settings.track_heat_walks, cx)
        });
    }

    pub fn enter_globe(&mut self, coordinate: LatLng, zoom_level: f64, cx: &mut Context<Self>) {
        if self.globe_entry.is_some() {
            return;
        }
        self.globe_entry = Some(MapGlobeEntry {
            id: Uuid::new_v4(),
            latitude: coordinate.latitude,
            longitude: coordinate.longitude,
            zoom_level: Some(zoom_level),
        });
        cx.notify();
    }

    pub fn globe_settled(&mut self, coordinate: LatLng, zoom_level: f64, cx: &mut Context<Self>) {
        if self.globe_entry.is_none() {
            return;
        }
        self.camera_request = Some(MapCameraRequest::new(
            Uuid::new_v4(),
            coordinate.latitude,
            coordinate.longitude,
            Self::LOWEST_ZOOM_AFTER_GLOBE.max(zoom_level),
        ));
        cx.notify();
    }

    pub fn leave_globe(&mut self, coordinate: LatLng, zoom_level: f64, cx: &mut Context<Self>) {
        if self.globe_entry.is_none() {
            return;
        }
        let landing = Self::LOWEST_ZOOM_AFTER_GLOBE.max(zoom_level);
        self.camera_request =
            Some(MapCameraRequest::new(Uuid::new_v4(), coordinate.latitude, coordinate.longitude, landing));
        self.globe_entry = None;
        cx.notify();
    }

    pub fn focus_self(&mut self, cx: &mut Context<Self>) {
        let Some(here) = Self::self_location(cx) else {
            Self::show_failure(Some(tr("map_home_location_failed")), cx);
            return;
        };
        let was_on_globe = self.globe_entry.is_some();
        self.globe_entry = None;
        self.camera_request = Some(MapCameraRequest {
            id: Uuid::new_v4(),
            latitude: here.latitude,
            longitude: here.longitude,
            zoom_level: Self::SELF_ZOOM_LEVEL,
            is_animated: !was_on_globe,
        });
        cx.notify();
    }

    pub fn focus_friend_from_globe(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if SaiAccountCacheScope::active_user_id().as_deref() == Some(user_id) {
            self.open_own_profile(cx);
            return;
        }
        self.focus_friend(user_id, cx);
    }

    pub fn own_map_profile(cx: &mut App) -> Option<MapLiveFriend> {
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile()?;
        if Some(&profile.id) != SaiAccountCacheScope::active_user_id().as_ref() {
            return None;
        }
        Some(MapLiveFriend {
            user: MapUser {
                id: profile.id.clone(),
                tag: profile.tag.clone(),
                name: profile.name.clone(),
                logo: profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()),
                is_premium: profile.more.is_premium,
                is_official: profile.more.is_official,
                name_color: profile.more.name_color.clone(),
                premium_badge_emoji: profile.more.premium_badge_emoji.clone(),
                is_deleted: false,
            },
            position: None,
        })
    }

    pub fn open_own_profile(&mut self, cx: &mut Context<Self>) {
        if !MapServicesStore::is_member(cx) {
            return;
        }
        let Some(own) = Self::own_map_profile(cx) else {
            return;
        };
        self.show_own_routes(cx);
        self.focused_profile_grid_id = None;
        self.select_grid(None, cx);
        self.focus_request = None;
        self.assign_selected_friend_id(Some(own.id().to_owned()), cx);
        Self::actions(cx, |actions, cx| {
            actions.open_tracks_action(None, false, cx);
            actions.open_photos_action(None, false, cx);
        });
    }

    pub fn self_pin_changed(&mut self, is_shown: bool, cx: &mut Context<Self>) {
        if self.self_pin_wanted == is_shown {
            return;
        }
        self.self_pin_wanted = is_shown;
        self.sync_self_pin_feed(cx);
    }

    fn sync_self_pin_feed(&mut self, cx: &mut Context<Self>) {
        let on_screen = self.self_pin_wanted && self.visible_screens > 0;
        RiseMapRecorder::shared(cx).update(cx, |recorder, cx| recorder.set_map_on_screen(on_screen, cx));
    }

    pub fn map_disappeared(&mut self, cx: &mut Context<Self>) {
        if self.visible_screens <= 0 {
            return;
        }
        self.visible_screens -= 1;
        if self.visible_screens != 0 {
            return;
        }

        self.reaction_selection_changed(None, cx);
        self.assign_selected_friend_id(None, cx);
        self.focus_request = None;
        self.globe_entry = None;
        self.sync_self_pin_feed(cx);
        self.show_own_routes(cx);
        self.visibility_sequence += 1;
        let sequence = self.visibility_sequence;
        Self::actions(cx, |actions, cx| actions.set_map_visible_action(false, sequence, cx));
        cx.notify();
    }

    pub fn retry_load(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx, |actions, cx| actions.open_action(true, cx));
    }

    pub fn set_panel_mode(&mut self, mode: MapPanelMode, cx: &mut Context<Self>) {
        if self.panel_mode == mode && self.selected_friend_id.is_none() && self.selected_grid_id.is_none() {
            return;
        }
        self.assign_panel_mode(mode, cx);
        self.assign_selected_friend_id(None, cx);
        self.select_grid(None, cx);
        if mode == MapPanelMode::Tracks {
            self.refresh_track_tail(cx);
            self.scan_gallery(cx);
        } else if self.routes_owner.is_some() {
            self.show_own_routes(cx);
        }
    }

    pub fn toggle_track_lines(&mut self, cx: &mut Context<Self>) {
        if !MapServicesStore::is_member(cx) {
            return;
        }
        if Self::owns_map_scene(cx) {
            let current = MapServicesStore::tracks(cx).shows_lines;
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.toggle_map_lines(current, cx));
            return;
        }
        self.lines_toggle_generation = self.lines_toggle_generation.wrapping_add(1);
        let generation = self.lines_toggle_generation;
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            if !Self::is_current_account(&account, &epoch) {
                return;
            }
            let toggle = cx.update(|cx| Self::actions(cx, |actions, cx| actions.toggle_track_lines_action(cx)));
            let Some(visible) = toggle.await else {
                return;
            };
            let _ = this.update(cx, |store, cx| {
                if generation != store.lines_toggle_generation || !Self::is_current_account(&account, &epoch) {
                    return;
                }
                if visible {
                    store.refresh_track_tail(cx);
                }
                let title = tr(if visible { "map_lines_shown" } else { "map_lines_hidden" });
                let options = NotifierSendOptions {
                    r#type: NotifierType::Info,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                };
                Notifier::send(title, options, cx);
                GuideSignalBus::emit(RiseGuideSignal::MapLinesToggled);
            });
        })
        .detach();
    }

    /// routes of the friend whose card just opened
    /// leaves the bottom mode switch where the reader put it
    pub fn show_selected_friend_routes(&mut self, friend: &MapLiveFriend, cx: &mut Context<Self>) {
        let mode = self.panel_mode;
        self.show_routes(friend, true, cx);
        self.assign_panel_mode(mode, cx);
    }

    pub fn show_routes(&mut self, friend: &MapLiveFriend, keep_selection: bool, cx: &mut Context<Self>) {
        self.routes_owner = Some(friend.user.clone());
        if !keep_selection {
            self.assign_selected_friend_id(None, cx);
        }
        self.assign_selected_grid_id(None, cx);
        self.focused_profile_grid_id = None;
        self.track_tail = Vec::new();
        self.assign_panel_mode(MapPanelMode::Tracks, cx);
        if Self::owns_map_user(friend.id(), cx) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.show_panda_routes(true, cx));
            return;
        }
        let owner_id = friend.id().to_owned();
        Self::actions(cx, |actions, cx| {
            actions.open_tracks_action(Some(owner_id.clone()), false, cx);
            actions.open_photos_action(Some(owner_id), false, cx);
        });
    }

    pub fn focus_profile_grid(&mut self, grid: MapPhotoGrid, owner: MapUser, edit: bool, cx: &mut Context<Self>) {
        let is_own = Some(&owner.id) == SaiAccountCacheScope::active_user_id().as_ref();
        let owner_id = owner.id.clone();
        self.routes_owner = if is_own { None } else { Some(owner) };
        self.globe_entry = None;
        self.focused_profile_grid_id = Some(grid.id.clone());
        self.assign_selected_friend_id(None, cx);
        self.assign_selected_grid_id(Some(grid.id.clone()), cx);
        self.track_tail = Vec::new();
        self.camera_request = Some(MapCameraRequest {
            id: Uuid::new_v4(),
            latitude: grid.latitude,
            longitude: grid.longitude,
            zoom_level: Self::SELF_ZOOM_LEVEL,
            is_animated: true,
        });
        cx.notify();
        let tracks_owner = if is_own { None } else { Some(owner_id) };
        Self::actions(cx, |actions, cx| actions.open_tracks_action(tracks_owner, false, cx));
        let grid_id = grid.id.clone();
        let focus = Self::actions(cx, |actions, cx| actions.focus_profile_grid_action(grid, cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let focused = focus.await;
            if !(focused && edit && is_own) {
                return;
            }
            let _ = this.update(cx, |store, cx| {
                if store.selected_grid_id.as_deref() == Some(grid_id.as_str()) {
                    store.set_grid_to_edit_id(Some(grid_id), cx);
                }
            });
        })
        .detach();
    }

    pub fn show_own_routes(&mut self, cx: &mut Context<Self>) {
        cx.defer(|cx| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.show_panda_routes(false, cx));
        });
        if self.routes_owner.is_none() && self.focused_profile_grid_id.is_none() {
            return;
        }
        self.routes_owner = None;
        self.focused_profile_grid_id = None;
        self.assign_selected_grid_id(None, cx);
        Self::actions(cx, |actions, cx| {
            actions.open_tracks_action(None, false, cx);
            actions.open_photos_action(None, false, cx);
        });
        if self.panel_mode == MapPanelMode::Tracks {
            self.refresh_track_tail(cx);
        }
    }

    pub fn select_track_period(&mut self, period: MapTrackPeriod, cx: &mut Context<Self>) {
        let changed = MapServicesStore::tracks(cx).period != period;
        Self::actions(cx, |actions, cx| actions.set_track_period_action(period, cx));
        if changed {
            GuideSignalBus::emit(RiseGuideSignal::MapTrackPeriodChanged);
        }
    }

    pub fn displayed_track_tail(cx: &mut App) -> Vec<Vec<LatLng>> {
        let practice = GuideSandbox::shared(cx).read(cx).map_demo_tail();
        let mut tail = Self::shared(cx).read(cx).track_tail.clone();
        if practice.is_empty() {
            return tail;
        }
        tail.extend(practice);
        tail
    }

    pub fn request_track_tiles(&mut self, keys: Vec<MapTileKey>, cx: &mut Context<Self>) {
        if keys.is_empty() {
            return;
        }
        Self::actions(cx, |actions, cx| actions.request_track_tiles_action(keys, cx));
    }

    fn refresh_track_tail(&mut self, cx: &mut Context<Self>) {
        self.track_tail_generation = self.track_tail_generation.wrapping_add(1);
        let generation = self.track_tail_generation;
        let epoch = SaiAccountCacheScope::active_epoch();
        let is_member = MapServicesStore::is_member(cx);
        let account_id = SaiAccountCacheScope::active_user_id().filter(|_| is_member && self.routes_owner.is_none());
        let Some(account_id) = account_id else {
            if !self.track_tail.is_empty() {
                self.track_tail = Vec::new();
                cx.notify();
            }
            return;
        };

        let journal = MapRecorderJournal::for_account(&account_id);
        let work = RiseAccountRuntime::current().spawn_blocking(move || {
            journal
                .unsent_lines()
                .into_iter()
                .map(|line| {
                    line.into_iter()
                        .map(|point| {
                            LatLng::new(f64::from(point.lat_e6) / 1_000_000.0, f64::from(point.lng_e6) / 1_000_000.0)
                        })
                        .collect::<Vec<LatLng>>()
                })
                .collect::<Vec<Vec<LatLng>>>()
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let Ok(next) = work.await else {
                return;
            };
            let account = Some(account_id);
            let _ = this.update(cx, |store, cx| {
                if generation != store.track_tail_generation
                    || store.routes_owner.is_some()
                    || store.visible_screens <= 0
                    || !Self::is_current_account(&account, &epoch)
                {
                    return;
                }
                if MapTrackLayer::tail_signature(&next) == MapTrackLayer::tail_signature(&store.track_tail) {
                    return;
                }
                store.track_tail = next;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn friends_page_appeared(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx, |actions, cx| {
            actions.open_action(false, cx);
            actions.load_invite_action(false, cx).detach();
        });
    }

    pub fn select_friend(&mut self, user_id: Option<String>, cx: &mut Context<Self>) {
        if self.selected_grid_id.is_some() {
            self.select_grid(None, cx);
        }
        if self.selected_friend_id == user_id {
            return;
        }
        match user_id {
            Some(user_id) => self.focus_friend(&user_id, cx),
            None => self.assign_selected_friend_id(None, cx),
        }
    }

    pub fn select_grid(&mut self, grid_id: Option<String>, cx: &mut Context<Self>) {
        if grid_id.is_some() && self.selected_friend_id.is_some() {
            self.assign_selected_friend_id(None, cx);
        }
        if self.selected_grid_id == grid_id {
            return;
        }
        self.assign_selected_grid_id(grid_id.clone(), cx);
        let is_draft_group = grid_id.as_deref().is_some_and(|id| id.starts_with(MapDraftLayout::PIN_PREFIX));
        let opened = match grid_id {
            Some(id) if !is_draft_group && !Self::owns_map_grid(Some(&id), cx) => Some(id),
            _ => None,
        };
        Self::actions(cx, |actions, cx| actions.open_grid_action(opened, cx));
    }

    pub fn reopen_grid(&mut self, cx: &mut Context<Self>) {
        let Some(grid_id) = self.selected_grid_id.clone() else {
            return;
        };
        if Self::owns_map_grid(Some(&grid_id), cx) {
            return;
        }
        Self::actions(cx, |actions, cx| {
            actions.open_grid_action(None, cx);
            actions.open_grid_action(Some(grid_id), cx);
        });
    }

    pub fn load_more_grid_photos(&mut self, cx: &mut Context<Self>) {
        if Self::owns_map_grid(self.selected_grid_id.as_deref(), cx) {
            return;
        }
        Self::actions(cx, |actions, cx| actions.load_more_grid_photos_action(cx));
    }

    pub fn is_own(&self, grid: &MapPhotoGrid) -> bool {
        self.routes_owner.is_none() && Some(&grid.owner_id) == SaiAccountCacheScope::active_user_id().as_ref()
    }

    pub fn present_photo_picker(&mut self, place: Option<LatLng>, cx: &mut Context<Self>) {
        if self.photo_upload_progress.is_some() || !MapServicesStore::is_member(cx) {
            return;
        }
        if place.is_some() && self.routes_owner.is_some() {
            return;
        }
        self.photo_place = place;
        self.photo_caption = String::new();
        self.assign_photo_picker_visible(true, cx);
    }

    fn grid_preview(item: &MediaPickerItem) -> MapGridPreview {
        item.asset_identifier
            .clone()
            .map_or_else(|| MapGridPreview::File(item.url.clone()), MapGridPreview::Asset)
    }

    pub fn add_photos(&mut self, items: Vec<MediaPickerItem>, cx: &mut Context<Self>) {
        if self.photo_upload_progress.is_some() || items.is_empty() {
            return;
        }
        if Self::owns_map_scene(cx) {
            let place = self.photo_place.or_else(|| Self::self_location(cx));
            let into = if self.photo_place.is_some() { self.selected_grid_id.clone() } else { None };
            self.photo_place = None;
            if into.is_none() {
                self.show_own_routes(cx);
            }
            let grid_id = GuideSandbox::shared(cx)
                .update(cx, |sandbox, cx| sandbox.add_practice_map_photos(&items, place, into.as_deref(), cx));
            if let Some(grid_id) = grid_id {
                self.select_grid(Some(grid_id), cx);
            }
            return;
        }
        let generation = self.media_generation;
        if self.photo_place.is_none() {
            self.show_own_routes(cx);
            self.select_grid(None, cx);
        }
        self.photo_upload_progress = Some(0.0);
        let picked = self.photo_place.or_else(|| Self::self_location(cx));
        let picked_into_grid = self.photo_place.is_some();
        self.photo_place = None;
        let grid_id = Uuid::new_v4().uuid_string().to_lowercase();
        let share = 1.0 / items.len() as f64;

        let center = MapGridUploadCenter::shared(cx);
        let open_grid_id = if picked_into_grid {
            let selected = self.selected_grid_id.clone();
            let grids = Self::real_photos(cx).grids;
            selected.filter(|id| grids.iter().any(|grid| &grid.id == id))
        } else {
            None
        };
        let photo_ids: Vec<String> = items.iter().map(|_| Uuid::new_v4().uuid_string().to_lowercase()).collect();
        if let Some(open_grid_id) = &open_grid_id {
            for (item, photo_id) in items.iter().zip(&photo_ids) {
                let preview = Self::grid_preview(item);
                center.update(cx, |center, cx| center.begin(photo_id, photo_id, preview, cx));
            }
            center.update(cx, |center, _| center.pin_front(&photo_ids));
            let mut uploads: Vec<MapGridUpload> = items
                .iter()
                .zip(&photo_ids)
                .map(|(item, photo_id)| MapGridUpload {
                    id: photo_id.clone(),
                    grid_id: open_grid_id.clone(),
                    preview: Self::grid_preview(item),
                    is_video: item.kind == MediaPickerItemKind::Video,
                    duration_ms: (item
                        .video_payload
                        .as_ref()
                        .map_or(0.0, |video| (video.end_seconds - video.start_seconds).max(0.0))
                        * 1_000.0) as i64,
                })
                .collect();
            uploads.append(&mut self.grid_uploads);
            self.grid_uploads = uploads;
        }
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let mut landed_grid_id: Option<String> = None;
            let mut failure: Option<String> = None;
            let mut ordered_items: Vec<(usize, MediaPickerItem)> = items.into_iter().enumerate().collect();
            if open_grid_id.is_some() {
                ordered_items.reverse();
            }
            for (step, (index, item)) in ordered_items.into_iter().enumerate() {
                if !Self::is_media_generation(&this, generation, cx) {
                    return;
                }
                let floor = step as f64 * share;
                let photo_id = photo_ids[index].clone();
                let stop = 'body: {
                    if Self::is_slot_cancelled(&this, &photo_id, cx) {
                        break 'body false;
                    }
                    let empty = MapPhotoMetadataReading { latitude: None, longitude: None, taken_at_ms: 0 };
                    let reading = if item.kind == MediaPickerItemKind::Image {
                        let file_url = item
                            .image_payload
                            .as_ref()
                            .and_then(|payload| payload.metadata_source_url.clone())
                            .unwrap_or_else(|| item.url.clone());
                        let traced_url = file_url.clone();
                        let reading = RiseAccountRuntime::current()
                            .spawn_blocking(move || MapPhotoMetadata::read(&file_url))
                            .await
                            .unwrap_or(empty);
                        tracing::info!(
                            target: "riseonly::X2064",
                            source = %traced_url,
                            has_place = reading.latitude.is_some() && reading.longitude.is_some(),
                            taken_at_ms = reading.taken_at_ms,
                            "map photo metadata read"
                        );
                        reading
                    } else {
                        empty
                    };
                    if !Self::is_media_generation(&this, generation, cx) {
                        break 'body true;
                    }
                    let latitude = picked.map(|place| place.latitude).or(reading.latitude);
                    let longitude = picked.map(|place| place.longitude).or(reading.longitude);
                    let (Some(latitude), Some(longitude)) = (latitude, longitude) else {
                        failure = Some(tr("map_photo_no_place"));
                        break 'body false;
                    };

                    let progress_owner = this.clone();
                    let on_fraction: Rc<dyn Fn(f64, &mut App)> = Rc::new(move |fraction, cx: &mut App| {
                        let _ = progress_owner.update(cx, |store, cx| {
                            if generation != store.media_generation {
                                return;
                            }
                            let total = floor + fraction * share;
                            let Some(current) = store.photo_upload_progress else {
                                return;
                            };
                            if total <= current {
                                return;
                            }
                            store.photo_upload_progress = Some(total);
                            cx.notify();
                        });
                    });
                    let upload = this.update(cx, |store, cx| {
                        store.upload(item.clone(), photo_id.clone(), photo_id.clone(), on_fraction, cx)
                    });
                    let Ok(upload) = upload else {
                        break 'body true;
                    };
                    match upload.await {
                        Ok(uploaded) => {
                            if !Self::is_media_generation(&this, generation, cx) {
                                break 'body true;
                            }
                            if Self::is_slot_cancelled(&this, &photo_id, cx) {
                                break 'body false;
                            }
                            let draft = MapPhotoDraft {
                                id: photo_id.clone(),
                                url: uploaded.file_url,
                                thumbnail_url: uploaded.thumbnail_url,
                                latitude,
                                longitude,
                                width: uploaded.width,
                                height: uploaded.height,
                                taken_at: reading.taken_at_ms,
                                caption: String::new(),
                                grid_id: grid_id.clone(),
                                media_kind: uploaded.media_kind,
                                duration_ms: uploaded.duration_ms,
                            };
                            let added = cx.update(|cx| Self::actions(cx, |actions, cx| actions.add_photo_action(draft, cx)));
                            let outcome = added.await;
                            if !Self::is_media_generation(&this, generation, cx) {
                                break 'body true;
                            }
                            match outcome {
                                RiseMapMutationOutcome::Done => {
                                    let grids = cx.update(MapServicesStore::photos).grids;
                                    let radius_meters = Self::grid_radius_meters();
                                    let distance = |grid: &MapPhotoGrid| {
                                        MapTrackGeometry::haversine_meters(
                                            latitude,
                                            longitude,
                                            grid.latitude,
                                            grid.longitude,
                                        )
                                    };
                                    landed_grid_id = grids
                                        .iter()
                                        .find(|grid| grid.id == grid_id)
                                        .map(|grid| grid.id.clone())
                                        .or_else(|| {
                                            grids
                                                .iter()
                                                .min_by(|left, right| distance(left).total_cmp(&distance(right)))
                                                .filter(|grid| distance(grid) <= radius_meters)
                                                .map(|grid| grid.id.clone())
                                        });
                                    let shown = this.update(cx, |store, cx| {
                                        store.lines_toggle_generation = store.lines_toggle_generation.wrapping_add(1);
                                        Self::actions(cx, |actions, cx| actions.show_track_lines_action(cx))
                                    });
                                    let Ok(shown) = shown else {
                                        break 'body true;
                                    };
                                    shown.await;
                                    if !Self::is_media_generation(&this, generation, cx) {
                                        break 'body true;
                                    }
                                    let _ = this.update(cx, |store, cx| store.refresh_track_tail(cx));
                                }
                                RiseMapMutationOutcome::Failed(text) => {
                                    failure = Some(text.unwrap_or_else(|| tr("map_photo_failed")));
                                }
                            }
                        }
                        Err(error) => {
                            if matches!(error.downcast_ref::<MapPhotoUploadError>(), Some(MapPhotoUploadError::VideoTooLong)) {
                                failure = Some(tr("map_photo_video_too_long"));
                            } else if !Self::is_slot_cancelled(&this, &photo_id, cx) {
                                failure = Some(tr("map_photo_failed"));
                            }
                        }
                    }
                    false
                };
                if open_grid_id.is_some() {
                    let _ = this.update(cx, |store, cx| {
                        store.grid_uploads.retain(|upload| upload.id != photo_id);
                        store.cancelled_slots.remove(&photo_id);
                        cx.notify();
                    });
                    cx.update(|cx| center.update(cx, |center, _| center.end(&photo_id)));
                }
                if stop {
                    return;
                }
            }

            if !Self::is_media_generation(&this, generation, cx) {
                return;
            }
            let grids = cx.update(MapServicesStore::photos).grids;
            let _ = this.update(cx, |store, cx| {
                store.photo_upload_progress = None;
                cx.notify();
                if let Some(failure) = failure {
                    Self::show_failure(Some(failure), cx);
                }
                if let Some(landed) = landed_grid_id.filter(|id| grids.iter().any(|grid| &grid.id == id)) {
                    store.select_grid(Some(landed), cx);
                }
            });
        })
        .detach();
    }

    pub fn scan_gallery(&mut self, cx: &mut Context<Self>) {
        if !MapServicesStore::is_member(cx) || self.routes_owner.is_some() || !MapGalleryScout::has_full_access() {
            return;
        }
        let joined_at_ms = MapServicesStore::state(cx).state.map_or(0, |state| state.joined_at);
        Self::actions(cx, |actions, cx| actions.scan_gallery_action(joined_at_ms, cx));
    }

    pub fn draft_layout(cx: &mut App) -> MapDraftLayout {
        let drafts = MapServicesStore::drafts(cx);
        let photos = MapServicesStore::photos(cx);
        Self::shared(cx).update(cx, |store, _| {
            if store.routes_owner.is_some() || photos.owner_id.is_some() || drafts.drafts.is_empty() {
                return MapDraftLayout::default();
            }
            let key = vec![drafts.revision, photos.revision];
            if key != store.draft_layout_key {
                store.draft_layout_key = key;
                let grids: Vec<MapPhotoGrid> = photos.grids.into_iter().filter(|grid| !grid.is_pending).collect();
                store.draft_layout_memo = MapDraftLayout::make(&drafts.drafts, &grids, Self::grid_radius_meters());
            }
            store.draft_layout_memo.clone()
        })
    }

    pub fn publish_drafts(&mut self, drafts: Vec<MapGalleryDraft>, cx: &mut Context<Self>) {
        if Self::owns_map_scene(cx) {
            return;
        }
        let generation = self.media_generation;
        let busy = MapServicesStore::drafts(cx).publishing;
        let fresh: Vec<MapGalleryDraft> = drafts.into_iter().filter(|draft| !busy.contains(&draft.id)).collect();
        if fresh.is_empty() {
            return;
        }
        let center = MapGridUploadCenter::shared(cx);
        for draft in fresh.iter().rev() {
            let photo_id = Uuid::new_v4().uuid_string().to_lowercase();
            center.update(cx, |center, cx| {
                center.begin(&draft.id, &photo_id, MapGridPreview::Asset(draft.id.clone()), cx)
            });
            self.cancelled_slots.remove(&draft.id);
            self.draft_queue.push((draft.clone(), 1, photo_id));
        }

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            for draft in fresh {
                if !Self::is_media_generation(&this, generation, cx) {
                    return;
                }
                let mark = cx.update(|cx| {
                    Self::actions(cx, |actions, cx| actions.mark_draft_publishing_action(draft.id.clone(), true, cx))
                });
                mark.await;
            }
            if !Self::is_media_generation(&this, generation, cx) {
                return;
            }
            let _ = this.update(cx, |store, cx| store.drain_drafts(cx));
        })
        .detach();
    }

    pub fn cancel_grid_upload(&mut self, slot_id: &str, cx: &mut Context<Self>) {
        self.cancelled_slots.insert(slot_id.to_owned());
        if let Some(task) = self.upload_tasks.get(slot_id) {
            task.abort();
        }

        if let Some(index) = self.draft_queue.iter().position(|queued| queued.0.id == slot_id) {
            let queued = self.draft_queue.remove(index);
            self.cancelled_slots.remove(slot_id);
            MapGridUploadCenter::shared(cx).update(cx, |center, _| center.forget(slot_id, &queued.2));
        }
        if MapServicesStore::drafts(cx).publishing.contains(slot_id) {
            let slot = slot_id.to_owned();
            Self::actions(cx, |actions, cx| actions.mark_draft_publishing_action(slot, false, cx)).detach();
        }
        self.grid_uploads.retain(|upload| upload.id != slot_id);
        cx.notify();
    }

    fn upload(
        &mut self,
        item: MediaPickerItem,
        slot_id: String,
        photo_id: String,
        on_fraction: Rc<dyn Fn(f64, &mut App)>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<MapPhotoUploadResult>> {
        let progress = MapGridUploadCenter::shared(cx).update(cx, |center, cx| center.box_for(&slot_id, cx));
        let size = item
            .url
            .to_file_path()
            .and_then(|path| std::fs::metadata(path).ok())
            .map_or(0, |metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX));
        progress.update(cx, |progress, cx| {
            progress.total_bytes = size;
            progress.fraction = 0.0;
            progress.is_active = true;
            cx.notify();
        });

        let (fractions, mut received) = mpsc::unbounded::<f64>();
        let on_progress: RiseMapPhotoUploadProgress = Arc::new(move |fraction| {
            let _ = fractions.unbounded_send(fraction);
        });
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            while let Some(fraction) = received.next().await {
                cx.update(|cx| {
                    progress.update(cx, |progress, cx| {
                        if fraction - progress.fraction >= 0.01 || fraction >= 1.0 {
                            progress.fraction = fraction;
                            cx.notify();
                        }
                    });
                    on_fraction(fraction, cx);
                });
            }
        })
        .detach();

        let (abort, registration) = AbortHandle::new_pair();
        self.upload_tasks.insert(slot_id.clone(), abort);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result =
                Abortable::new(RiseMapPhotoUpload::upload(&item, &photo_id, on_progress, cx), registration).await;
            let _ = this.update(cx, |store, _| {
                store.upload_tasks.remove(&slot_id);
            });
            match result {
                Ok(uploaded) => uploaded,
                Err(_) => Err(CancellationError.into()),
            }
        })
    }

    pub fn discard_drafts(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let ids: HashSet<String> = ids.into_iter().collect();
        Self::actions(cx, |actions, cx| actions.remove_drafts_action(ids, cx)).detach();
    }

    pub fn discard_all_drafts(&mut self, cx: &mut Context<Self>) {
        if self.selected_grid_id.as_deref().is_some_and(|id| id.starts_with(MapDraftLayout::PIN_PREFIX)) {
            self.assign_selected_grid_id(None, cx);
        }
        Self::actions(cx, |actions, cx| actions.remove_all_drafts_action(cx));
    }

    fn drain_drafts(&mut self, cx: &mut Context<Self>) {
        if self.is_draining_drafts {
            return;
        }
        self.is_draining_drafts = true;
        let generation = self.media_generation;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                let next = this
                    .update(cx, |store, _| {
                        (store.media_generation == generation && !store.draft_queue.is_empty())
                            .then(|| store.draft_queue.remove(0))
                    })
                    .ok()
                    .flatten();
                let Some((draft, attempt, photo_id)) = next else {
                    break;
                };
                Self::publish(this.clone(), draft, attempt, photo_id, cx).await;
            }
            let _ = this.update(cx, |store, _| {
                if store.media_generation == generation {
                    store.is_draining_drafts = false;
                }
            });
        })
        .detach();
    }

    async fn publish(this: WeakEntity<Self>, draft: MapGalleryDraft, attempt: i64, photo_id: String, cx: &mut AsyncApp) {
        let Ok(generation) = this.read_with(cx, |store, _| store.media_generation) else {
            return;
        };
        let center = cx.update(MapGridUploadCenter::shared);
        if Self::take_cancelled_slot(&this, &draft.id, cx) {
            return;
        }
        let Some(item) = MapGalleryExport::item(&draft).await else {
            if !Self::is_media_generation(&this, generation, cx) {
                return;
            }
            cx.update(|cx| center.update(cx, |center, _| center.forget(&draft.id, &photo_id)));
            let mark = cx.update(|cx| {
                Self::actions(cx, |actions, cx| actions.mark_draft_publishing_action(draft.id.clone(), false, cx))
            });
            mark.await;
            cx.update(|cx| Self::show_failure(Some(tr("map_photo_failed")), cx));
            return;
        };

        'body: {
            if !Self::is_media_generation(&this, generation, cx) {
                break 'body;
            }

            let mut failure: Option<String> = None;
            let upload = this.update(cx, |store, cx| {
                store.upload(item.clone(), draft.id.clone(), photo_id.clone(), Rc::new(|_, _| {}), cx)
            });
            let Ok(upload) = upload else {
                break 'body;
            };
            match upload.await {
                Ok(uploaded) => {
                    if !Self::is_media_generation(&this, generation, cx) {
                        break 'body;
                    }
                    if Self::take_cancelled_slot(&this, &draft.id, cx) {
                        break 'body;
                    }
                    let photo = MapPhotoDraft {
                        id: photo_id.clone(),
                        url: uploaded.file_url,
                        thumbnail_url: uploaded.thumbnail_url,
                        latitude: draft.latitude,
                        longitude: draft.longitude,
                        width: uploaded.width,
                        height: uploaded.height,
                        taken_at: draft.taken_at,
                        caption: String::new(),
                        grid_id: Uuid::new_v4().uuid_string().to_lowercase(),
                        media_kind: draft.media_kind,
                        duration_ms: draft.duration_ms,
                    };
                    let added = cx.update(|cx| Self::actions(cx, |actions, cx| actions.add_photo_action(photo, cx)));
                    let outcome = added.await;
                    if !Self::is_media_generation(&this, generation, cx) {
                        break 'body;
                    }
                    if let RiseMapMutationOutcome::Failed(text) = outcome {
                        failure = Some(text.unwrap_or_else(|| tr("map_photo_failed")));
                    }
                }
                Err(_) => {
                    if !Self::is_media_generation(&this, generation, cx) {
                        break 'body;
                    }
                    if Self::take_cancelled_slot(&this, &draft.id, cx) {
                        break 'body;
                    }
                    if attempt < Self::DRAFT_ATTEMPTS {
                        cx.update(|cx| {
                            let progress = center.update(cx, |center, cx| center.box_for(&draft.id, cx));
                            progress.update(cx, |progress, cx| {
                                progress.is_active = false;
                                cx.notify();
                            });
                        });
                        cx.background_executor().timer(Duration::from_millis(Self::DRAFT_RETRY_MS)).await;
                        if !Self::is_media_generation(&this, generation, cx) {
                            break 'body;
                        }
                        if Self::take_cancelled_slot(&this, &draft.id, cx) {
                            break 'body;
                        }
                        let _ = this.update(cx, |store, _| {
                            store.draft_queue.push((draft.clone(), attempt + 1, photo_id.clone()));
                        });
                        break 'body;
                    }
                    failure = Some(tr("map_photo_failed"));
                }
            }

            if let Some(failure) = failure {
                cx.update(|cx| center.update(cx, |center, _| center.forget(&draft.id, &photo_id)));
                let mark = cx.update(|cx| {
                    Self::actions(cx, |actions, cx| actions.mark_draft_publishing_action(draft.id.clone(), false, cx))
                });
                mark.await;
                cx.update(|cx| Self::show_failure(Some(failure), cx));
            } else {
                cx.update(|cx| center.update(cx, |center, _| center.end(&draft.id)));
                let ids: HashSet<String> = HashSet::from([draft.id.clone()]);
                let removed = cx.update(|cx| Self::actions(cx, |actions, cx| actions.remove_drafts_action(ids, cx)));
                removed.await;
            }
        }
        MapGalleryExport::discard(&item);
    }

    pub fn delete_photo(&mut self, photo_id: &str, cx: &mut Context<Self>) {
        let deleted = GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_practice_map_photo(photo_id, cx));
        if deleted {
            if let Some(grid_id) = self.selected_grid_id.clone()
                && !Self::owns_map_grid(Some(&grid_id), cx)
            {
                self.assign_selected_grid_id(None, cx);
            }
            return;
        }
        let id = photo_id.to_owned();
        let task = Self::actions(cx, |actions, cx| actions.delete_photo_action(id, cx));
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            if let RiseMapMutationOutcome::Failed(text) = task.await {
                cx.update(|cx| Self::show_failure(text, cx));
            }
        })
        .detach();
    }

    pub fn reset_map_bearing(&mut self, cx: &mut Context<Self>) {
        cx.defer(|cx| {
            MapGlobeCameraLink::shared(cx).update(cx, |link, cx| link.reset_north(cx));
        });
    }

    pub fn update_grid(&mut self, body: UpdatePhotoGrid, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let practice = GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.update_practice_grid(&body, cx));
        if practice {
            return Task::ready(RiseMapMutationOutcome::Done);
        }
        Self::actions(cx, |actions, cx| actions.update_grid_action(body, cx))
    }

    pub fn delete_grid(&mut self, grid_id: &str, cx: &mut Context<Self>) {
        if self.selected_grid_id.as_deref() == Some(grid_id) {
            self.assign_selected_grid_id(None, cx);
        }
        if GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_practice_grid(grid_id, cx)) {
            return;
        }
        let id = grid_id.to_owned();
        let task = Self::actions(cx, |actions, cx| actions.delete_grid_action(id, cx));
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            if let RiseMapMutationOutcome::Failed(text) = task.await {
                cx.update(|cx| Self::show_failure(text, cx));
            }
        })
        .detach();
    }

    pub fn focus_friend(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if SaiAccountCacheScope::active_user_id().as_deref() == Some(user_id) {
            self.open_own_profile(cx);
            return;
        }
        let was_on_globe = self.globe_entry.is_some();
        self.globe_entry = None;
        self.assign_selected_friend_id(Some(user_id.to_owned()), cx);

        let fix = MapServicesStore::live(cx)
            .friends
            .into_iter()
            .find(|friend| friend.id() == user_id)
            .and_then(|friend| friend.position)
            .map(|position| position.fix);
        let Some(fix) = fix else {
            self.focus_request = Some(MapFocusRequest { id: Uuid::new_v4(), user_id: user_id.to_owned() });
            cx.notify();
            return;
        };
        self.focus_request = None;
        self.camera_request = Some(MapCameraRequest {
            id: Uuid::new_v4(),
            latitude: fix.latitude,
            longitude: fix.longitude,
            zoom_level: Self::SELF_ZOOM_LEVEL,
            is_animated: !was_on_globe,
        });
        cx.notify();
    }

    pub fn focus_handled(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.focus_request.as_ref().map(|request| request.id) != Some(id) {
            return;
        }
        self.focus_request = None;
        cx.notify();
    }

    pub fn open_route(&mut self, friend: &MapLiveFriend, cx: &mut Context<Self>) {
        if Self::owns_map_user(friend.id(), cx) {
            return;
        }
        let Some(position) = friend.position.as_ref() else {
            return;
        };
        MapDirections::open(position.fix.coordinate(), friend.user.display_name(), cx);
    }

    pub fn present_invite(&mut self, token: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(token).to_owned();
        if trimmed.is_empty() {
            return;
        }
        if self.is_joining || self.invite_sheet.as_ref().is_some_and(|sheet| sheet.is_submitting) {
            return;
        }
        self.dismiss_onboarding(cx);
        self.invite_generation = self.invite_generation.wrapping_add(1);
        self.invite_destination = None;
        self.invite_sheet = Some(MapInviteSheetState::new(trimmed.clone(), MapInviteSheetStatePhase::Loading));
        cx.notify();
        self.load_invite_preview(trimmed, cx);
    }

    pub fn retry_invite_preview(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.invite_sheet.as_ref().map(|sheet| sheet.token.clone()) else {
            return;
        };
        self.invite_generation = self.invite_generation.wrapping_add(1);
        if let Some(sheet) = self.invite_sheet.as_mut() {
            sheet.phase = MapInviteSheetStatePhase::Loading;
        }
        cx.notify();
        self.load_invite_preview(token, cx);
    }

    pub fn dismiss_invite_sheet(&mut self, cx: &mut Context<Self>) {
        if self.invite_sheet.as_ref().is_some_and(|sheet| sheet.is_submitting) {
            return;
        }
        self.invite_generation = self.invite_generation.wrapping_add(1);
        self.invite_sheet = None;
        cx.notify();
    }

    pub fn invite_sheet_did_dismiss(&mut self, open_map: impl FnOnce(&mut App), cx: &mut Context<Self>) {
        if self.invite_sheet.is_some() {
            return;
        }
        let Some(destination) = self.invite_destination.take() else {
            return;
        };
        open_map(cx);
        if let MapInteractionsStoreInviteDestination::Onboarding(token) = destination {
            self.pending_invite_token = token;
            self.present_onboarding(cx);
        }
    }

    pub fn submit_invite(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.invite_sheet.clone() else {
            return;
        };
        if sheet.is_submitting {
            return;
        }
        let MapInviteSheetStatePhase::Preview(preview) = sheet.phase else {
            return;
        };

        if !preview.viewer_is_member && preview.relation != MapInviteRelation::Requested {
            let token = if preview.relation == MapInviteRelation::Own { None } else { Some(sheet.token.clone()) };
            self.invite_destination = Some(MapInteractionsStoreInviteDestination::Onboarding(token));
            self.invite_sheet = None;
            cx.notify();
            return;
        }

        match preview.relation {
            MapInviteRelation::Own | MapInviteRelation::Friends => {
                self.invite_destination = Some(MapInteractionsStoreInviteDestination::Map);
                self.invite_sheet = None;
                cx.notify();
            }
            MapInviteRelation::None => {
                let token = sheet.token.clone();
                self.run_invite_submission(
                    sheet.token,
                    move |cx| {
                        let task = Self::actions(cx, |actions, cx| actions.use_invite_action(token, cx));
                        cx.spawn(async move |_: &mut AsyncApp| match task.await {
                            RiseMapFriendRequestResult::Sent(outcome) => {
                                let key = if outcome.friend.is_none() { "map_request_sent" } else { "map_friend_added" };
                                MapInteractionsStoreInviteSubmission::Success(key.to_owned())
                            }
                            RiseMapFriendRequestResult::Failed(text) => MapInteractionsStoreInviteSubmission::Failure(text),
                        })
                    },
                    cx,
                );
            }
            MapInviteRelation::Incoming => {
                let Some(request_id) = preview.request_id else {
                    return;
                };
                self.run_invite_submission(
                    sheet.token,
                    move |cx| {
                        let task = Self::actions(cx, |actions, cx| {
                            actions.respond_friend_request_action(request_id, true, cx)
                        });
                        cx.spawn(async move |_: &mut AsyncApp| match task.await {
                            RiseMapMutationOutcome::Done => {
                                MapInteractionsStoreInviteSubmission::Success("map_friend_added".to_owned())
                            }
                            RiseMapMutationOutcome::Failed(text) => MapInteractionsStoreInviteSubmission::Failure(text),
                        })
                    },
                    cx,
                );
            }
            MapInviteRelation::Requested => {
                let Some(request_id) = preview.request_id else {
                    return;
                };
                self.run_invite_submission(
                    sheet.token,
                    move |cx| {
                        let task =
                            Self::actions(cx, |actions, cx| actions.cancel_friend_request_action(request_id, cx));
                        cx.spawn(async move |_: &mut AsyncApp| match task.await {
                            RiseMapMutationOutcome::Done => {
                                MapInteractionsStoreInviteSubmission::Success("map_request_cancelled".to_owned())
                            }
                            RiseMapMutationOutcome::Failed(text) => MapInteractionsStoreInviteSubmission::Failure(text),
                        })
                    },
                    cx,
                );
            }
        }
    }

    pub fn present_onboarding(&mut self, cx: &mut Context<Self>) {
        if self.is_onboarding_presented || self.is_joining {
            return;
        }
        self.onboarding_generation = self.onboarding_generation.wrapping_add(1);
        self.onboarding_step = MapOnboardingStep::Intro;
        self.is_onboarding_presented = true;
        cx.notify();
    }

    pub fn dismiss_onboarding(&mut self, cx: &mut Context<Self>) {
        if !self.is_onboarding_presented || self.is_joining {
            return;
        }
        self.onboarding_generation = self.onboarding_generation.wrapping_add(1);
        self.is_onboarding_presented = false;
        self.pending_invite_token = None;
        cx.notify();
    }

    pub fn advance_onboarding(&mut self, cx: &mut Context<Self>) {
        match self.onboarding_step {
            MapOnboardingStep::Intro => self.onboarding_step = Self::step_after_intro(),
            MapOnboardingStep::Photos => self.onboarding_step = MapOnboardingStep::Ready,
            MapOnboardingStep::Ready => {
                self.finish_onboarding(cx);
                return;
            }
        }
        cx.notify();
    }

    fn step_after_intro() -> MapOnboardingStep {
        if MapGalleryScout::has_full_access() { MapOnboardingStep::Ready } else { MapOnboardingStep::Photos }
    }

    pub fn request_gallery_access(&mut self, cx: &mut Context<Self>) {
        let generation = self.onboarding_generation;
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            if PhotoLibraryPermissions::current() == PhotoLibraryAccessLevel::NotDetermined {
                let _ = PhotoLibraryPermissions::request().await;
            } else if !MapGalleryScout::has_full_access() {
                cx.update(PhotoLibraryPermissions::open_system_settings);
            }
            let _ = this.update(cx, |store, cx| {
                if !store.is_onboarding_presented
                    || store.onboarding_generation != generation
                    || !Self::is_current_account(&account, &epoch)
                {
                    return;
                }
                store.onboarding_step = MapOnboardingStep::Ready;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        if !self.is_onboarding_presented || self.onboarding_step != MapOnboardingStep::Ready || self.is_joining {
            return;
        }
        if MapServicesStore::is_member(cx) {
            self.is_onboarding_presented = false;
            cx.notify();
            self.resume_pending_invite(cx);
            return;
        }

        self.is_joining = true;
        cx.notify();
        let generation = self.onboarding_generation;
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            if !Self::is_current_account(&account, &epoch) {
                return;
            }
            let join = cx.update(|cx| Self::actions(cx, |actions, cx| actions.join_action(cx)));
            let outcome = join.await;
            let _ = this.update(cx, |store, cx| {
                if store.onboarding_generation != generation || !Self::is_current_account(&account, &epoch) {
                    return;
                }
                store.is_joining = false;
                cx.notify();

                match outcome {
                    RiseMapMutationOutcome::Done => {
                        store.is_onboarding_presented = false;
                        store.resume_pending_invite(cx);
                        store.scan_gallery(cx);
                    }
                    RiseMapMutationOutcome::Failed(text) => Self::show_failure(text, cx),
                }
            });
        })
        .detach();
    }

    pub fn update_settings(&mut self, patch: MapSettingsPatch, cx: &mut Context<Self>) {
        if !MapServicesStore::is_member(cx) {
            return;
        }
        if Self::owns_map_scene(cx) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.apply_map_settings(&patch, cx));
            return;
        }
        if patch.track_cooling_days.is_some() || patch.track_heat_walks.is_some() {
            let cooling_days = patch.track_cooling_days.unwrap_or(0);
            let heat_walks = patch.track_heat_walks.unwrap_or(0);
            Self::actions(cx, |actions, cx| actions.apply_own_track_colouring_action(cooling_days, heat_walks, cx));
        }

        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            if !Self::is_current_account(&account, &epoch) {
                return;
            }
            let update = cx.update(|cx| Self::actions(cx, |actions, cx| actions.update_settings_action(patch, cx)));
            let outcome = update.await;
            if !Self::is_current_account(&account, &epoch) {
                return;
            }
            match outcome {
                RiseMapMutationOutcome::Done => GuideSignalBus::emit(RiseGuideSignal::MapSettingsChanged),
                RiseMapMutationOutcome::Failed(text) => cx.update(|cx| Self::show_failure(text, cx)),
            }
        })
        .detach();
    }

    pub fn toggle_precise_recording(&mut self, cx: &mut Context<Self>) {
        if !MapServicesStore::is_member(cx) {
            return;
        }
        let Some(settings) = MapServicesStore::state(cx).state.map(|state| state.settings) else {
            return;
        };
        let next = !settings.track_precise_recording;
        self.update_settings(MapSettingsPatch { track_precise_recording: Some(next), ..MapSettingsPatch::default() }, cx);
        let title = tr(if next { "map_precise_recording_on" } else { "map_precise_recording_off" });
        Self::send_notice(title, Some(tr("map_precise_recording_hint")), NotifierType::Info, cx);
    }

    pub fn preview_merge(
        &mut self,
        radius_m: i64,
        lanes: i64,
        source: Option<MapMergePreviewSource>,
        debounced: bool,
        cx: &mut Context<Self>,
    ) {
        if !MapServicesStore::is_member(cx) {
            return;
        }
        if let Some(source) = source {
            self.merge_preview_source = source;
        }
        self.merge_preview_task = None;
        self.merge_preview_generation = self.merge_preview_generation.wrapping_add(1);
        let generation = self.merge_preview_generation;
        let chosen = self.merge_preview_source;
        let centre = if chosen == MapMergePreviewSource::Mine { Self::merge_preview_centre(cx) } else { None };
        if chosen != MapMergePreviewSource::Sample && centre.is_none() {
            self.merge_preview = MapMergePreviewState::Loaded(MapMergePreview {
                walks: Vec::new(),
                lines: Vec::new(),
                radius_m,
                lanes,
                window_meters: Self::merge_preview_window_meters(),
                truncated: false,
                walk_count: 0,
                line_count: 0,
            });
            self.notify_merge_preview(cx);
            return;
        }
        self.merge_preview = MapMergePreviewState::Loading(self.merge_preview.preview().cloned());
        self.notify_merge_preview(cx);
        let request = MapMergePreviewRequest { source: chosen, radius_m, lanes, centre };
        let delay = if debounced { Self::merge_preview_debounce_ms() } else { 0 };
        self.merge_preview_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            if delay > 0 {
                cx.background_executor().timer(Duration::from_millis(delay)).await;
            }
            let Ok(preview) = this.update(cx, |_, cx| {
                Self::actions(cx, |actions, cx| actions.preview_track_merge_action(request, cx))
            }) else {
                return;
            };
            let result = preview.await;
            let _ = this.update(cx, |store, cx| {
                if generation != store.merge_preview_generation {
                    return;
                }
                store.merge_preview = match result {
                    RiseMapMergePreviewResult::Loaded(preview) => MapMergePreviewState::Loaded(preview),
                    RiseMapMergePreviewResult::Failed(text) => MapMergePreviewState::Failed(text),
                };
                store.notify_merge_preview(cx);
            });
        }));
    }

    pub fn merge_preview_disappeared(&mut self, cx: &mut Context<Self>) {
        self.merge_preview_task = None;
        self.merge_preview_generation = self.merge_preview_generation.wrapping_add(1);
        self.merge_preview = MapMergePreviewState::Idle;
        self.notify_merge_preview(cx);
    }

    fn notify_merge_preview(&self, cx: &mut Context<Self>) {
        self.merge_preview_signal.update(cx, |_, cx| cx.notify());
    }

    fn merge_preview_centre(cx: &mut App) -> Option<MapCoordinate> {
        if let Some(centre) = RiseMapCamera::saved_centre() {
            return Some(MapCoordinate { latitude: centre.latitude, longitude: centre.longitude });
        }
        Self::self_location(cx).map(|location| MapCoordinate { latitude: location.latitude, longitude: location.longitude })
    }

    pub fn leave_map(&mut self, on_left: impl FnOnce(&mut App) + 'static, cx: &mut Context<Self>) {
        let leave = Self::actions(cx, |actions, cx| actions.leave_action(cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = leave.await;
            let _ = this.update(cx, |store, cx| match outcome {
                RiseMapMutationOutcome::Done => {
                    store.routes_owner = None;
                    store.assign_panel_mode(MapPanelMode::Friends, cx);
                    on_left(cx);
                }
                RiseMapMutationOutcome::Failed(text) => Self::show_failure(text, cx),
            });
        })
        .detach();
    }

    pub fn settings_appeared(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx, |actions, cx| {
            actions.open_action(false, cx);
            actions.open_privacy_action(false, cx);
            actions.open_tracks_action(None, false, cx);
        });
    }

    pub fn set_privacy_rule(&mut self, rule: MapPrivacyRule, kind: MapPrivacyKind, cx: &mut Context<Self>) {
        let task = Self::actions(cx, |actions, cx| actions.set_privacy_rule_action(rule, kind, cx));
        Self::show_failure_of(task, cx);
    }

    pub fn save_privacy_users(
        &mut self,
        users: Vec<MapUser>,
        kind: MapPrivacyKind,
        action: MapPrivacyAction,
        cx: &mut Context<Self>,
    ) {
        let task = Self::actions(cx, |actions, cx| actions.set_privacy_users_action(users, kind, action, cx));
        Self::show_failure_of(task, cx);
    }

    fn show_failure_of(task: Task<RiseMapMutationOutcome>, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            if let RiseMapMutationOutcome::Failed(text) = task.await {
                cx.update(|cx| Self::show_failure(text, cx));
            }
        })
        .detach();
    }

    pub fn set_home_here(&mut self, cx: &mut Context<Self>) {
        if self.is_setting_home {
            return;
        }
        if Self::owns_map_scene(cx) {
            self.set_practice_home_here(cx);
            return;
        }
        let Some(location) = Self::self_location(cx) else {
            Self::show_failure(Some(tr("map_location_from_phone")), cx);
            return;
        };
        self.is_setting_home = true;
        cx.notify();

        let set_home =
            Self::actions(cx, |actions, cx| actions.set_home_action(location.latitude, location.longitude, cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = set_home.await;
            let _ = this.update(cx, |store, cx| {
                store.is_setting_home = false;
                cx.notify();
                match outcome {
                    RiseMapMutationOutcome::Done => {
                        Self::send_notice(tr("map_home_saved"), None, NotifierType::Success, cx);
                    }
                    RiseMapMutationOutcome::Failed(text) => Self::show_failure(text, cx),
                }
            });
        })
        .detach();
    }

    fn set_practice_home_here(&mut self, cx: &mut Context<Self>) {
        let here = Self::self_location(cx).unwrap_or_else(|| GuideSandbox::shared(cx).read(cx).map_panda_coordinate());
        GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_map_home(Some(here), cx));
        Self::send_notice(tr("map_home_saved"), None, NotifierType::Success, cx);
    }

    pub fn remove_home(&mut self, cx: &mut Context<Self>) {
        if Self::owns_map_scene(cx) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_map_home(None, cx));
            return;
        }
        let task = Self::actions(cx, |actions, cx| actions.delete_home_action(cx));
        Self::show_failure_of(task, cx);
    }

    pub fn delete_all_tracks(&mut self, cx: &mut Context<Self>) {
        if self.is_deleting_tracks {
            return;
        }
        self.is_deleting_tracks = true;
        cx.notify();

        let task = Self::actions(cx, |actions, cx| actions.delete_all_tracks_action(cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = task.await;
            let _ = this.update(cx, |store, cx| {
                store.is_deleting_tracks = false;
                store.track_tail = Vec::new();
                cx.notify();

                match outcome {
                    RiseMapMutationOutcome::Done => {
                        Self::send_notice(tr("map_tracks_deleted"), None, NotifierType::Success, cx);
                    }
                    RiseMapMutationOutcome::Failed(text) => {
                        Self::show_failure(Some(text.unwrap_or_else(|| tr("map_tracks_delete_later"))), cx);
                    }
                }
            });
        })
        .detach();
    }

    pub fn load_more(&mut self, list: RiseMapFriendsRepositoryList, cx: &mut Context<Self>) {
        Self::actions(cx, |actions, cx| actions.load_more_action(list, cx));
    }

    pub fn resume_friends(&mut self, lists: Vec<RiseMapFriendsRepositoryList>, cx: &mut Context<Self>) {
        Self::actions(cx, |actions, cx| actions.resume_friends_action(lists, cx));
    }

    pub fn send_friend_request(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() || self.busy_request_ids.contains(user_id) {
            return;
        }
        if Self::owns_map_user(user_id, cx) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.request_map_friendship(cx));
            Self::send_notice(tr("map_request_sent"), None, NotifierType::Success, cx);
            return;
        }
        self.busy_request_ids.insert(user_id.to_owned());

        let target = user_id.to_owned();
        let task = Self::actions(cx, |actions, cx| actions.send_friend_request_action(target.clone(), cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = this.update(cx, |store, _| {
                store.busy_request_ids.remove(&target);
            });

            match result {
                RiseMapFriendRequestResult::Sent(outcome) => {
                    GuideSignalBus::emit(RiseGuideSignal::MapFriendRequestSent);
                    if outcome.friend.is_some() {
                        GuideSignalBus::emit(RiseGuideSignal::MapFriendAdded);
                    }
                    Self::announce(outcome, cx).await;
                }
                RiseMapFriendRequestResult::Failed(text) => cx.update(|cx| Self::show_failure(text, cx)),
            }
        })
        .detach();
    }

    pub fn pick_person_to_add(&mut self, cx: &mut Context<Self>) {
        let friends = MapServicesStore::friends(cx);
        let mut excluded: HashSet<String> = friends.friends.iter().map(|friend| friend.id().to_owned()).collect();
        excluded.extend(friends.outgoing.iter().map(|request| request.to_user.id.clone()));
        if let Some(own_id) = SaiAccountCacheScope::active_user_id() {
            excluded.insert(own_id);
        }

        let this = cx.entity().downgrade();
        PeoplePickerStore::shared(cx).update(cx, |picker, cx| {
            picker.present(
                PeoplePickerMode::SearchAll,
                1,
                Vec::new(),
                excluded,
                tr("map_add_friend_title"),
                Some(tr("map_add_friend_hint")),
                PeoplePickerHostContext::Root,
                move |picked, cx: &mut App| {
                    let Some(user_id) = picked.first().map(|user| user.id.clone()) else {
                        return;
                    };
                    let _ = this.update(cx, |store, cx| store.send_friend_request(&user_id, cx));
                },
                cx,
            )
        });
    }

    pub fn respond(&mut self, request_id: &str, accept: bool, cx: &mut Context<Self>) {
        if self.busy_request_ids.contains(request_id) {
            return;
        }
        self.busy_request_ids.insert(request_id.to_owned());

        let id = request_id.to_owned();
        let task = Self::actions(cx, |actions, cx| actions.respond_friend_request_action(id.clone(), accept, cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = task.await;
            let _ = this.update(cx, |store, _| {
                store.busy_request_ids.remove(&id);
            });
            match outcome {
                RiseMapMutationOutcome::Done => {
                    if accept {
                        GuideSignalBus::emit(RiseGuideSignal::MapFriendAdded);
                    }
                }
                RiseMapMutationOutcome::Failed(text) => cx.update(|cx| Self::show_failure(text, cx)),
            }
        })
        .detach();
    }

    pub fn cancel_request(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if self.busy_request_ids.contains(request_id) {
            return;
        }
        if Self::owns_map_scene(cx) && request_id == GuideSandbox::map_request_id() {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.cancel_map_friendship(cx));
            Self::send_notice(tr("map_request_cancelled"), None, NotifierType::Info, cx);
            return;
        }
        self.busy_request_ids.insert(request_id.to_owned());

        let id = request_id.to_owned();
        let task = Self::actions(cx, |actions, cx| actions.cancel_friend_request_action(id.clone(), cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let outcome = task.await;
            let _ = this.update(cx, |store, _| {
                store.busy_request_ids.remove(&id);
            });
            if let RiseMapMutationOutcome::Failed(text) = outcome {
                cx.update(|cx| Self::show_failure(text, cx));
            }
        })
        .detach();
    }

    pub fn remove_friend(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if Self::owns_map_user(user_id, cx) {
            return;
        }
        let id = user_id.to_owned();
        let task = Self::actions(cx, |actions, cx| actions.remove_friend_action(id, cx));
        Self::show_failure_of(task, cx);
    }

    pub fn remove_profile_friend(&mut self, user_id: String, cx: &mut Context<Self>) -> Task<bool> {
        if Self::owns_map_user(&user_id, cx) {
            return Task::ready(false);
        }
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let task = Self::actions(cx, |actions, cx| actions.remove_friend_action(user_id, cx));
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = task.await;
            if !Self::is_current_account(&account, &epoch) {
                return false;
            }
            if let RiseMapMutationOutcome::Failed(text) = result {
                cx.update(|cx| Self::show_failure(text, cx));
                return false;
            }
            true
        })
    }

    pub fn delete_profile_grid(&mut self, grid: MapPhotoGrid, cx: &mut Context<Self>) -> Task<bool> {
        if Some(&grid.owner_id) != SaiAccountCacheScope::active_user_id().as_ref() {
            return Task::ready(false);
        }
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let grid_id = grid.id.clone();
        let task = Self::actions(cx, |actions, cx| actions.delete_grid_action(grid.id, cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = task.await;
            if !Self::is_current_account(&account, &epoch) {
                return false;
            }
            if let RiseMapMutationOutcome::Failed(text) = result {
                cx.update(|cx| Self::show_failure(text, cx));
                return false;
            }
            let _ = this.update(cx, |store, cx| {
                if store.selected_grid_id.as_deref() == Some(grid_id.as_str()) {
                    store.select_grid(None, cx);
                }
            });
            true
        })
    }

    pub fn edit_profile_grid(&mut self, grid: MapPhotoGrid, owner: MapUser, cx: &mut Context<Self>) {
        if Some(&grid.owner_id) != SaiAccountCacheScope::active_user_id().as_ref() {
            return;
        }
        self.focus_profile_grid(grid, owner, true, cx);
    }

    pub fn load_invite(&mut self, cx: &mut Context<Self>) -> Task<Option<MapInvite>> {
        Self::actions(cx, |actions, cx| actions.load_invite_action(false, cx))
    }

    pub fn reset_invite(&mut self, cx: &mut Context<Self>) {
        let task = Self::actions(cx, |actions, cx| actions.reset_invite_action(cx));
        Self::show_failure_of(task, cx);
    }

    pub fn share_invite(&mut self, invite: &MapInvite, cx: &mut Context<Self>) {
        let Some(url) = MapInviteLink::url(&invite.token) else {
            return;
        };

        ShareSheetStore::shared(cx).update(cx, |share, cx| share.present_link(url, cx));
    }

    pub fn copy_invite(&mut self, invite: &MapInvite, cx: &mut Context<Self>) {
        let Some(url) = MapInviteLink::url(&invite.token) else {
            return;
        };

        UIPasteboard::general().set_string(url.absolute_string(), cx);
        Self::send_notice(tr("copied"), None, NotifierType::Success, cx);
    }

    pub fn open_chat(&mut self, user: &MapUser, open: impl FnOnce(String, &mut App), cx: &mut Context<Self>) {
        if user.id.is_empty() || user.is_deleted || Self::owns_map_user(&user.id, cx) {
            return;
        }
        let target = User {
            id: user.id.clone(),
            name: user.name.clone(),
            tag: user.tag.clone(),
            more: AuthorInfoMore {
                logo: user.logo.clone(),
                is_premium: user.is_premium,
                is_official: user.is_official,
                premium_badge_emoji: user.premium_badge_emoji.clone(),
                name_color: user.name_color.clone(),
                ..AuthorInfoMore::default()
            },
            ..User::default()
        };
        let chat_id = ChatInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.direct_conversation_id(&target, cx));
        if chat_id.is_empty() {
            return;
        }

        open(chat_id, cx);
    }

    pub fn load_profile(
        &mut self,
        owner_id: String,
        tab: Option<MapProfileTab>,
        more: bool,
        cx: &mut Context<Self>,
    ) -> Task<Option<MapProfileSnapshot>> {
        if Self::owns_map_user(&owner_id, cx) {
            return Task::ready(GuideSandbox::shared(cx).read(cx).map_panda_profile());
        }
        Self::actions(cx, |actions, cx| actions.profile_action(owner_id, tab, more, cx))
    }

    pub fn load_peer_profile(&mut self, owner_id: String, cx: &mut Context<Self>) -> Task<Option<MapProfile>> {
        if Self::owns_map_user(&owner_id, cx) {
            let profile = GuideSandbox::shared(cx).read(cx).map_panda_profile().and_then(|snapshot| snapshot.profile);
            return Task::ready(profile);
        }
        Self::actions(cx, |actions, cx| actions.peer_profile_action(owner_id, cx))
    }

    pub fn change_profile_friend_request(
        &mut self,
        user_id: String,
        request_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if Self::owns_map_user(&user_id, cx) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| {
                if request_id.is_some() {
                    sandbox.cancel_map_friendship(cx);
                } else {
                    sandbox.request_map_friendship(cx);
                }
            });
            return Task::ready(true);
        }
        if let Some(request_id) = request_id {
            let task = Self::actions(cx, |actions, cx| actions.cancel_friend_request_action(request_id, cx));
            return cx.spawn(async move |_, _: &mut AsyncApp| task.await == RiseMapMutationOutcome::Done);
        }

        let task = Self::actions(cx, |actions, cx| actions.send_friend_request_action(user_id, cx));
        cx.spawn(async move |_, _: &mut AsyncApp| !matches!(task.await, RiseMapFriendRequestResult::Failed(_)))
    }

    pub fn preview_invite(&mut self, token: String, cx: &mut Context<Self>) -> Task<RiseMapInvitePreviewResult> {
        Self::actions(cx, |actions, cx| actions.preview_invite_action(token, cx))
    }

    pub fn use_invite(&mut self, token: String, cx: &mut Context<Self>) -> Task<RiseMapFriendRequestResult> {
        Self::actions(cx, |actions, cx| actions.use_invite_action(token, cx))
    }

    pub fn send_reaction(
        &mut self,
        user_id: &str,
        emoji: &str,
        source: Point<Pixels>,
        custom: Option<StickerItem>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !Self::is_valid_reaction(emoji)
            || self.visible_screens <= 0
            || self.selected_friend_id.as_deref() != Some(user_id)
        {
            return false;
        }
        let target = user_id.to_owned();
        let emoji = emoji.to_owned();
        if Self::owns_map_user(user_id, cx) {
            cx.defer(move |cx| {
                MapReactionStage::shared(cx)
                    .update(cx, |stage, cx| stage.send(&emoji, source, &target, custom, cx));
                GuideSignalBus::emit(RiseGuideSignal::MapReactionSent);
            });
            return true;
        }
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let generation = self.reaction_playback_generation;
        let enqueue =
            Self::actions(cx, |actions, cx| actions.enqueue_reaction_action(target.clone(), emoji.clone(), cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            if !enqueue.await {
                return;
            }
            let is_current = this
                .read_with(cx, |store, _| {
                    generation == store.reaction_playback_generation
                        && store.selected_friend_id.as_deref() == Some(target.as_str())
                })
                .unwrap_or(false);
            if !is_current || !Self::is_current_account(&account, &epoch) {
                return;
            }
            cx.update(|cx| {
                MapReactionStage::shared(cx).update(cx, |stage, cx| stage.send(&emoji, source, &target, custom, cx));
            });
            GuideSignalBus::emit(RiseGuideSignal::MapReactionSent);
        })
        .detach();
        true
    }

    pub fn note_reaction_burst(&mut self, count: i64) {
        if count != 3 {
            return;
        }
        GuideSignalBus::emit(RiseGuideSignal::MapReactionBurst);
    }

    pub fn reaction_selection_changed(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        self.reaction_playback_generation = self.reaction_playback_generation.wrapping_add(1);
        self.reaction_playback_task = None;
        MapReactionStage::shared(cx).update(cx, |stage, cx| stage.cancel_flights(cx));
        if user_id.is_some() {
            self.play_selected_reactions_if_needed(cx);
        }
    }

    fn is_reaction_playback_current(
        &self,
        generation: u64,
        sender: &str,
        account: &Option<String>,
        epoch: &Option<String>,
        cx: &App,
    ) -> bool {
        self.reaction_playback_generation == generation
            && self.selected_friend_id.as_deref() == Some(sender)
            && self.visible_screens > 0
            && Self::is_app_active(cx)
            && Self::is_current_account(account, epoch)
    }

    pub fn play_selected_reactions_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.visible_screens <= 0 || !Self::is_app_active(cx) || self.reaction_playback_task.is_some() {
            return;
        }
        let Some(sender) = self.selected_friend_id.clone() else {
            return;
        };
        if !MapServicesStore::reaction_inbox(cx).iter().any(|entry| entry.sender_id == sender && entry.count > 0) {
            return;
        }
        let generation = self.reaction_playback_generation;
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        self.reaction_playback_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let current = |cx: &mut AsyncApp| {
                this.read_with(cx, |store, cx| {
                    store.is_reaction_playback_current(generation, &sender, &account, &epoch, cx)
                })
                .unwrap_or(false)
            };
            let outcome: anyhow::Result<bool> = async {
                if !current(cx) {
                    return Ok(false);
                }
                let claim = cx.update(|cx| {
                    Self::actions(cx, |actions, cx| actions.claim_reactions_action(sender.clone(), cx))
                });
                let mut page = claim.await?;
                if !current(cx) {
                    return Ok(false);
                }
                cx.background_executor().timer(Duration::from_millis(700)).await;
                while current(cx) {
                    for reaction in page.reactions.clone() {
                        if !current(cx) {
                            return Ok(false);
                        }
                        let play = cx.update(|cx| {
                            MapReactionStage::shared(cx).update(cx, |stage, cx| stage.play(reaction, sender.clone(), cx))
                        });
                        play.await;
                    }
                    if !current(cx) || !page.has_more || page.claim_id.is_empty() {
                        break;
                    }
                    let cursor = page.next_cursor;
                    let claim_id = page.claim_id.clone();
                    let next = cx.update(|cx| {
                        Self::actions(cx, |actions, cx| actions.reaction_page_action(sender.clone(), claim_id, cursor, cx))
                    });
                    page = next.await?;
                    if page.next_cursor <= cursor {
                        break;
                    }
                }
                Ok(!page.reactions.is_empty())
            }
            .await;
            let completed = match outcome {
                Ok(completed) => completed,
                Err(error) => {
                    if current(cx) && !error.is::<CancellationError>() {
                        cx.update(|cx| Self::show_failure(RiseMapFailure::text(&error), cx));
                    }
                    false
                }
            };
            let _ = this.update(cx, |store, cx| {
                if store.reaction_playback_generation != generation {
                    return;
                }
                store.reaction_playback_task = None;
                if completed && store.is_reaction_playback_current(generation, &sender, &account, &epoch, cx) {
                    store.play_selected_reactions_if_needed(cx);
                }
            });
        }));
    }

    fn load_invite_preview(&mut self, token: String, cx: &mut Context<Self>) {
        let generation = self.invite_generation;
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let preview = Self::actions(cx, |actions, cx| actions.preview_invite_action(token.clone(), cx));
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = preview.await;
            let _ = this.update(cx, |store, cx| {
                if store.invite_sheet.as_ref().map(|sheet| sheet.token.as_str()) != Some(token.as_str())
                    || store.invite_generation != generation
                    || !Self::is_current_account(&account, &epoch)
                {
                    return;
                }

                if let Some(sheet) = store.invite_sheet.as_mut() {
                    sheet.phase = match result {
                        RiseMapInvitePreviewResult::Loaded(preview) => MapInviteSheetStatePhase::Preview(preview),
                        RiseMapInvitePreviewResult::Failed(text) => MapInviteSheetStatePhase::Failed(text),
                    };
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_invite_submission(
        &mut self,
        token: String,
        operation: impl FnOnce(&mut App) -> Task<MapInteractionsStoreInviteSubmission> + 'static,
        cx: &mut Context<Self>,
    ) {
        if let Some(sheet) = self.invite_sheet.as_mut() {
            sheet.is_submitting = true;
        }
        cx.notify();
        let generation = self.invite_generation;
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            if !Self::is_current_account(&account, &epoch) {
                return;
            }
            let submission = cx.update(operation);
            let result = submission.await;
            let _ = this.update(cx, |store, cx| {
                if store.invite_sheet.as_ref().map(|sheet| sheet.token.as_str()) != Some(token.as_str())
                    || store.invite_generation != generation
                    || !Self::is_current_account(&account, &epoch)
                {
                    return;
                }
                if let Some(sheet) = store.invite_sheet.as_mut() {
                    sheet.is_submitting = false;
                }

                match result {
                    MapInteractionsStoreInviteSubmission::Success(key) => {
                        store.invite_sheet = None;
                        Self::send_notice(tr(&key), None, NotifierType::Success, cx);
                    }
                    MapInteractionsStoreInviteSubmission::Failure(text) => {
                        if let Some(sheet) = store.invite_sheet.as_mut() {
                            sheet.submission_error = Some(text.unwrap_or_else(|| tr("map_action_failed")));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    async fn announce(outcome: MapFriendRequestOutcome, cx: &mut AsyncApp) {
        if outcome.friend.is_some() {
            cx.update(|cx| Self::send_notice(tr("map_friend_added"), None, NotifierType::Success, cx));
            return;
        }
        if outcome.target_is_member {
            cx.update(|cx| Self::send_notice(tr("map_request_sent"), None, NotifierType::Success, cx));
            return;
        }

        cx.update(|cx| {
            Self::send_notice(
                tr("map_request_sent"),
                Some(tr("map_request_sent_not_member")),
                NotifierType::Info,
                cx,
            )
        });
        let load = cx.update(|cx| Self::actions(cx, |actions, cx| actions.load_invite_action(false, cx)));
        let Some(invite) = load.await else {
            return;
        };
        cx.update(|cx| Self::shared(cx).update(cx, |store, cx| store.share_invite(&invite, cx)));
    }

    fn is_valid_reaction(emoji: &str) -> bool {
        if i64::try_from(emoji.len()).unwrap_or(i64::MAX) > Self::reaction_max_bytes() {
            return false;
        }
        if let Some(item_id) = emoji.strip_prefix(MessageReaction::CUSTOM_EMOJI_PREFIX) {
            return EmojiPickerCatalog::custom_emoji_ref(item_id).as_deref() == Some(emoji);
        }
        !emoji.is_empty() && i64::try_from(character_count(emoji)).unwrap_or(i64::MAX) <= Self::reaction_max_graphemes()
    }

    fn resume_pending_invite(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.pending_invite_token.take() else {
            return;
        };
        let generation = self.onboarding_generation;
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            if !Self::is_current_account(&account, &epoch) {
                return;
            }
            let use_invite = cx.update(|cx| Self::actions(cx, |actions, cx| actions.use_invite_action(token, cx)));
            let result = use_invite.await;
            let _ = this.update(cx, |store, cx| {
                if store.onboarding_generation != generation || !Self::is_current_account(&account, &epoch) {
                    return;
                }

                match result {
                    RiseMapFriendRequestResult::Sent(outcome) => {
                        let key = if outcome.friend.is_none() { "map_request_sent" } else { "map_friend_added" };
                        Self::send_notice(tr(key), None, NotifierType::Success, cx);
                    }
                    RiseMapFriendRequestResult::Failed(text) => Self::show_failure(text, cx),
                }
            });
        })
        .detach();
    }

    fn show_failure(text: Option<String>, cx: &mut App) {
        Self::send_notice(text.unwrap_or_else(|| tr("map_action_failed")), None, NotifierType::Error, cx);
    }

    pub fn open_journey_composer(&mut self, cx: &mut Context<Self>) {
        self.assign_selected_friend_id(None, cx);
        self.globe_entry = None;
        self.routes_owner = None;
        self.focused_profile_grid_id = None;
        self.assign_selected_grid_id(None, cx);
        Self::actions(cx, |actions, cx| {
            actions.open_tracks_action(None, false, cx);
            actions.open_photos_action(None, false, cx);
        });
    }

    pub fn toggle_journey_grid(&mut self, id: &str, session: &Entity<MapEditingSession>, cx: &mut Context<Self>) {
        session.update(cx, |session, cx| {
            if !session.is_current() || session.saving || !session.candidates.iter().any(|grid| grid.id == id) {
                return;
            }
            if session.selected.contains(id) {
                session.selected.remove(id);
                cx.notify();
                return;
            }
            if i64::try_from(session.selected.len()).unwrap_or(i64::MAX)
                >= RiseLimits::service_value("map_journey_max_grids")
            {
                session.error = Some(tr("map_journey_limit"));
                cx.notify();
                return;
            }
            let count: i64 = session
                .candidates
                .iter()
                .filter(|grid| session.selected.contains(&grid.id) || grid.id == id)
                .map(|grid| grid.photo_count)
                .sum();
            let media_limit = RiseLimits::service_value("map_journey_max_media");
            if count > media_limit {
                session.error = Some(tr("map_journey_media_limit").replace("{{limit}}", &media_limit.to_string()));
                cx.notify();
                return;
            }
            session.selected.insert(id.to_owned());
            cx.notify();
        });
    }

    pub fn save_erased_tracks(
        &mut self,
        session: Entity<MapEditingSession>,
        on_saved: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let (is_current, saving, strokes) = {
            let session = session.read(cx);
            (session.is_current(), session.saving, session.strokes.clone())
        };
        if !is_current || saving || strokes.is_empty() {
            return;
        }
        if Self::owns_map_scene(cx) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.erase_map_tail(&strokes, cx));
            on_saved(cx);
            return;
        }
        let strokes = session.update(cx, |session, cx| {
            session.saving = true;
            session.error = None;
            let strokes = session.pending_save.clone().unwrap_or_else(|| session.strokes.clone());
            session.pending_save = Some(strokes.clone());
            cx.notify();
            strokes
        });
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            if !cx.update(|cx| session.read(cx).is_current()) {
                return;
            }
            let erase_session = session.clone();
            let erase = cx.update(|cx| {
                Self::actions(cx, |actions, cx| actions.erase_tracks_action(strokes, erase_session, cx))
            });
            let result = erase.await;
            if !cx.update(|cx| session.read(cx).is_current()) {
                return;
            }
            cx.update(|cx| {
                session.update(cx, |session, cx| {
                    session.saving = false;
                    cx.notify();
                })
            });
            match result {
                RiseMapMutationOutcome::Done => {
                    let _ = this.update(cx, |store, cx| store.refresh_track_tail(cx));
                    cx.update(on_saved);
                    GuideSignalBus::emit(RiseGuideSignal::MapTracksErased);
                }
                RiseMapMutationOutcome::Failed(text) => cx.update(|cx| {
                    session.update(cx, |session, cx| {
                        session.error = Some(text.unwrap_or_else(|| tr("map_action_failed")));
                        cx.notify();
                    })
                }),
            }
        })
        .detach();
    }

    pub fn prepare_journey(
        &mut self,
        selection: MapJourneySelection,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<MapJourneySnapshot>> {
        if let Some(practice) = GuideSandbox::shared(cx).read(cx).prepare_practice_journey(&selection) {
            return Task::ready(Ok(practice));
        }
        Self::actions(cx, |actions, cx| actions.prepare_journey_action(selection, cx))
    }

    pub fn open_journey(&mut self, post_id: i64, scope: Uuid, cx: &mut Context<Self>) -> Task<()> {
        Self::actions(cx, |actions, cx| actions.open_journey_action(post_id, scope, cx))
    }

    pub fn close_journey(&mut self, scope: Uuid, cx: &mut Context<Self>) -> Task<()> {
        Self::actions(cx, |actions, cx| actions.close_journey_action(scope, cx))
    }

    pub fn revoke_journey(&mut self, id: String, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::actions(cx, |actions, cx| actions.revoke_journey_action(id, cx))
    }
}

impl AccountResettable for MapInteractionsStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        MapInteractionsStore::reset_account_state(self, cx);
    }
}
