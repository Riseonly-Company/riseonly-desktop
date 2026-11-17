use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, Empty, Entity, Hsla, Pixels, Point, SharedString, Subscription, Window};
use rise_canon::RiseGuideAnchor;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_i18n::tr;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::rise_symbol::RiseSymbol;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::{MusicTrackSharePolicy, ShareSheetStore};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouter;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::music::pages::track::track_row::TrackRow;
use crate::modules::music::shared::utils::music::music_models::{MusicLocalTrackTransferPhase, TrackModel};
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::shared::utils::music::music_track_download_store::MusicTrackDownloadStore;
use crate::modules::music::shared::utils::music::music_track_file_export::MusicTrackSavePolicy;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::playlists::add_to_playlist_sheet::AddToPlaylistSheet;
use crate::modules::music::widgets::bottomsheets::playlists::edit_local_track_sheet::EditLocalTrackSheet;

type TrackContextMenuRowAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackContextSource {
    OwnPlaylist { playlist_id: String },
    External,
    OwnArtistStudio,
}

/// a private upload isnt moderatable, it lives only in its owners local playlists
/// report-service refuses such a report so the menu must not offer it
/// an unpredictable refusal reads as a bug
pub enum MusicTrackReportPolicy {}

impl MusicTrackReportPolicy {
    pub fn can_report(track: &TrackModel) -> bool {
        if track.transfer_phase.is_some() || MusicTrackSharePolicy::is_pending(track) {
            return false;
        }
        track.status.to_lowercase() != RiseMusicTrackStatus::LocalLibrary.wire()
    }
}

pub enum MusicLocalTrackEditPolicy {}

impl MusicLocalTrackEditPolicy {
    pub fn can_edit(track: &TrackModel, current_user_id: Option<&str>) -> bool {
        let Some(current_user_id) = current_user_id.filter(|user_id| !user_id.is_empty()) else {
            return false;
        };
        track.status == RiseMusicTrackStatus::LocalLibrary.wire()
            && track.library_owner_user_id.as_deref() == Some(current_user_id)
    }
}

/// state of the row and what its menu reads besides the like
/// only the two facts wake the row, the player publishes position
/// 2.5 times a second and a list would redraw with it
struct TrackContextMenuRowState {
    show_add_to_playlist: bool,
    show_edit_local_track: bool,
    current_track_id: Option<String>,
    listening_playlist_id: Option<String>,
    can_start_download: bool,
    _player_observation: Subscription,
    _download_observation: Subscription,
}

#[derive(IntoElement)]
pub struct TrackContextMenuRow {
    id: ElementId,
    track: TrackModel,
    context: TrackContextSource,
    is_playing: bool,
    shows_like_button: bool,
    share_playlist_id: Option<String>,
    allows_local_track_editing: bool,
    guide_anchor: Option<RiseGuideAnchor>,
    on_delete_local_track: Option<TrackContextMenuRowAction>,
    on_tap: TrackContextMenuRowAction,
}

