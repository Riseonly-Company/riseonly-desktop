use std::collections::HashSet;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::user::engine::core::rise_user_profile_photo_wire_models::RiseProfilePhoto;

/// where one album stands, load state lives in the snapshot
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUserProfilePhotosLoadState {
    Idle,
    Loading,
    Loaded,
    Failed,
}

/// one profiles photo album as this device last saw it
/// photos stay newest first, promoting moves is_main not a row
/// for the avatar use main_index, never index 0
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseProfilePhotoAlbum {
    pub user_id: String,
    pub photos: Vec<RiseProfilePhoto>,
    pub total: i64,
    pub has_more: bool,
    pub next_cursor: Option<String>,
    pub is_visible: bool,
    pub load_state: RiseUserProfilePhotosLoadState,
    pub is_refreshing: bool,
    pub mutating_ids: HashSet<String>,
    pub error: Option<String>,
    pub mutation_error: Option<String>,
}

impl RiseProfilePhotoAlbum {
    pub fn id(&self) -> &str {
        &self.user_id
    }

    pub fn main_index(&self) -> Option<usize> {
        self.photos.iter().position(|photo| photo.is_main)
    }

    pub fn main_photo(&self) -> Option<&RiseProfilePhoto> {
        self.main_index().map(|index| &self.photos[index])
    }

    pub fn empty(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            photos: Vec::new(),
            total: 0,
            has_more: false,
            next_cursor: None,
            is_visible: true,
            load_state: RiseUserProfilePhotosLoadState::Idle,
            is_refreshing: false,
            mutating_ids: HashSet::new(),
            error: None,
            mutation_error: None,
        }
    }
}

/// immutable bounded ui state for profile photo albums
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseUserProfilePhotosSnapshot {
    pub albums: Vec<RiseProfilePhotoAlbum>,
}

impl RiseUserProfilePhotosSnapshot {
    pub const EMPTY: Self = Self { albums: Vec::new() };

    pub fn album(&self, user_id: &str) -> Option<&RiseProfilePhotoAlbum> {
        self.albums.iter().find(|album| album.user_id == user_id)
    }
}

/// publish gate, a snapshot from a stale account
/// epoch or session is dropped
pub struct RiseUserProfilePhotosPresentation {
    snapshot: RiseUserProfilePhotosSnapshot,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseUserProfilePhotosPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseUserProfilePhotosSnapshot::EMPTY,
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseUserProfilePhotosSnapshot {
        &self.snapshot
    }

    pub fn activate(&mut self, account_id: &str, account_epoch: &str, session_id: Uuid, cx: &mut Context<Self>) {
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.snapshot = RiseUserProfilePhotosSnapshot::EMPTY;
        cx.notify();
    }

    pub fn publish(
        &mut self,
        next: RiseUserProfilePhotosSnapshot,
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
        self.snapshot = RiseUserProfilePhotosSnapshot::EMPTY;
        cx.notify();
    }
}
