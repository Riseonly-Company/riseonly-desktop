#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Window, black};
use rise_canon::rise_service_text::music_service;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationMode, BottomSheetDebugRegistrationModule,
};
use crate::modules::music::stores::music_actions::music_types::{
    MusicPlaylistInvitePreview, MusicPlaylistInviteSheetPhase, MusicPlaylistInviteSheetState, MusicUserReference,
};
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::widgets::bottomsheets::now_playing::music_audio_effects_sheet::MusicAudioEffectsSheet;
use crate::modules::music::widgets::bottomsheets::now_playing::music_queue_sheet::MusicQueueSheet;
use crate::modules::music::widgets::bottomsheets::playlists::create_playlist_sheet::CreatePlaylistSheet;
use crate::modules::music::widgets::bottomsheets::playlists::playlist_invite_sheet::PlaylistInviteSheetContent;
use crate::modules::music::widgets::bottomsheets::playlists::playlist_visibility::PlaylistVisibility;
use crate::modules::music::widgets::bottomsheets::studio::create_album_sheet::CreateAlbumSheet;
use crate::modules::music::widgets::bottomsheets::studio::publish_schedule_sheet::PublishScheduleSheet;
use crate::modules::music::widgets::bottomsheets::studio::upload_track_sheet::UploadTrackSheet;
use crate::modules::music::widgets::bottomsheets::track_picker::editor_track_picker_sheet::EditorTrackPickerSheet;
use crate::modules::music::widgets::bottomsheets::track_picker::post_track_picker_sheet::PostTrackPickerSheet;
use crate::modules::music::widgets::bottomsheets::track_picker::track_trim_sheet::TrackTrimSheet;

const MUSIC: BottomSheetDebugRegistrationModule = BottomSheetDebugRegistrationModule::Music;
const LIVE: BottomSheetDebugRegistrationMode = BottomSheetDebugRegistrationMode::Live;

/// a callback without a window closes the preview after the
/// current update when the window can be borrowed again
fn close_deferred(open: &Binding<bool>, cx: &mut App) {
    let open = open.clone();
    cx.defer(move |cx| {
        let Some(window) = cx.active_window() else {
            return;
        };
        let _ = window.update(cx, |_, window, cx| open.set(false, window, cx));
    });
}

pub enum MusicBottomSheetPreviews {}