impl TrackContextMenuRow {
    pub fn new(id: impl Into<ElementId>, track: TrackModel) -> Self {
        Self {
            id: id.into(),
            track,
            context: TrackContextSource::External,
            is_playing: false,
            shows_like_button: true,
            share_playlist_id: None,
            allows_local_track_editing: false,
            guide_anchor: None,
            on_delete_local_track: None,
            on_tap: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn context(mut self, context: TrackContextSource) -> Self {
        self.context = context;
        self
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn shows_like_button(mut self, shows_like_button: bool) -> Self {
        self.shows_like_button = shows_like_button;
        self
    }

    pub fn share_playlist_id(mut self, share_playlist_id: Option<String>) -> Self {
        self.share_playlist_id = share_playlist_id;
        self
    }

    pub fn allows_local_track_editing(mut self, allows_local_track_editing: bool) -> Self {
        self.allows_local_track_editing = allows_local_track_editing;
        self
    }

    pub fn guide_anchor(mut self, guide_anchor: impl Into<Option<RiseGuideAnchor>>) -> Self {
        self.guide_anchor = guide_anchor.into();
        self
    }

    pub fn on_delete_local_track(mut self, on_delete_local_track: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_delete_local_track = Some(Rc::new(on_delete_local_track));
        self
    }

    pub fn on_tap(mut self, on_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_tap = Rc::new(on_tap);
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn menu_actions(&self, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let track = &self.track;
        let mut actions: Vec<HoldContextMenuAction> = Vec::new();
        let can_start_download = MusicTrackDownloadStore::shared(cx).read(cx).can_start_download(&track.id);

        if track.transfer_phase.is_some() {
            if !track.is_local_transfer_active() {
                if track.is_device_only_draft() {
                    let title = if track.transfer_phase == Some(MusicLocalTrackTransferPhase::WaitingForConnection) {
                        tr("music_local_upload_send_to_server")
                    } else {
                        tr("music_message_import_retry")
                    };
                    actions.push(HoldContextMenuAction::new("retry_local_upload", title).system_image("arrow.up.circle"));
                } else if track.transfer_phase == Some(MusicLocalTrackTransferPhase::Failed)
                    && MusicServicesStore::shared(cx).read(cx).can_retry_message_mp3_import(&track.id)
                {
                    actions.push(
                        HoldContextMenuAction::new("retry_message_import", tr("music_message_import_retry"))
                            .system_image("arrow.up.circle"),
                    );
                }
            }
            if track.can_remove_pending_local_track() {
                actions.push(
                    HoldContextMenuAction::new("remove_local_draft", tr("ctx_remove_from_playlist"))
                        .system_image("trash")
                        .destructive(true),
                );
            }
            if MusicTrackSavePolicy::can_save(track) {
                actions.push(
                    HoldContextMenuAction::new("save", tr("ctx_save"))
                        .system_image("arrow.down.doc")
                        .disabled(!can_start_download),
                );
            }
            return actions;
        }

        let is_liked = MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.is_liked(track, cx));
        actions.push(
            HoldContextMenuAction::new("like", if is_liked { tr("ctx_unlike") } else { tr("ctx_like") })
                .system_image(if is_liked { "heart.slash" } else { "heart" }),
        );

        if self.allows_local_track_editing {
            let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
            if MusicLocalTrackEditPolicy::can_edit(track, current_user_id.as_deref()) {
                actions.push(HoldContextMenuAction::new("edit_local_track", tr("ctx_edit")).system_image("pencil"));
            }
        }

        match self.context {
            TrackContextSource::OwnPlaylist { .. } => {
                actions.push(
                    HoldContextMenuAction::new("remove_from_playlist", tr("ctx_remove_from_playlist"))
                        .system_image("minus.circle")
                        .destructive(false),
                );
            }
            TrackContextSource::External | TrackContextSource::OwnArtistStudio => {
                actions.push(
                    HoldContextMenuAction::new("add_to_playlist", tr("ctx_add_to_playlist"))
                        .system_image("music.note.list"),
                );
            }
        }

        let is_current_track = MusicPlayerState::shared(cx)
            .read(cx)
            .current_track
            .as_ref()
            .is_some_and(|current| current.id == track.id);
        if !is_current_track {
            actions.push(
                HoldContextMenuAction::new("play_next", tr("ctx_play_next"))
                    .system_image("text.line.first.and.arrowtriangle.forward"),
            );
            actions.push(HoldContextMenuAction::new("add_to_queue", tr("ctx_add_to_queue")).system_image("text.badge.plus"));
        }

        let resolved_share_playlist_id = self.resolved_share_playlist_id(cx);
        if MusicTrackSharePolicy::can_share(track, resolved_share_playlist_id.as_deref()) {
            actions.push(HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"));
        }

        if MusicTrackSavePolicy::can_save(track) {
            actions.push(
                HoldContextMenuAction::new("save", tr("ctx_save"))
                    .system_image("arrow.down.doc")
                    .disabled(!can_start_download),
            );
        }

        if !track.artist_id.is_empty() {
            actions.push(HoldContextMenuAction::new("go_to_artist", tr("ctx_go_to_artist")).system_image(RiseSymbol::ARTIST));
        }

        if !track.album_id.is_empty() {
            actions.push(HoldContextMenuAction::new("go_to_album", tr("ctx_go_to_album")).system_image("square.stack"));
        }

        if self.on_delete_local_track.is_some() {
            actions.push(
                HoldContextMenuAction::new("delete_local_track", tr("ctx_delete"))
                    .system_image("trash")
                    .destructive(true),
            );
        } else if self.context == TrackContextSource::OwnArtistStudio {
            actions.push(
                HoldContextMenuAction::new("delete_track", tr("ctx_delete_track"))
                    .system_image("trash")
                    .destructive(true),
            );
        } else if MusicTrackReportPolicy::can_report(track) {
            actions.push(
                HoldContextMenuAction::new("report", tr("ctx_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            );
        }

        actions
    }

    fn handle_action(
        &self,
        id: &str,
        state: &Entity<TrackContextMenuRowState>,
        nav_store: Option<&Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let track = &self.track;
        match id {
            "retry_local_upload" => {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.retry_pending_local_track(&track.id, cx));
            }
            "retry_message_import" => {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.retry_message_mp3_import(&track.id, cx));
            }
            "remove_local_draft" => {
                let playlist_id = self.pending_track_playlist_id();
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.remove_pending_local_track(playlist_id.as_deref(), &track.id, cx);
                });
            }
            "like" => {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.toggle_like(&track.id, cx));
            }
            "add_to_playlist" => {
                state.update(cx, |state, cx| {
                    state.show_add_to_playlist = true;
                    cx.notify();
                });
            }
            "edit_local_track" => {
                state.update(cx, |state, cx| {
                    state.show_edit_local_track = true;
                    cx.notify();
                });
            }
            "remove_from_playlist" => {
                if let TrackContextSource::OwnPlaylist { playlist_id } = &self.context {
                    MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.remove_track_from_playlist(playlist_id, &track.id, None, None, cx);
                    });
                }
            }
            "play_next" => {
                let track = track.clone();
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.play_next(track, cx));
            }
            "add_to_queue" => {
                let track = track.clone();
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.add_to_queue(track, cx));
            }
            "share" => {
                let playlist_id = self.share_playlist_id.clone().or_else(|| match &self.context {
                    TrackContextSource::OwnPlaylist { playlist_id } => Some(playlist_id.clone()),
                    TrackContextSource::External | TrackContextSource::OwnArtistStudio => None,
                });
                let track = track.clone();
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_track(track, playlist_id, cx));
            }
            "save" => {
                let track = track.clone();
                MusicTrackDownloadStore::shared(cx).update(cx, |store, cx| store.present_options(track, cx));
            }
            "go_to_artist" => {
                if !track.artist_id.is_empty() {
                    Self::push(RootRoute::MusicArtistProfile { artist_id: track.artist_id.clone() }, nav_store, cx);
                }
            }
            "go_to_album" => {
                if !track.album_id.is_empty() {
                    Self::push(RootRoute::MusicAlbumDetail { album_id: track.album_id.clone() }, nav_store, cx);
                }
            }
            "delete_track" => {
                let track_id = track.id.clone();
                NativeConfirmAlert::present_delete(
                    &tr("ctx_delete_track"),
                    &tr("music_confirm_delete_track"),
                    Some(tr("ctx_delete_track")),
                    None,
                    false,
                    None,
                    None,
                    move |cx: &mut App| {
                        MusicActionsStore::shared(cx)
                            .update(cx, |store, cx| store.delete_track_action(&track_id, None, None, cx));
                    },
                    cx,
                );
            }
            "delete_local_track" => {
                if let Some(on_delete_local_track) = &self.on_delete_local_track {
                    on_delete_local_track(window, cx);
                }
            }
            "report" => {
                if !track.id.is_empty() {
                    let target = ReportTarget::MusicTrack(track.id.clone());
                    ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, None, None, cx));
                }
            }
            _ => {}
        }
    }

    fn push(route: RootRoute, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn pending_track_playlist_id(&self) -> Option<String> {
        if let TrackContextSource::OwnPlaylist { playlist_id } = &self.context {
            return Some(playlist_id.clone());
        }
        self.share_playlist_id.clone()
    }

    fn resolved_share_playlist_id(&self, cx: &mut App) -> Option<String> {
        if let Some(share_playlist_id) = &self.share_playlist_id {
            return Some(share_playlist_id.clone());
        }
        if let TrackContextSource::OwnPlaylist { playlist_id } = &self.context {
            return Some(playlist_id.clone());
        }
        let player = MusicPlayerState::shared(cx);
        let player = player.read(cx);
        if player.current_track.as_ref().is_none_or(|current| current.id != self.track.id) {
            return None;
        }
        player.listening_playlist_id().map(str::to_owned)
    }

    fn add_to_playlist_sheet(&self, state: &Entity<TrackContextMenuRowState>, background: Hsla) -> BottomSheetUi {
        let binding = Binding::entity(
            state.clone(),
            |state: &TrackContextMenuRowState| state.show_add_to_playlist,
            |state, value, cx| {
                state.show_add_to_playlist = value;
                cx.notify();
            },
        );
        let track = self.track.clone();
        let content_id = self.child_id("add-to-playlist-content");
        let dismissed = state.clone();
        BottomSheetUi::new(
            self.child_id("add-to-playlist"),
            binding,
            move |_, _| {
                let dismissed = dismissed.clone();
                AddToPlaylistSheet::new(content_id.clone(), track.clone()).on_dismiss(move |_, cx| {
                    dismissed.update(cx, |state, cx| {
                        state.show_add_to_playlist = false;
                        cx.notify();
                    });
                })
            },
            |_, _| Empty,
        )
        .snap_fractions(vec![0.55, 0.88])
        .footer_shows_top_border(false)
        .fits_content(true)
        .sheet_background(Some(background))
    }

    fn edit_local_track_sheet(&self, state: &Entity<TrackContextMenuRowState>, background: Hsla) -> BottomSheetUi {
        let binding = Binding::entity(
            state.clone(),
            |state: &TrackContextMenuRowState| state.show_edit_local_track,
            |state, value, cx| {
                state.show_edit_local_track = value;
                cx.notify();
            },
        );
        let track = self.track.clone();
        let content_id = self.child_id("edit-local-track-content");
        let dismissed = state.clone();
        BottomSheetUi::new(
            self.child_id("edit-local-track"),
            binding,
            move |window, cx| {
                let track = track.clone();
                let dismissed = dismissed.clone();
                window.use_keyed_state(content_id.clone(), cx, move |_, cx| {
                    EditLocalTrackSheet::new(
                        track,
                        move |cx: &mut App| {
                            dismissed.update(cx, |state, cx| {
                                state.show_edit_local_track = false;
                                cx.notify();
                            });
                        },
                        cx,
                    )
                })
            },
            |_, _| Empty,
        )
        .snap_fractions(vec![0.62, 0.86])
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .footer_shows_top_border(false)
        .fits_content(true)
        .sheet_background(Some(background))
    }
}

