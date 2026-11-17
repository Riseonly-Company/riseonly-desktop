use std::collections::{HashMap, HashSet};

use gpui::{App, Context};
use rise_canon::RiseGuideSignal;
use rise_canon::RiseIdentityCanon;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_url::Url;
use rise_energy::rise_energy_saving::RiseEnergySavingStore;
use rise_i18n::tr;
use rise_theme::ThemeStore;

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{
    ChatWallpaperLook, ChatWallpaperStore, ChatWallpaperStoreGuideSnapshot,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_types::BugReportRoute;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportInteractionsStore;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_services::bug_report_services::BugReportServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::shared::utils::music::featured_profile_track_store::FeaturedProfileTrackStore;
use crate::modules::notify::components::notifications_hub_sheet_store::NotificationsHubSheetStore;
use crate::modules::notify::stores::notify_actions::notify_profile_settings_wire_types::{
    ProfileNotificationScope, ProfileNotificationSettingsModel,
};
use crate::modules::session::stores::session_actions::session_types::SessionEntry;
use crate::modules::theme::engine::core::rise_theme_wallpaper_wire_models::RiseChatWallpaper;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::{RiseChatWallpaperDraft, RiseChatWallpaperUpload};
use crate::modules::theme::engine::rise_theme_wallpapers_presentation::{
    RiseThemeWallpapersLoadState, RiseThemeWallpapersPresentation, RiseThemeWallpapersSnapshot,
};
use crate::modules::user::engine::core::rise_user_achievement_wire_models::{RiseAchievement, RiseAchievementMediaKind};
use crate::modules::user::engine::rise_user_achievements_presentation::{
    RiseUserAchievementsLoadState, RiseUserAchievementsSnapshot,
};
use crate::modules::user::engine::rise_user_profile_banner_upload::RiseUserProfileBannerUpload;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const OPTIMISTIC_TEMPORARY_PREFIX: &str = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;

impl GuideSandbox {
    pub fn arm_settings_scenes(&mut self, cx: &mut Context<Self>) {
        if self.scenes.contains(&*Self::energy_scene_id()) {
            self.energy_before = Some(RiseEnergySavingStore::shared(cx).read(cx).settings().clone());
        }
        if self.scenes.contains(&*Self::sessions_scene_id()) {
            self.sessions_scene = Some(GuideSandboxSessionsScene::default());
        }
        if self.scenes.contains(&*Self::goals_scene_id()) {
            let store = UserServicesStore::shared(cx);
            let services = store.read(cx);
            self.goals_before = Some((services.goals().to_vec(), services.plans().to_vec()));
        }
        if self.scenes.contains(&*Self::peer_scene_id()) {
            self.panda_notifications = Some(ProfileNotificationSettingsModel {
                target_user_id: Self::panda_user_id(),
                ..ProfileNotificationSettingsModel::default()
            });
        }
        if self.scenes.contains(&*Self::stickers_scene_id()) {
            self.arm_stickers(cx);
        }
        if self.scenes.contains(&*Self::profile_tracks_scene_id()) {
            self.featured_tracks_before = Some(FeaturedProfileTrackStore::shared(cx).update(cx, |store, _| store.guide_snapshot()));
        }
        if self.scenes.contains(&*Self::profile_look_scene_id()) {
            let badge_before = AuthServicesStore::shared(cx)
                .read(cx)
                .profile()
                .and_then(|profile| profile.more.premium_badge_emoji.clone());
            self.profile_look_scene = Some(GuideSandboxProfileLookScene {
                badge_before,
                banner_file: None,
            });
        }
        if self.scenes.contains(&*Self::app_wallpaper_scene_id()) {
            self.arm_app_wallpaper(cx);
        }
        if self.scenes.contains(&*Self::theme_scene_id()) {
            self.theme_before = Some(ThemeStore::shared(cx).read(cx).guide_snapshot());
        }
        if self.scenes.contains(&*Self::achievements_scene_id()) {
            self.achievements_scene = Some(GuideSandboxAchievementsScene::default());
        }
        if self.scenes.contains(&*Self::bug_report_scene_id()) {
            self.bug_report_scene = true;
            BugReportServicesStore::shared(cx).update(cx, |services, cx| services.begin_practice_report(cx));
        }
        if self.scenes.contains(&*Self::mute_scene_id()) {
            self.arm_mute(cx);
        }
        self.arm_jobs(cx);
        cx.notify();
    }

    pub fn owns_bug_report_scene(&self) -> bool {
        self.is_armed && self.bug_report_scene
    }

    pub fn owns_theme_scene(&self) -> bool {
        self.is_armed && self.theme_before.is_some()
    }

    pub fn owns_goals_scene(&self) -> bool {
        self.is_armed && self.goals_before.is_some()
    }

    pub fn withdraw_settings_scenes(&mut self, cx: &mut Context<Self>) {
        self.withdraw_mute(cx);
        self.withdraw_jobs(cx);
        self.withdraw_bots(cx);
        self.withdraw_call(cx);
        if let Some(energy_before) = self.energy_before.clone() {
            RiseEnergySavingStore::shared(cx).update(cx, |store, cx| store.restore(energy_before, cx));
        }
        self.withdraw_app_wallpaper(cx);
        self.withdraw_profile_look(cx);
        self.withdraw_stickers();
        if let Some((user_id, tracks)) = self.featured_tracks_before.clone() {
            FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.restore(user_id, tracks, cx));
        }
        if let Some(theme_before) = self.theme_before.clone() {
            ThemeStore::shared(cx).update(cx, |store, cx| store.restore(theme_before, cx));
        }
        if let Some((goals, plans)) = self.goals_before.clone() {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.restore_goals_and_plans(goals, plans, cx));
        }
        if self.panda_notifications.is_some() {
            let hub = NotificationsHubSheetStore::shared(cx);
            if hub.read(cx).target_user_id() == Some(&*Self::panda_user_id()) {
                hub.update(cx, |hub, cx| {
                    hub.dismiss(cx);
                    hub.did_dismiss(cx);
                });
            }
        }
        if self.bug_report_scene {
            let services = BugReportServicesStore::shared(cx);
            if services.read(cx).route() == BugReportRoute::Sheet {
                BugReportInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.set_sheet_presented(false, cx));
            }
            services.update(cx, |services, cx| services.end_practice_report(cx));
        }
    }
}