impl MusicBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        let mut entries = vec![
            BottomSheetDebugRegistration::new(
                "music.playlist.create",
                MUSIC,
                "music_create_playlist",
                "CreatePlaylistSheet",
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    CreatePlaylistSheet::new("music-preview-playlist-create", open)
                },
            )
            .with_mode(LIVE),
            BottomSheetDebugRegistration::new(
                "music.playlist.local",
                MUSIC,
                "music_create_playlist",
                "CreatePlaylistSheet",
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    CreatePlaylistSheet::new("music-preview-playlist-local", open)
                        .locked_visibility(Some(PlaylistVisibility::Local))
                },
            )
            .with_variant(Some("music_playlist_local".to_owned()))
            .with_mode(LIVE),
            BottomSheetDebugRegistration::sheet(
                "music.album.create",
                MUSIC,
                "music_create_album",
                "CreateAlbumSheet",
                vec![0.78, 0.93],
                None,
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    CreateAlbumSheet::new("music-preview-album").on_dismiss(move |cx| close_deferred(&open, cx))
                },
            )
            .with_mode(LIVE),
            BottomSheetDebugRegistration::sheet(
                "music.publish",
                MUSIC,
                "music_publish",
                "PublishScheduleSheet",
                vec![0.6, 0.93],
                None,
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    let published = open.clone();
                    let scheduled = open.clone();
                    PublishScheduleSheet::new(
                        "music-preview-publish",
                        move |window, cx| published.set(false, window, cx),
                        move |_, window, cx| scheduled.set(false, window, cx),
                    )
                    .on_dismiss(move |window, cx| open.set(false, window, cx))
                },
            ),
            BottomSheetDebugRegistration::sheet(
                "music.effects",
                MUSIC,
                "music_effects_title",
                "MusicAudioEffectsSheet",
                vec![0.88, 0.93],
                None,
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    MusicAudioEffectsSheet::new("music-preview-effects", move |window, cx| open.set(false, window, cx))
                },
            )
            .with_mode(LIVE),
            BottomSheetDebugRegistration::sheet(
                "music.queue",
                MUSIC,
                "music_queue_title",
                "MusicQueueSheet",
                vec![0.55, 0.88],
                Some(black()),
                |_: Binding<bool>, _: &mut Window, _: &mut App| MusicQueueSheet::new("music-preview-queue"),
            )
            .with_mode(LIVE),
        ];
        entries.extend(Self::additional());
        entries.extend(Self::invites());
        entries
    }

    fn additional() -> Vec<BottomSheetDebugRegistration> {
        vec![
            BottomSheetDebugRegistration::sheet(
                "music.upload",
                MUSIC,
                "music_upload_track",
                "UploadTrackSheet",
                vec![0.9],
                None,
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    UploadTrackSheet::new("music-preview-upload").on_dismiss(move |cx| close_deferred(&open, cx))
                },
            )
            .with_mode(LIVE),
            BottomSheetDebugRegistration::sheet(
                "music.post_picker",
                MUSIC,
                "post_music_picker_title",
                "PostTrackPickerSheet",
                vec![0.9],
                None,
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    let picked = open.clone();
                    PostTrackPickerSheet::new("music-preview-post-picker", move |_, cx| close_deferred(&picked, cx))
                        .on_dismiss(move |window, cx| open.set(false, window, cx))
                },
            )
            .with_mode(LIVE),
            BottomSheetDebugRegistration::sheet(
                "music.editor_picker",
                MUSIC,
                "post_music_picker_title",
                "EditorTrackPickerSheet",
                vec![0.9],
                None,
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    let picked = open.clone();
                    EditorTrackPickerSheet::new(
                        "music-preview-editor-picker",
                        None,
                        move |_, cx| close_deferred(&picked, cx),
                        |_, _| {},
                        |_, _| {},
                        move |window, cx| open.set(false, window, cx),
                    )
                },
            )
            .with_variant(Some("chat_profile_tab_media".to_owned()))
            .with_mode(LIVE),
            BottomSheetDebugRegistration::sheet(
                "music.trim",
                MUSIC,
                "music_trim_title",
                "TrackTrimSheet",
                vec![0.6],
                None,
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    let confirmed = open.clone();
                    TrackTrimSheet::new(
                        "music-preview-trim",
                        Self::track(),
                        15_000,
                        45_000,
                        move |_, _, window, cx| confirmed.set(false, window, cx),
                    )
                    .on_dismiss(move |window, cx| open.set(false, window, cx))
                },
            ),
        ]
    }

    fn invites() -> Vec<BottomSheetDebugRegistration> {
        let variants: HashMap<&str, &str> = HashMap::from([
            ("can_request", "music_playlist_preview_join"),
            ("pending", "music_playlist_preview_pending"),
            ("member", "music_playlist_preview_open"),
            ("suspended_plan", "music_playlist_preview_suspended"),
        ]);
        ["can_request", "pending", "member", "suspended_plan"]
            .into_iter()
            .map(|state| {
                let module = if state == music_service::PLAYLIST_MEMBER_STATUS_SUSPENDED_PLAN {
                    BottomSheetDebugRegistrationModule::Limits
                } else {
                    MUSIC
                };
                BottomSheetDebugRegistration::sheet(
                    format!("music.invite.{state}"),
                    module,
                    "music_playlist_preview_type",
                    "PlaylistInviteSheetContent",
                    vec![0.62],
                    None,
                    move |open: Binding<bool>, _: &mut Window, _: &mut App| Self::invite_content(state, open),
                )
                .with_variant(variants.get(state).map(|variant| (*variant).to_owned()))
            })
            .collect()
    }

    fn invite_content(state: &str, open: Binding<bool>) -> PlaylistInviteSheetContent {
        let mut sheet_state = MusicPlaylistInviteSheetState::new("debug-preview".to_owned());
        sheet_state.phase = MusicPlaylistInviteSheetPhase::Preview(MusicPlaylistInvitePreview {
            playlist_id: "debug-playlist".to_owned(),
            owner_user_id: "debug-owner".to_owned(),
            title: "Late night sounds".to_owned(),
            description: "A playlist for slow evenings and good company.".to_owned(),
            cover_url: String::new(),
            owner: MusicUserReference {
                id: "debug-owner".to_owned(),
                name: "Alex Morgan".to_owned(),
                tag: "alex".to_owned(),
                logo: String::new(),
                is_premium: false,
                premium_badge_emoji: None,
                name_color: None,
            },
            active_participant_count: 2,
            effective_participant_limit: 2,
            owner_is_premium: false,
            viewer_state: state.to_owned(),
        });
        let (retry, request, opened, profile) = (open.clone(), open.clone(), open.clone(), open);
        PlaylistInviteSheetContent::new(
            sheet_state,
            Rc::new(move |window, cx| retry.set(false, window, cx)),
            Rc::new(move |_, window, cx| request.set(false, window, cx)),
            Rc::new(move |_, window, cx| opened.set(false, window, cx)),
            Rc::new(move |_, _, window, cx| profile.set(false, window, cx)),
        )
    }

    fn track() -> RiseStoredMusicTrack {
        RiseStoredMusicTrack {
            id: Some("debug-track".to_owned()),
            title: Some("Midnight Drive".to_owned()),
            duration_ms: Some(180_000),
            play_count: Some(1200),
            like_count: Some(84),
            is_liked: Some(false),
            status: Some("published".to_owned()),
            genre: Some("Electronic".to_owned()),
            is_exclusive: Some(false),
            ..RiseStoredMusicTrack::default()
        }
    }
}
