use std::collections::HashMap;

use gpui::Context;
use rise_canon::rise_service_text::music_service;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_errors::ErrorCode;
use rise_i18n::tr;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::{SaiCacheCleanLayer, SaiCacheCleaner};
use crate::modules::music::stores::music_actions::music_types::{
    MusicPlaylistInvitePreview, MusicPlaylistInviteSheetPhase, MusicPlaylistInviteSheetState,
};
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

impl MusicServicesStore {
    pub fn begin_playlist_invite_sheet(&mut self, token: &str, cx: &mut Context<Self>) -> Option<Uuid> {
        let token = trimming_whitespaces_and_newlines(token);
        if token.is_empty() {
            return None;
        }

        if let Some(current) = &self.playlist_invite_sheet {
            if current.is_submitting() {
                return None;
            }
        }

        if let Some(current) = &self.playlist_invite_sheet {
            if current.token == token {
                match current.phase {
                    MusicPlaylistInviteSheetPhase::Loading => return None,
                    MusicPlaylistInviteSheetPhase::Preview(_) => {
                        return self.refresh_playlist_invite_preview(None, cx).map(|(_, request_id)| request_id);
                    }
                    MusicPlaylistInviteSheetPhase::Failed { .. } => {}
                }
            }
        }

        let mut state = MusicPlaylistInviteSheetState::new(token.to_owned());
        if let Some(cached) = self.playlist_invite_preview(token, cx) {
            state.phase = MusicPlaylistInviteSheetPhase::Preview(self.localized_invite_preview(cached, cx));
        }
        let preview_request_id = state.preview_request_id;
        self.playlist_invite_sheet = Some(state);
        cx.notify();
        Some(preview_request_id)
    }

    pub fn retry_playlist_invite_preview(&mut self, cx: &mut Context<Self>) -> Option<(String, Uuid)> {
        let mut state = self.playlist_invite_sheet.clone()?;
        if state.is_loading() || state.is_submitting() {
            return None;
        }

        state.preview_request_id = Uuid::new_v4();
        state.phase = MusicPlaylistInviteSheetPhase::Loading;
        state.submission_error = None;
        let request = (state.token.clone(), state.preview_request_id);
        self.playlist_invite_sheet = Some(state);
        cx.notify();
        Some(request)
    }