/// sessions lesson, a practice device joins the list so everyone has something to end
/// ending only hides it and signing out this computer is refused
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuideSandboxSessionsScene {
    pub ended: HashSet<String>,
}

impl GuideSandboxSessionsScene {
    pub fn practice_session_id() -> String {
        format!("{OPTIMISTIC_TEMPORARY_PREFIX}guide_session")
    }
}

impl GuideSandbox {
    const PRACTICE_SESSION_DEVICE: &'static str = "iPad Air";
    const PRACTICE_SESSION_APP_LINE: &'static str = "Riseonly";
    const PRACTICE_SESSION_LAST_ACTIVE_AGO_SECONDS: f64 = 2.0 * 3_600.0;
    const PRACTICE_SESSION_FIRST_ACTIVE_AGO_SECONDS: f64 = 40.0 * 86_400.0;

    pub fn owns_sessions_scene(&self) -> bool {
        self.is_armed && self.sessions_scene.is_some()
    }

    pub fn practice_sessions(&self, loaded: Vec<SessionEntry>) -> Vec<SessionEntry> {
        let Some(scene) = self.sessions_scene.as_ref().filter(|_| self.owns_sessions_scene()) else {
            return loaded;
        };
        if loaded.is_empty() {
            return loaded;
        }
        let current = loaded.iter().find(|entry| entry.is_current).or_else(|| loaded.first()).cloned();
        let practice = SessionEntry {
            id: GuideSandboxSessionsScene::practice_session_id(),
            device: Self::PRACTICE_SESSION_DEVICE.to_owned(),
            app_line: Self::PRACTICE_SESSION_APP_LINE.to_owned(),
            location: current.as_ref().map(|entry| entry.location.clone()).unwrap_or_default(),
            ip: String::new(),
            is_current: false,
            last_active: Date::from_time_interval_since_1970(
                Self::scene_stamp() - Self::PRACTICE_SESSION_LAST_ACTIVE_AGO_SECONDS,
            ),
            first_active: Some(Date::from_time_interval_since_1970(
                Self::scene_stamp() - Self::PRACTICE_SESSION_FIRST_ACTIVE_AGO_SECONDS,
            )),
        };
        let mut list = loaded;
        let insert_at = current
            .and_then(|first| list.iter().position(|entry| entry.id == first.id))
            .map_or(0, |index| index + 1);
        let insert_at = insert_at.min(list.len());
        list.insert(insert_at, practice);
        list.retain(|entry| !scene.ended.contains(&entry.id));
        list
    }

