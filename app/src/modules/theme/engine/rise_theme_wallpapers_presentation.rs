use std::collections::HashSet;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::theme::engine::core::rise_theme_wallpaper_wire_models::{
    RiseChatWallpaper, RiseWallpaperPreferences,
};

/// load state belongs to the snapshot not to a transport object in the view
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseThemeWallpapersLoadState {
    Idle,
    Loading,
    Loaded,
    Failed,
}

/// wallpapers newest first and only this accounts uploads,
/// shipped ones are drawn by the preset grid,
/// uploading rows sit at the head as pending
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseThemeWallpapersSnapshot {
    pub wallpapers: Vec<RiseChatWallpaper>,
    pub selected_id: Option<String>,
    pub preferences: RiseWallpaperPreferences,
    pub load_state: RiseThemeWallpapersLoadState,
    pub is_refreshing: bool,
    pub mutating_ids: HashSet<String>,
    pub error: Option<String>,
    pub mutation_error: Option<String>,
}

impl RiseThemeWallpapersSnapshot {
    pub fn empty() -> Self {
        Self {
            wallpapers: Vec::new(),
            selected_id: None,
            preferences: RiseWallpaperPreferences::default_preferences(),
            load_state: RiseThemeWallpapersLoadState::Idle,
            is_refreshing: false,
            mutating_ids: HashSet::new(),
            error: None,
            mutation_error: None,
        }
    }

    pub fn selected(&self) -> Option<&RiseChatWallpaper> {
        let selected_id = self.selected_id.as_deref()?;
        self.wallpapers.iter().find(|wallpaper| wallpaper.id == selected_id)
    }

    pub fn wallpaper(&self, id: &str) -> Option<&RiseChatWallpaper> {
        self.wallpapers.iter().find(|wallpaper| wallpaper.id == id)
    }
}

/// publish gate,
/// a snapshot of a wrong account or epoch or session is dropped so a late reply
/// cant repaint after logout
pub struct RiseThemeWallpapersPresentation {
    snapshot: RiseThemeWallpapersSnapshot,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseThemeWallpapersPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseThemeWallpapersSnapshot::empty(),
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseThemeWallpapersSnapshot {
        &self.snapshot
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.snapshot = RiseThemeWallpapersSnapshot::empty();
        cx.notify();
    }

    pub fn publish(
        &mut self,
        next: RiseThemeWallpapersSnapshot,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.account_id.as_deref() != Some(account_id)
            || self.account_epoch.as_deref() != Some(account_epoch)
            || self.session_id != Some(session_id)
        {
            return false;
        }
        if next == self.snapshot {
            return true;
        }
        self.snapshot = next;
        cx.notify();
        true
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.snapshot = RiseThemeWallpapersSnapshot::empty();
        cx.notify();
    }
}
