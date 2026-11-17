use std::collections::HashSet;

use gpui::{App, Context, Entity, Subscription};

use crate::modules::notify::engine::core::rise_announcements_wire_models::RiseAnnouncement;
use crate::modules::notify::engine::core::rise_release_notes_wire_models::RiseReleaseNote;
use crate::modules::notify::engine::rise_announcements_presentation::RiseAnnouncementsPresentation;
use crate::modules::notify::engine::rise_release_notes_presentation::RiseReleaseNotesPresentation;

/// which of whats new and a broadcast may take the screen per launch
/// process scoped, no modal on the launch that made the account
/// one modal per launch, forced update never held
pub struct RiseLaunchInterruptions {
    is_registration_launch: bool,
    on_screen_announcement_id: Option<i64>,
    spent_announcement_id: Option<i64>,
    spent_release_id: Option<i64>,
    requested_announcement_ids: HashSet<i64>,
    releases: Entity<RiseReleaseNotesPresentation>,
    announcements: Entity<RiseAnnouncementsPresentation>,
    _observations: [Subscription; 2],
}

impl RiseLaunchInterruptions {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let releases = RiseReleaseNotesPresentation::shared(cx);
        let announcements = RiseAnnouncementsPresentation::shared(cx);
        let observations = [
            cx.observe(&releases, |_, _, cx| cx.notify()),
            cx.observe(&announcements, |_, _, cx| cx.notify()),
        ];
        Self {
            is_registration_launch: false,
            on_screen_announcement_id: None,
            spent_announcement_id: None,
            spent_release_id: None,
            requested_announcement_ids: HashSet::new(),
            releases,
            announcements,
            _observations: observations,
        }
    }

    pub fn is_registration_launch(&self) -> bool {
        self.is_registration_launch
    }

    pub fn on_screen_announcement_id(&self) -> Option<i64> {
        self.on_screen_announcement_id
    }

    pub fn note_account_created(&mut self, cx: &mut Context<Self>) {
        self.is_registration_launch = true;
        cx.notify();
    }

    pub fn note_announcement_requested(&mut self, announcement_id: i64, cx: &mut Context<Self>) {
        if self.requested_announcement_ids.insert(announcement_id) {
            cx.notify();
        }
    }

    pub fn note_announcement_shown(&mut self, announcement_id: i64, cx: &mut Context<Self>) {
        self.on_screen_announcement_id = Some(announcement_id);
        if self.spent_announcement_id.is_none()
            && !self.requested_announcement_ids.contains(&announcement_id)
        {
            self.spent_announcement_id = Some(announcement_id);
        }
        cx.notify();
    }

    pub fn note_release_note_shown(&mut self, release_id: i64, cx: &mut Context<Self>) {
        if self.spent_release_id.is_none() {
            self.spent_release_id = Some(release_id);
            cx.notify();
        }
    }

    pub fn release_note(&self, cx: &App) -> Option<RiseReleaseNote> {
        let releases = self.releases.read(cx);
        let note = releases.snapshot().current()?;
        if !note.is_presentable() {
            return None;
        }
        if note.is_blocking {
            return Some(note.clone());
        }
        if self.is_registration_launch {
            return None;
        }
        if let Some(spent_release_id) = self.spent_release_id
            && spent_release_id != note.id
        {
            return None;
        }
        if self.spent_release_id.is_none() && self.spent_announcement_id.is_some() {
            return None;
        }
        if let Some(on_screen_announcement_id) = self.on_screen_announcement_id
            && self
                .announcements
                .read(cx)
                .snapshot()
                .current
                .as_ref()
                .map(|current| current.id)
                == Some(on_screen_announcement_id)
        {
            return None;
        }
        Some(note.clone())
    }

    pub fn announcement(&self, cx: &App) -> Option<RiseAnnouncement> {
        if self.releases.read(cx).is_blocking() {
            return None;
        }
        let candidate = self.announcement_candidate(cx)?;
        if Some(candidate.id) == self.on_screen_announcement_id {
            return Some(candidate);
        }
        if self.requested_announcement_ids.contains(&candidate.id) {
            return if self.release_note(cx).is_none() { Some(candidate) } else { None };
        }
        if !self.releases.read(cx).did_refresh() || self.release_note(cx).is_some() {
            return None;
        }
        Some(candidate)
    }

    pub fn is_owed(&self, cx: &App) -> bool {
        self.release_note(cx).is_some() || self.announcement_candidate(cx).is_some()
    }

    fn announcement_candidate(&self, cx: &App) -> Option<RiseAnnouncement> {
        let current = self.announcements.read(cx).snapshot().current.as_ref()?;
        if !current.is_presentable() {
            return None;
        }
        if Some(current.id) == self.on_screen_announcement_id
            || self.requested_announcement_ids.contains(&current.id)
        {
            return Some(current.clone());
        }
        if self.is_registration_launch {
            return None;
        }
        if let Some(spent_announcement_id) = self.spent_announcement_id
            && spent_announcement_id != current.id
        {
            return None;
        }
        if self.spent_announcement_id.is_none() && self.spent_release_id.is_some() {
            return None;
        }
        Some(current.clone())
    }
}