    pub fn end_practice_sessions(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        if !self.owns_sessions_scene() || ids.is_empty() {
            return;
        }
        if let Some(scene) = self.sessions_scene.as_mut() {
            scene.ended.extend(ids);
        }
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::SessionTerminated);
    }
}

impl GuideSandbox {
    pub fn owns_person_notifications(&self, target_user_id: &str) -> bool {
        self.is_armed && self.panda_notifications.is_some() && target_user_id == &*Self::panda_user_id()
    }

    pub fn person_notifications(&self, target_user_id: &str) -> Option<ProfileNotificationSettingsModel> {
        if !self.owns_person_notifications(target_user_id) {
            return None;
        }
        self.panda_notifications.clone()
    }

    pub fn set_person_notification(&mut self, scope: ProfileNotificationScope, enabled: bool, cx: &mut Context<Self>) {
        let Some(notifications) = self.panda_notifications.as_mut() else {
            return;
        };
        notifications.set(scope, enabled);
        notifications.has_override = true;
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::PersonNotificationsChanged);
    }

    pub fn set_all_person_notifications(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let Some(notifications) = self.panda_notifications.as_mut() else {
            return;
        };
        for scope in ProfileNotificationScope::ALL_CASES {
            notifications.set(scope, enabled);
        }
        notifications.has_override = true;
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::PersonNotificationsChanged);
    }
}

/// achievements lesson, a practice achievement heads the list so there is always one to hide and share
/// hiding only moves it on the device
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuideSandboxAchievementsScene {
    pub hidden: HashMap<i64, bool>,
}

impl GuideSandboxAchievementsScene {
    pub const PRACTICE_ID: i64 = 9_100_000_000_000_000_001;
}

impl GuideSandbox {
    const PRACTICE_ACHIEVEMENT_CODE_KEY: &'static str = "guide_practice";
    const PRACTICE_ACHIEVEMENT_MEDIA_REF: &'static str = "hello";
    const PRACTICE_ACHIEVEMENT_ACCENT: &'static str = "#FF8A3D";

    pub fn owns_achievements_scene(&self) -> bool {
        self.is_armed && self.achievements_scene.is_some()
    }

    pub fn achievements_snapshot(&self, real: RiseUserAchievementsSnapshot) -> RiseUserAchievementsSnapshot {
        let Some(scene) = self.achievements_scene.as_ref().filter(|_| self.owns_achievements_scene()) else {
            return real;
        };
        let mut list: Vec<RiseAchievement> =
            std::iter::once(Self::practice_achievement()).chain(real.achievements.iter().cloned()).collect();
        for achievement in &mut list {
            if let Some(hidden) = scene.hidden.get(&achievement.id) {
                achievement.hidden = *hidden;
            }
        }
        let mut snapshot = real;
        snapshot.visible_achievements = list.iter().filter(|achievement| !achievement.hidden).cloned().collect();
        snapshot.hidden_achievements = list.iter().filter(|achievement| achievement.hidden).cloned().collect();
        snapshot.hidden_count = snapshot.hidden_achievements.len() as i64;
        snapshot.achievements = list;
        if snapshot.load_state != RiseUserAchievementsLoadState::Loaded {
            snapshot.load_state = RiseUserAchievementsLoadState::Loaded;
            snapshot.error = None;
        }
        snapshot
    }

