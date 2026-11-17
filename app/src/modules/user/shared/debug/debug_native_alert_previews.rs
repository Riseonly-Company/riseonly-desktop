#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{AnyWindowHandle, App, RenderOnce, Window};
use rise_canon::RiseLimits;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_ui::native_confirm_alert::{NativeConfirmAlert, NativeConfirmAlertAction};
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationKind,
    BottomSheetDebugRegistrationModule as Module,
};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;

pub enum DebugNativeAlertPreviews {}

impl DebugNativeAlertPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        DEFINITIONS.iter().map(DebugAlertDefinition::registration).collect()
    }
}

const DEFINITIONS: &[DebugAlertDefinition] = &[
    DebugAlertDefinition::new(
        "bot_commands_delete_all.0",
        Module::Bots,
        "bot_commands_delete_all",
        "bot_commands_delete_message",
        "modules/bot/pages/BotCommands.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "bot_commands_delete_all", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "bot_mini_app_delete.1",
        Module::Bots,
        "bot_mini_app_delete",
        "bot_mini_app_delete_message",
        "modules/bot/pages/BotMiniAppEditor.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "bot_mini_app_delete", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "bot_rotate_token_title.2",
        Module::Bots,
        "bot_rotate_token_title",
        "bot_rotate_token_message",
        "modules/bot/pages/BotSettings.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "bot_rotate_token", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "bot_delete_title.3",
        Module::Bots,
        "bot_delete_title",
        "bot_delete_message",
        "modules/bot/pages/BotSettings.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "bot_delete", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "bot_webhook_delete.4",
        Module::Bots,
        "bot_webhook_delete",
        "bot_webhook_delete_message",
        "modules/bot/pages/BotWebhook.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "bot_webhook_delete", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "chat_settings_type_private_warning_title.5",
        Module::Chats,
        "chat_settings_type_private_warning_title",
        "chat_settings_type_private_warning_body",
        "modules/chat/pages/Chats/Chat/ChatSettings/ChatSettingsType/ChatSettingsType.swift",
        &[DebugAlertAction { key: "chat_settings_type_private_warning_action", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "map_grid_delete_title.6",
        Module::Map,
        "map_grid_delete_title",
        "map_grid_delete_text",
        "modules/map/components/MapPhotoGridCard.swift",
        &[DebugAlertAction { key: "map_grid_delete", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "map_drafts_clear_title.7",
        Module::Map,
        "map_drafts_clear_title",
        "map_drafts_clear_text",
        "modules/map/components/MapPhotoGridCard.swift",
        &[DebugAlertAction { key: "map_drafts_clear", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "map_drafts_clear_all_title.8",
        Module::Map,
        "map_drafts_clear_all_title",
        "map_drafts_clear_all_text",
        "modules/map/components/MapPhotoGridCard.swift",
        &[DebugAlertAction { key: "map_drafts_clear_all", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "map_friend_remove_title.9",
        Module::Map,
        "map_friend_remove_title",
        "map_friend_remove_message",
        "modules/map/pages/MapFriends/MapFriendsPage.swift",
        &[DebugAlertAction { key: "map_friend_remove_action", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "map_invite_reset_title.10",
        Module::Map,
        "map_invite_reset_title",
        "map_invite_reset_message",
        "modules/map/pages/MapFriends/MapInviteLinkCard.swift",
        &[DebugAlertAction { key: "map_invite_reset_action", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "map_tracks_delete_title.11",
        Module::Map,
        "map_tracks_delete_title",
        "map_tracks_delete_message",
        "modules/map/pages/MapSettings/MapSettingsPage.swift",
        &[DebugAlertAction { key: "map_tracks_delete_action", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "map_leave_title.12",
        Module::Map,
        "map_leave_title",
        "map_leave_message",
        "modules/map/pages/MapSettings/MapSettingsPage.swift",
        &[DebugAlertAction { key: "map_leave_action", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "music_live_close_confirmation_title.13",
        Module::Music,
        "music_live_close_confirmation_title",
        "music_live_close_confirmation_message",
        "modules/music/pages/MusicTab/MiniPlayerBar.swift",
        &[DebugAlertAction { key: "music_live_leave", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "music_live_close_confirmation_title.14",
        Module::Music,
        "music_live_close_confirmation_title",
        "music_live_close_confirmation_message",
        "modules/music/pages/NowPlaying/MusicListeningSessionScreen.swift",
        &[DebugAlertAction { key: "music_live_close", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "music_live_control_locked_title.15",
        Module::Music,
        "music_live_control_locked_title",
        "music_live_control_locked_specific",
        "modules/music/pages/NowPlaying/NowPlaying.swift",
        &[DebugAlertAction { key: "music_live_request_permissions", role: None }, DebugAlertAction { key: "music_live_leave", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "music_live_switch_session_title.16",
        Module::Music,
        "music_live_switch_session_title",
        "music_live_switch_session_message",
        "modules/music/pages/NowPlaying/NowPlaying.swift",
        &[DebugAlertAction { key: "music_live_switch_session_confirm", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "music_playlist_member_remove_title.17",
        Module::Music,
        "music_playlist_member_remove_title",
        "music_playlist_member_remove_message",
        "modules/music/pages/PlaylistCollaboration/PlaylistMemberRightsScreen.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "music_playlist_member_remove_confirm", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "music_playlist_leave_title.18",
        Module::Music,
        "music_playlist_leave_title",
        "music_playlist_leave_message",
        "modules/music/pages/PlaylistCollaboration/PlaylistSettingsScreen.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "music_playlist_leave_confirm", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "music_delete_playlist.19",
        Module::Music,
        "music_delete_playlist",
        "music_confirm_delete_playlist",
        "modules/music/pages/PlaylistCollaboration/PlaylistSettingsScreen.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "music_delete_playlist", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "music_delete_playlist.20",
        Module::Music,
        "music_delete_playlist",
        "music_confirm_delete_playlist",
        "modules/music/pages/PlaylistDetail/PlaylistDetail.swift",
        &[DebugAlertAction { key: "music_delete_playlist", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "music_artist_success_title.21",
        Module::Music,
        "music_artist_success_title",
        "music_artist_success_msg",
        "modules/music/pages/Settings/BecomeArtistSettings.swift",
        &[DebugAlertAction { key: "ok", role: None }],
    ),
    DebugAlertDefinition::new(
        "ctx_delete_track.22",
        Module::Music,
        "ctx_delete_track",
        "music_confirm_delete_track",
        "modules/music/pages/Track/TrackContextMenuRow.swift",
        &[DebugAlertAction { key: "ctx_delete_track", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "music_album_create_success_title.23",
        Module::Music,
        "music_album_create_success_title",
        "music_album_create_success_msg",
        "modules/music/widgets/bottomsheets/Studio/CreateAlbumSheet.swift",
        &[DebugAlertAction { key: "ok", role: None }],
    ),
    DebugAlertDefinition::new(
        "music_upload_success_title.24",
        Module::Music,
        "music_upload_success_title",
        "music_upload_success_msg",
        "modules/music/widgets/bottomsheets/Studio/UploadTrackSheet.swift",
        &[DebugAlertAction { key: "ok", role: None }],
    ),
    DebugAlertDefinition::new(
        "profile_remove_friend_title.25",
        Module::Posts,
        "profile_remove_friend_title",
        "profile_remove_friend_message",
        "modules/post/components/Post/PostLikesSheet.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "profile_remove_friend_confirm", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "error.26",
        Module::Stickers,
        "error",
        "sticker_pack_upload_failed",
        "modules/stickers/components/StickerAssetComposerHost.swift",
        &[DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "retry", role: None }],
    ),
    DebugAlertDefinition::new(
        "delete_pack_confirm_title.27",
        Module::Stickers,
        "delete_pack_confirm_title",
        "delete_pack_confirm_message",
        "modules/stickers/components/StickerPackSheet/StickerPackSheet.swift",
        &[DebugAlertAction { key: "cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "delete_action", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "story_pin_limit_title.28",
        Module::Limits,
        "story_pin_limit_title",
        "story_pin_limit_message",
        "modules/story/components/StoriesGrid/StoriesGridPage.swift",
        &[DebugAlertAction { key: "ok", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "story_close_friends_limit_title.29",
        Module::Limits,
        "story_close_friends_limit_title",
        "story_close_friends_limit_message",
        "modules/story/pages/CloseFriends/CloseFriendsScreen.swift",
        &[DebugAlertAction { key: "ok", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "highlight_delete_title.30",
        Module::Stories,
        "highlight_delete_title",
        "highlight_delete_message",
        "modules/story/pages/StoriesArchive/HighlightsArchiveGrid.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "highlight_menu_delete", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "highlight_remove_story_title.31",
        Module::Profile,
        "highlight_remove_story_title",
        "highlight_remove_last_story_message",
        "modules/user/components/ProfileTop/HighlightEditSheet.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "highlight_remove_story_action", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "highlight_archive_confirm_title.32",
        Module::Profile,
        "highlight_archive_confirm_title",
        "highlight_archive_confirm_message",
        "modules/user/components/ProfileTop/ProfileHighlights.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "highlight_menu_archive", role: None }],
    ),
    DebugAlertDefinition::new(
        "privacy_exceptions_limit_title.33",
        Module::Limits,
        "privacy_exceptions_limit_title",
        "privacy_exceptions_limit_message",
        "modules/user/pages/Profile/Settings/PrivacySettings/PrivacyExceptionsScreen.swift",
        &[DebugAlertAction { key: "ok", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "profile_unsaved_title.34",
        Module::Profile,
        "profile_unsaved_title",
        "profile_unsaved_message",
        "modules/user/pages/Profile/Settings/ProfileSettings/ProfileSettings.swift",
        &[DebugAlertAction { key: "profile_unsaved_discard", role: Some(ButtonRole::Destructive) }, DebugAlertAction { key: "profile_unsaved_keep", role: Some(ButtonRole::Cancel) }],
    ),
    DebugAlertDefinition::new(
        "settings_delete_account_confirm_title.35",
        Module::Profile,
        "settings_delete_account_confirm_title",
        "settings_delete_account_confirm_message",
        "modules/user/pages/Profile/Settings/Settings.swift",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "settings_delete_account_confirm_button", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "vacancy_resume_success_title.36",
        Module::Jobs,
        "vacancy_resume_success_title",
        "vacancy_resume_success_msg",
        "modules/vacancy/pages/CreateResume/CreateResume.swift",
        &[DebugAlertAction { key: "ok", role: None }],
    ),
    DebugAlertDefinition::new(
        "vacancy_create_success_title.37",
        Module::Jobs,
        "vacancy_create_success_title",
        "vacancy_create_success_msg",
        "modules/vacancy/pages/CreateVacancy/CreateVacancy.swift",
        &[DebugAlertAction { key: "ok", role: None }],
    ),
    DebugAlertDefinition::new(
        "native.profile_photos",
        Module::Media,
        "profile_photos_delete_confirm",
        "story_delete_confirm_message",
        "NativeConfirmAlert",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "ctx_delete", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "native.repost",
        Module::Posts,
        "repost_delete_confirm_title",
        "repost_delete_confirm_message",
        "NativeConfirmAlert",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "ctx_delete", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "native.riff",
        Module::Posts,
        "riff_delete_confirm_title",
        "riff_delete_confirm_message",
        "NativeConfirmAlert",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "ctx_delete", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "native.post",
        Module::Posts,
        "post_delete_confirm_title",
        "post_delete_confirm_message",
        "NativeConfirmAlert",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "ctx_delete", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "native.comment",
        Module::Comments,
        "comment_delete_confirm_title",
        "comment_delete_confirm_message",
        "NativeConfirmAlert",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "ctx_delete", role: Some(ButtonRole::Destructive) }],
    ),
    DebugAlertDefinition::new(
        "native.story",
        Module::Stories,
        "story_delete_confirm_title",
        "story_delete_confirm_message",
        "NativeConfirmAlert",
        &[DebugAlertAction { key: "common_cancel", role: Some(ButtonRole::Cancel) }, DebugAlertAction { key: "ctx_delete", role: Some(ButtonRole::Destructive) }],
    ),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonRole {
    Cancel,
    Destructive,
}

#[derive(Clone, Copy)]
struct DebugAlertAction {
    key: &'static str,
    role: Option<ButtonRole>,
}

#[derive(Clone, Copy)]
struct DebugAlertDefinition {
    id: &'static str,
    module: Module,
    title: &'static str,
    message: &'static str,
    source: &'static str,
    actions: &'static [DebugAlertAction],
}

impl DebugAlertDefinition {
    const fn new(
        id: &'static str,
        module: Module,
        title: &'static str,
        message: &'static str,
        source: &'static str,
        actions: &'static [DebugAlertAction],
    ) -> Self {
        Self { id, module, title, message, source, actions }
    }

    fn registration(&self) -> BottomSheetDebugRegistration {
        let definition = *self;
        BottomSheetDebugRegistration::new(
            format!("alert.{}", self.id),
            self.module,
            self.title,
            self.source,
            move |open: Binding<bool>, _: &mut Window, _: &mut App| DebugNativeAlert {
                is_presented: open,
                definition,
            },
        )
        .with_kind(BottomSheetDebugRegistrationKind::Alert)
    }

    fn message_text(&self, cx: &mut App) -> String {
        match self.message {
            "privacy_exceptions_limit_message" => {
                tr_args(self.message, &[FormatArg::Int(RiseLimits::service_value("max_exceptions"))])
            }
            "story_pin_limit_message" => {
                tr_args(self.message, &[FormatArg::Int(StoryInteractionsStore::pinned_stories_limit(cx))])
            }
            "story_close_friends_limit_message" => {
                tr_args(self.message, &[FormatArg::Int(StoryInteractionsStore::close_friends_limit())])
            }
            _ => tr(self.message).replace("%@", "Alex").replace("%d", "100").replace("{name}", "Alex"),
        }
    }

    fn present(self, is_presented: Binding<bool>, host: AnyWindowHandle, cx: &mut App) {
        let close: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let _ = host.update(cx, |_, window, cx| is_presented.set(false, window, cx));
        });
        if NativeConfirmAlert::is_presenting(cx) {
            close(cx);
            return;
        }
        let title = tr(self.title);
        let message = self.message_text(cx);
        let destructive = self.actions.iter().find(|action| action.role == Some(ButtonRole::Destructive));
        let plain = self.actions.iter().find(|action| action.role.is_none());
        let has_cancel = self.actions.iter().any(|action| action.role == Some(ButtonRole::Cancel));
        let on_answer = close.clone();
        let on_answer = move |cx: &mut App| on_answer(cx);
        match (destructive, plain) {
            (Some(destructive), secondary) => NativeConfirmAlert::present_delete(
                &title,
                &message,
                Some(tr(destructive.key)),
                secondary.map(|action| tr(action.key)),
                false,
                secondary.map(|_| close_action(&close)),
                Some(close_action(&close)),
                on_answer,
                cx,
            ),
            (None, Some(plain)) if has_cancel => NativeConfirmAlert::present_notice(
                &title,
                &message,
                Some(tr(plain.key)),
                Some(close_action(&close)),
                on_answer,
                cx,
            ),
            _ => NativeConfirmAlert::present_notice(&title, &message, None, None, on_answer, cx),
        }
        if !NativeConfirmAlert::is_presenting(cx) {
            close(cx);
        }
    }
}

fn close_action(close: &Rc<dyn Fn(&mut App)>) -> NativeConfirmAlertAction {
    let close = close.clone();
    Box::new(move |cx: &mut App| close(cx))
}

#[derive(IntoElement)]
struct DebugNativeAlert {
    is_presented: Binding<bool>,
    definition: DebugAlertDefinition,
}

impl RenderOnce for DebugNativeAlert {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_open = self.is_presented.get(cx);
        let definition = self.definition;
        let is_presented = self.is_presented;
        on_change(
            format!("alert.{}.open", definition.id),
            is_open,
            true,
            window,
            cx,
            move |_, open: &bool, window, cx| {
                if *open {
                    let host = window.window_handle();
                    cx.defer(move |cx| definition.present(is_presented, host, cx));
                }
            },
        );
        div().w(px(0.0)).h(px(0.0))
    }
}