    /// starts a fresh invite preview load for the open sheet, a shown preview stays until it lands
    /// with a playlist id a sheet for another playlist is left alone
    pub fn refresh_playlist_invite_preview(
        &mut self,
        playlist_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<(String, Uuid)> {
        let mut state = self.playlist_invite_sheet.clone()?;
        if state.is_loading() || state.is_submitting() {
            return None;
        }
        if let (Some(playlist_id), Some(preview)) = (playlist_id, state.preview()) {
            if preview.playlist_id != playlist_id {
                return None;
            }
        }

        state.preview_request_id = Uuid::new_v4();
        if state.preview().is_none() {
            state.phase = MusicPlaylistInviteSheetPhase::Loading;
        }
        state.submission_error = None;
        let request = (state.token.clone(), state.preview_request_id);
        self.playlist_invite_sheet = Some(state);
        cx.notify();
        Some(request)
    }

    pub fn resolve_playlist_invite_preview(
        &mut self,
        token: &str,
        request_id: Uuid,
        preview: Option<MusicPlaylistInvitePreview>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut state) = self.playlist_invite_sheet.clone() else {
            return false;
        };
        if state.token != token || state.preview_request_id != request_id {
            return false;
        }

        if preview.is_none() {
            SaiCacheCleaner::clean(
                &[Self::playlist_invite_preview_cache_key(token)],
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
        }
        state.phase = match preview {
            Some(preview) => MusicPlaylistInviteSheetPhase::Preview(self.localized_invite_preview(preview, cx)),
            None => MusicPlaylistInviteSheetPhase::Failed {
                message: tr("music_playlist_preview_unavailable"),
                retryable: false,
            },
        };
        state.submission_error = None;
        self.playlist_invite_sheet = Some(state);
        cx.notify();
        true
    }

    pub fn fail_playlist_invite_preview(
        &mut self,
        token: &str,
        request_id: Uuid,
        error: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut state) = self.playlist_invite_sheet.clone() else {
            return false;
        };
        if state.token != token || state.preview_request_id != request_id {
            return false;
        }

        let unavailable = self.is_unavailable_playlist_invite_failure(error);
        if !(unavailable || state.preview().is_none()) {
            return false;
        }

        if unavailable {
            SaiCacheCleaner::clean(
                &[Self::playlist_invite_preview_cache_key(token)],
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
        }
        state.phase = MusicPlaylistInviteSheetPhase::Failed {
            message: tr(if unavailable {
                "music_playlist_preview_unavailable"
            } else {
                "music_playlist_load_error"
            }),
            retryable: !unavailable,
        };
        state.submission_error = None;
        self.playlist_invite_sheet = Some(state);
        cx.notify();
        true
    }

    pub fn begin_playlist_join_submission(&mut self, token: &str, cx: &mut Context<Self>) -> Option<Uuid> {
        let mut state = self.playlist_invite_sheet.clone()?;
        let can_request = state.preview().map(|preview| preview.viewer_state.as_str())
            == Some(music_service::PLAYLIST_VIEWER_STATE_CAN_REQUEST);
        if state.token != token || !can_request || state.is_submitting() {
            return None;
        }

        let submission_id = Uuid::new_v4();
        state.submission_id = Some(submission_id);
        state.submission_error = None;
        self.playlist_invite_sheet = Some(state);
        cx.notify();
        Some(submission_id)
    }

    pub fn finish_playlist_join_submission(
        &mut self,
        submission_id: Uuid,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(mut state) = self.playlist_invite_sheet.clone() else {
            return false;
        };
        if state.submission_id != Some(submission_id) {
            return false;
        }

        state.submission_id = None;
        state.submission_error = error;
        self.playlist_invite_sheet = Some(state);
        cx.notify();
        true
    }

    pub fn dismiss_playlist_invite_sheet(&mut self, cx: &mut Context<Self>) -> bool {
        if self.playlist_invite_sheet.as_ref().is_some_and(MusicPlaylistInviteSheetState::is_submitting) {
            return false;
        }
        self.playlist_invite_sheet = None;
        cx.notify();
        true
    }

    fn localized_invite_preview(
        &self,
        preview: MusicPlaylistInvitePreview,
        cx: &mut Context<Self>,
    ) -> MusicPlaylistInvitePreview {
        let known = match self.playlist(&preview.playlist_id, cx) {
            Some(known) => Some(known),
            None => self
                .playlists_mine(cx)
                .into_iter()
                .find(|playlist| playlist.id.as_deref() == Some(preview.playlist_id.as_str())),
        };
        let Some(known) = known else {
            return preview;
        };
        if known.is_default_local != Some(true) || known.title.as_deref() != Some(preview.title.as_str()) {
            return preview;
        }
        MusicPlaylistInvitePreview {
            title: known.display_title(),
            ..preview
        }
    }

    /// server names a dead invite (revoked or playlist gone) by PLAYLIST_INVITE_INVALID
    /// it arrives here as that codes canon phrase in the viewer language
    fn is_canon_playlist_failure(raw: Option<&str>, code: ErrorCode) -> bool {
        let Some(phrase) = raw.map(trimming_whitespaces_and_newlines).filter(|phrase| !phrase.is_empty()) else {
            return false;
        };
        CanonErrors::text(Some(code.wire()), &HashMap::new()).as_deref() == Some(phrase)
    }

    fn is_unavailable_playlist_invite_failure(&self, raw: Option<&str>) -> bool {
        if Self::is_canon_playlist_failure(raw, ErrorCode::PlaylistInviteInvalid) {
            return true;
        }
        let value = self.normalized_playlist_invite_failure(raw);
        value.contains("expired")
            || value.contains("revoked")
            || value.contains("invalid")
            || value.contains("not found")
    }

    pub fn localized_playlist_join_failure(&self, raw: Option<&str>) -> String {
        if Self::is_canon_playlist_failure(raw, ErrorCode::PlaylistJoinQueueFull) {
            return raw.map(trimming_whitespaces_and_newlines).unwrap_or_default().to_owned();
        }
        if Self::is_canon_playlist_failure(raw, ErrorCode::PlaylistInviteInvalid) {
            return tr("music_playlist_preview_unavailable");
        }
        let value = self.normalized_playlist_invite_failure(raw);
        if value.contains("limit") || value.contains("full") || value.contains("capacity") {
            return tr("music_playlist_preview_capacity");
        }
        if value.contains("expired")
            || value.contains("revoked")
            || value.contains("invalid")
            || value.contains("not found")
        {
            return tr("music_playlist_preview_unavailable");
        }
        tr("music_playlist_request_error")
    }

    pub fn is_pending_playlist_join_failure(&self, raw: Option<&str>) -> bool {
        let value = self.normalized_playlist_invite_failure(raw);
        value.contains("already") && value.contains("pending")
    }

    fn normalized_playlist_invite_failure(&self, raw: Option<&str>) -> String {
        raw.map(|raw| trimming_whitespaces_and_newlines(raw).to_lowercase()).unwrap_or_default()
    }
}