    pub fn set_achievement_hidden(&mut self, id: i64, hidden: bool, cx: &mut Context<Self>) {
        if !self.owns_achievements_scene() {
            return;
        }
        if let Some(scene) = self.achievements_scene.as_mut() {
            scene.hidden.insert(id, hidden);
        }
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::AchievementVisibilityChanged);
    }

    fn practice_achievement() -> RiseAchievement {
        let stamp = (Self::scene_stamp() * 1000.0) as i64;
        RiseAchievement {
            id: GuideSandboxAchievementsScene::PRACTICE_ID,
            code_key: Self::PRACTICE_ACHIEVEMENT_CODE_KEY.to_owned(),
            media_kind: RiseAchievementMediaKind::Builtin,
            media_ref: Self::PRACTICE_ACHIEVEMENT_MEDIA_REF.to_owned(),
            accent_color: Self::PRACTICE_ACHIEVEMENT_ACCENT.to_owned(),
            title: tr("guide_scene_achievement_title"),
            description: tr("guide_scene_achievement_text"),
            how_to_earn: tr("guide_scene_achievement_text"),
            granted_at_ms: stamp,
            seen_at_ms: stamp,
            hidden: false,
            owner_only: false,
            granted_count: 1,
            rarity: String::new(),
        }
    }
}

/// background lesson, reader tries gradient and palette and own photo
/// on the real wallpaper screen
/// old background is noted first and restored after
#[derive(Clone, Debug)]
pub struct GuideSandboxAppWallpaperScene {
    pub before: ChatWallpaperStoreGuideSnapshot,
    pub rows: Vec<RiseChatWallpaper>,
    pub selected_id: Option<Option<String>>,
    pub adopted_files: Vec<Url>,
}

impl GuideSandbox {
    pub fn owns_app_wallpaper_scene(&self) -> bool {
        self.is_armed && self.app_wallpaper_scene.is_some()
    }

    pub fn arm_app_wallpaper(&mut self, cx: &mut Context<Self>) {
        let store = ChatWallpaperStore::shared(cx);
        self.app_wallpaper_scene = Some(GuideSandboxAppWallpaperScene {
            before: store.read(cx).guide_snapshot(),
            rows: Vec::new(),
            selected_id: Some(None),
            adopted_files: Vec::new(),
        });
        store.update(cx, |store, cx| store.apply_practice_image_wallpaper(None, None, None, cx));
        cx.notify();
    }

