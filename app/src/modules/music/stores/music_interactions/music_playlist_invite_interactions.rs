use std::rc::Rc;

use gpui::{App, Context};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::{MusicPlaylistInvitePreview, MusicPlaylistJoinRequest};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

impl MusicInteractionsStore {
    pub fn open_playlist_invite_link(&mut self, token: &str, cx: &mut Context<Self>) {
        let clean = trimming_whitespaces_and_newlines(token).to_owned();
        if clean.is_empty() {
            return;
        }
        let Some(request_id) =
            MusicServicesStore::shared(cx).update(cx, |service, cx| service.begin_playlist_invite_sheet(&clean, cx))
        else {
            return;
        };

        self.load_playlist_invite_sheet_preview(clean, request_id, cx);
    }

    pub fn retry_playlist_invite_preview(&mut self, cx: &mut Context<Self>) {
        let Some((token, request_id)) =
            MusicServicesStore::shared(cx).update(cx, |service, cx| service.retry_playlist_invite_preview(cx))
        else {
            return;
        };

        self.load_playlist_invite_sheet_preview(token, request_id, cx);
    }

    pub fn refresh_open_playlist_invite_preview(&mut self, playlist_id: &str, cx: &mut Context<Self>) {
        let Some((token, request_id)) = MusicServicesStore::shared(cx)
            .update(cx, |service, cx| service.refresh_playlist_invite_preview(Some(playlist_id), cx))
        else {
            return;
        };

        self.load_playlist_invite_sheet_preview(token, request_id, cx);
    }

    pub fn request_playlist_join_from_sheet(
        &mut self,
        token: &str,
        on_done: impl Fn(bool, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let service = MusicServicesStore::shared(cx);
        let Some(submission_id) = service.update(cx, |service, cx| service.begin_playlist_join_submission(token, cx))
        else {
            return;
        };

        let on_done: Rc<dyn Fn(bool, &mut App)> = Rc::new(on_done);
        let success_service = service.clone();
        let success_done = Rc::clone(&on_done);
        let on_success: Rc<dyn Fn(Option<MusicPlaylistJoinRequest>, &mut App)> =
            Rc::new(move |_: Option<MusicPlaylistJoinRequest>, cx: &mut App| {
                let finished = success_service
                    .update(cx, |service, cx| service.finish_playlist_join_submission(submission_id, None, cx));
                if !finished {
                    return;
                }
                success_done(true, cx);
            });
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |error: String, cx: &mut App| {
            let outcome = service.update(cx, |service, cx| {
                if service.is_pending_playlist_join_failure(Some(&error)) {
                    return service.finish_playlist_join_submission(submission_id, None, cx).then_some(true);
                }
                let failure = service.localized_playlist_join_failure(Some(&error));
                service.finish_playlist_join_submission(submission_id, Some(failure), cx).then_some(false)
            });
            if let Some(joined) = outcome {
                on_done(joined, cx);
            }
        });

        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        MusicActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.request_playlist_join_action(token, &client_request_id, Some(on_success), Some(on_error), cx);
        });
    }

    pub fn dismiss_playlist_invite_sheet(&mut self, cx: &mut Context<Self>) {
        MusicServicesStore::shared(cx).update(cx, |service, cx| {
            let _ = service.dismiss_playlist_invite_sheet(cx);
        });
    }

    fn load_playlist_invite_sheet_preview(&mut self, token: String, request_id: Uuid, cx: &mut Context<Self>) {
        let service = MusicServicesStore::shared(cx);
        let success_service = service.clone();
        let success_token = token.clone();
        let on_success: Rc<dyn Fn(Option<MusicPlaylistInvitePreview>, &mut App)> =
            Rc::new(move |preview: Option<MusicPlaylistInvitePreview>, cx: &mut App| {
                success_service.update(cx, |service, cx| {
                    let _ = service.resolve_playlist_invite_preview(&success_token, request_id, preview, cx);
                });
            });
        let error_token = token.clone();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |error: String, cx: &mut App| {
            service.update(cx, |service, cx| {
                let _ = service.fail_playlist_invite_preview(&error_token, request_id, Some(&error), cx);
            });
        });

        MusicActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_playlist_invite_preview_action(&token, false, Some(on_success), Some(on_error), cx);
        });
    }
}