impl RenderOnce for TrackContextMenuRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let player = MusicPlayerState::shared(cx);
        let download_store = MusicTrackDownloadStore::shared(cx);
        let track_id = self.track.id.clone();
        let state = window.use_keyed_state(self.child_id("state"), cx, move |_, cx| {
            let (current_track_id, listening_playlist_id) = {
                let player = player.read(cx);
                (
                    player.current_track.as_ref().map(|track| track.id.clone()),
                    player.listening_playlist_id().map(str::to_owned),
                )
            };
            let can_start_download = download_store.read(cx).can_start_download(&track_id);
            TrackContextMenuRowState {
                show_add_to_playlist: false,
                show_edit_local_track: false,
                current_track_id,
                listening_playlist_id,
                can_start_download,
                _player_observation: cx.observe(&player, |state: &mut TrackContextMenuRowState, player, cx| {
                    let (current_track_id, listening_playlist_id) = {
                        let player = player.read(cx);
                        (
                            player.current_track.as_ref().map(|track| track.id.clone()),
                            player.listening_playlist_id().map(str::to_owned),
                        )
                    };
                    if state.current_track_id != current_track_id || state.listening_playlist_id != listening_playlist_id {
                        state.current_track_id = current_track_id;
                        state.listening_playlist_id = listening_playlist_id;
                        cx.notify();
                    }
                }),
                _download_observation: cx.observe(&download_store, move |state, store, cx| {
                    let can_start_download = store.read(cx).can_start_download(&track_id);
                    if state.can_start_download != can_start_download {
                        state.can_start_download = can_start_download;
                        cx.notify();
                    }
                }),
            }
        });

        let nav_store = env::<NavigationStoreKey>(cx);
        let actions = self.menu_actions(cx);
        let add_to_playlist_sheet = self.add_to_playlist_sheet(&state, theme.bg._100);
        let edit_local_track_sheet = self.edit_local_track_sheet(&state, theme.bg._100);

        let row = Rc::new(self);
        let action_row = row.clone();
        let tap_row = row.clone();
        let content_row = row.clone();

        let container = HoldContextMenuContainer::new(
            row.child_id("hold"),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                TrackRow::new(content_row.child_id("row"), content_row.track.clone())
                    .is_playing(content_row.is_playing)
                    .shows_like_button(content_row.shows_like_button)
                    .guide_anchor(content_row.guide_anchor)
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .lifted_background(Some(theme.bg._100))
        .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
            action_row.handle_action(&id, &state, nav_store.as_ref(), window, cx);
        }))
        .on_tap(Rc::new(move |point: Point<Pixels>, window: &mut Window, cx: &mut App| {
            if BubbleTapRouter::dispatch(point, None, window, cx) {
                return;
            }
            (tap_row.on_tap)(window, cx);
        }));

        div().w_full().child(container).child(add_to_playlist_sheet).child(edit_local_track_sheet)
    }
}