    pub fn withdraw_app_wallpaper(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.app_wallpaper_scene.as_ref() else {
            return;
        };
        let before = scene.before.clone();
        ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.restore(before, cx));
        for file in &scene.adopted_files {
            RiseChatWallpaperUpload::discard_staged_file(file);
        }
    }

    pub fn app_wallpapers_snapshot(&self, real: RiseThemeWallpapersSnapshot) -> RiseThemeWallpapersSnapshot {
        let Some(scene) = self.app_wallpaper_scene.as_ref().filter(|_| self.owns_app_wallpaper_scene()) else {
            return real;
        };
        let mut snapshot = real;
        snapshot.wallpapers = scene.rows.iter().cloned().chain(snapshot.wallpapers).collect();
        if let Some(selected) = scene.selected_id.clone() {
            snapshot.selected_id = selected;
        }
        if snapshot.load_state != RiseThemeWallpapersLoadState::Loaded {
            snapshot.load_state = RiseThemeWallpapersLoadState::Loaded;
            snapshot.error = None;
        }
        snapshot
    }

    /// the gallery photo as its own row, drawn from the exported file
    pub fn adopt_app_wallpaper(
        &mut self,
        draft: RiseChatWallpaperDraft,
        look: ChatWallpaperLook,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if !self.owns_app_wallpaper_scene() {
            return None;
        }
        let scene = self.app_wallpaper_scene.as_mut()?;
        let id = format!("{}_{}", Self::app_wallpaper_scene_id(), scene.adopted_files.len() + 1);
        let row = RiseChatWallpaper {
            id: id.clone(),
            preview_url: draft.local_url.absolute_string().to_owned(),
            image_url: draft.local_url.absolute_string().to_owned(),
            created_at_ms: (RiseClock::now().time_interval_since_1970() * 1000.0) as i64,
            dim_percent: look.dim_percent,
            blur_percent: look.blur_percent,
            ..RiseChatWallpaper::default()
        };
        scene.rows.insert(0, row);
        scene.adopted_files.push(draft.local_url);
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::AppWallpaperAdded);
        Some(id)
    }

    pub fn select_app_wallpaper(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        if !self.owns_app_wallpaper_scene() {
            return;
        }
        let row = id.and_then(|id| self.app_wallpaper_row(id, cx));
        if let Some(scene) = self.app_wallpaper_scene.as_mut() {
            scene.selected_id = Some(row.as_ref().map(|row| row.id.clone()));
        }
        cx.notify();
        let (row_id, url, look) = match row.as_ref() {
            Some(row) => (Some(row.id.as_str()), Some(row.image_url.as_str()), Some(row.look())),
            None => (None, None, None),
        };
        ChatWallpaperStore::shared(cx)
            .update(cx, |store, cx| store.apply_practice_image_wallpaper(row_id, url, look, cx));
        if row.is_some() {
            GuideSignalBus::emit(RiseGuideSignal::AppWallpaperSet);
        }
    }

    fn app_wallpaper_row(&self, id: &str, cx: &mut App) -> Option<RiseChatWallpaper> {
        let real = RiseThemeWallpapersPresentation::shared(cx).read(cx).snapshot().clone();
        self.app_wallpapers_snapshot(real).wallpaper(id).cloned()
    }

    pub fn update_app_wallpaper_look(&mut self, id: &str, look: ChatWallpaperLook, cx: &mut Context<Self>) {
        let Some(row) = self
            .app_wallpaper_scene
            .as_mut()
            .and_then(|scene| scene.rows.iter_mut().find(|row| row.id == id))
        else {
            return;
        };
        row.dim_percent = look.dim_percent;
        row.blur_percent = look.blur_percent;
        cx.notify();
    }

    pub fn delete_app_wallpaper(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(scene) = self.app_wallpaper_scene.as_mut() else {
            return;
        };
        if !scene.rows.iter().any(|row| row.id == id) {
            return;
        }
        let was_selected = scene.selected_id.as_ref().is_some_and(|selected| selected.as_deref() == Some(id));
        scene.rows.retain(|row| row.id != id);
        cx.notify();
        if was_selected {
            self.select_app_wallpaper(None, cx);
        }
    }
}

/// profile look lesson with a gallery banner and an emoji by the name, device only
/// badge row shows for everyone and the last step says it needs premium
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuideSandboxProfileLookScene {
    pub badge_before: Option<String>,
    pub banner_file: Option<Url>,
}

impl GuideSandbox {
    pub fn owns_profile_look_scene(&self) -> bool {
        self.is_armed && self.profile_look_scene.is_some()
    }

    pub fn practice_banner_url(&self) -> Option<Url> {
        if !self.owns_profile_look_scene() {
            return None;
        }
        self.profile_look_scene.as_ref().and_then(|scene| scene.banner_file.clone())
    }

    /// none takes the practice banner away like the trash button
    pub fn adopt_practice_banner(&mut self, file: Option<Url>, cx: &mut Context<Self>) {
        if !self.owns_profile_look_scene() {
            return;
        }
        let Some(scene) = self.profile_look_scene.as_mut() else {
            return;
        };
        if let Some(old) = scene.banner_file.as_ref()
            && Some(old) != file.as_ref()
        {
            RiseUserProfileBannerUpload::discard_staged_file(old);
        }
        scene.banner_file = file;
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::ProfileBannerChanged);
    }

    pub fn withdraw_profile_look(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.profile_look_scene.clone() else {
            return;
        };
        if let Some(file) = scene.banner_file.as_ref() {
            RiseUserProfileBannerUpload::discard_staged_file(file);
        }
        let badge_now = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .and_then(|profile| profile.more.premium_badge_emoji.clone());
        if badge_now != scene.badge_before {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.apply_premium_badge_optimistic(scene.badge_before.clone(), cx);
                services.commit_premium_badge_optimistic(cx);
            });
        }
    }
}
