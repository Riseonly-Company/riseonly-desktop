use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, BoxShadow, Context, ElementId, Empty, Entity, FontWeight, ListState, RenderOnce, SharedString, Subscription,
    Task, Window, black, list, point, white,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, format_printf, tr, tr_plural};
use rise_platform::haptics::selection_changed;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, ThemeStore, color};
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::download_selection_badge::DownloadSelectionBadge;
use rise_ui::expandable_text_ui::{EdgeInsets, ExpandableTextUi};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::{PageHeaderIconButton, PageHeaderToolbarSlot};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::{ListScrollbar, scroll_view};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::UnitPoint;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::push_notifications::universal_link_parser::RiseonlyUniversalLink;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, SettingsWrapperUi, list_shell_floating_top_inset,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::pages::chats::chats_reorder_toggle_button::ChatsReorderToggleButton;
use crate::modules::media_picker::export::media_picker_export_progress_overlay::MediaPickerExportProgressOverlay;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::music::pages::playlist_detail::music_playlist_listener_cluster::{
    MusicPlaylistListenerCluster, MusicPlaylistLiveScopeIdKey,
};
use crate::modules::music::pages::playlist_detail::music_playlist_listeners_screen::MusicPlaylistListenersScreen;
use crate::modules::music::pages::playlist_detail::playlist_reorder_list::PlaylistReorderList;
use crate::modules::music::pages::track::track_context_menu_row::{TrackContextMenuRow, TrackContextSource};
use crate::modules::music::pages::track::track_row::TrackRow;
use crate::modules::music::shared::utils::music::music_models::{PlaylistModel, TrackModel};
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::shared::utils::music::music_track_batch_export::MusicTrackBatchExport;
use crate::modules::music::shared::utils::music::music_track_file_export::MusicTrackSavePolicy;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::{
    MusicPlaylistListener, MusicPlaylistListeningSession, MusicPlaylistSettings,
};
use crate::modules::music::stores::music_actions::music_wire_types::{
    ProtoTrackRecordExt, RiseStoredMusicPlaylist, WireAlbumDTOExt,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::playlists::add_local_track_sheet::AddLocalTrackSheet;
use crate::modules::music::widgets::bottomsheets::playlists::edit_playlist_sheet::EditPlaylistSheet;
use crate::modules::music::widgets::bottomsheets::playlists::playlist_visibility::PlaylistVisibility;
use crate::modules::music::widgets::bottomsheets::track_picker::track_picker_shared::TrackPickerListModel;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::music::widgets::music_mini_player_bottom_padding::{
    MusicMiniPlayerBottomPaddingModifiers, hear_player_through_observation_while_covered,
    observe_player_unless_covered,
};
use crate::modules::music::widgets::music_scope_playback_button::MusicScopePlaybackButton;
use crate::modules::music::widgets::music_skeletons::{PlaylistHeroSkeleton, TrackListSkeleton};
use crate::modules::music::widgets::music_track_cover_image::{MusicCoverImage, MusicTrackCoverImageModifiers};

const LIKED_SONGS_ID: &str = "liked_songs";
const LIKED_COLORS: [&str; 2] = ["#E84393", "#FD79A8"];
const COVER_SIDE: f32 = 180.0;
const COVER_RADIUS: f32 = 16.0;
const PLAYBACK_BUTTON_SIZE: f32 = 56.0;
const LISTENER_CLUSTER_SIZE: f32 = 42.0;
const PAGINATION_THRESHOLD: usize = 8;
const EDIT_SHEET_FRACTION: f32 = 0.72;
const ADD_LOCAL_TRACK_SHEET_FRACTION: f32 = 0.68;
const ACTION_BUTTON_MAX_WIDTH: f32 = 220.0;
const LISTENERS_ROW_MAX_WIDTH: f32 = 393.0;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// state of the page and the observed stores
/// every store a computed property of the page reads is observed here
struct PlaylistDetailState {
    show_add_local_track_sheet: bool,
    show_edit_sheet: bool,
    is_playlist_menu_presented: bool,
    show_live_listeners: bool,
    is_selecting_tracks: bool,
    is_reordering_tracks: bool,
    selected_track_ids: HashSet<String>,
    list_model: TrackPickerListModel,
    _subscriptions: [Subscription; 6],
}

impl PlaylistDetailState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let player = MusicPlayerState::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let batch_export = MusicTrackBatchExport::shared(cx);
        Self {
            show_add_local_track_sheet: false,
            show_edit_sheet: false,
            is_playlist_menu_presented: false,
            show_live_listeners: false,
            is_selecting_tracks: false,
            is_reordering_tracks: false,
            selected_track_ids: HashSet::new(),
            list_model: TrackPickerListModel::new(cx),
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
                observe_player_unless_covered(&player, cx),
                cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
                cx.observe(&batch_export, |_, _, cx| cx.notify()),
            ],
        }
    }
}

/// the info tuple of the page
struct PlaylistDetailInfo {
    title: String,
    subtitle: String,
    colors: Vec<String>,
    image_url: String,
    tracks: Rc<Vec<TrackModel>>,
}

/// the visible playlist listeners tuple of the page
struct VisiblePlaylistListeners {
    listeners: Vec<MusicPlaylistListener>,
    total_count: i64,
    is_loading: bool,
}

#[derive(IntoElement)]
pub struct PlaylistDetail {
    id: ElementId,
    collection_id: String,
    is_album: bool,
}

impl PlaylistDetail {
    pub fn new(collection_id: impl Into<String>) -> Self {
        let collection_id = collection_id.into();
        Self { id: ElementId::Name(SharedString::from(format!("playlist-detail-{collection_id}"))), collection_id, is_album: false }
    }

    pub fn is_album(mut self, is_album: bool) -> Self {
        self.is_album = is_album;
        self
    }

    fn is_own_playlist(&self, playlist_dto: Option<&RiseStoredMusicPlaylist>, current_user_id: Option<&str>) -> bool {
        if self.is_album || self.collection_id == LIKED_SONGS_ID {
            return false;
        }
        playlist_dto.and_then(|playlist| playlist.owner_user_id.as_deref()) == current_user_id
    }

    fn reportable_playlist_owner_id(
        &self,
        playlist_dto: Option<&RiseStoredMusicPlaylist>,
        current_user_id: Option<&str>,
    ) -> Option<String> {
        if self.is_album || self.collection_id == LIKED_SONGS_ID {
            return None;
        }
        let owner = playlist_dto.and_then(|playlist| playlist.owner_user_id.clone())?;
        if owner.is_empty() || Some(owner.as_str()) == current_user_id {
            return None;
        }
        Some(owner)
    }

    /// the playlists own order may be changed by whoever may
    /// remove its tracks, same right the server checks
    /// albums and the liked list keep theirs
    fn can_reorder_tracks(&self, track_count: usize, can_remove_local_tracks: bool) -> bool {
        if self.is_album || self.collection_id == LIKED_SONGS_ID || track_count <= 1 {
            return false;
        }
        can_remove_local_tracks
    }

    fn is_collaborative_playlist(&self, playlist_dto: Option<&RiseStoredMusicPlaylist>) -> bool {
        !self.is_album
            && self.collection_id != LIKED_SONGS_ID
            && playlist_dto.and_then(|playlist| playlist.visibility.as_deref())
                == Some(PlaylistVisibility::Local.raw_value())
    }

    fn visible_playlist_listeners(&self, current_user_id: Option<&str>, cx: &mut App) -> VisiblePlaylistListeners {
        let snapshot = MusicServicesStore::shared(cx)
            .update(cx, |service, cx| service.playlist_listening_snapshot(&self.collection_id, None, cx));
        let is_snapshot_missing = snapshot.is_none();
        let reported_total = snapshot.as_ref().map(|snapshot| snapshot.total_listener_count);
        let all_listeners = snapshot.map(|snapshot| snapshot.listeners).unwrap_or_default();
        let contains_current_user =
            current_user_id.is_some_and(|user_id| all_listeners.iter().any(|listener| listener.user_id == user_id));
        let reported_total = reported_total.unwrap_or(all_listeners.len() as i64);
        let listeners: Vec<MusicPlaylistListener> =
            all_listeners.into_iter().filter(|listener| Some(listener.user_id.as_str()) != current_user_id).collect();
        let total_count = (listeners.len() as i64).max(reported_total - i64::from(contains_current_user));
        let is_pending = MusicActionsStore::slices(cx).get_playlist_listening_snapshot_sai.is_pending();

        VisiblePlaylistListeners {
            listeners,
            total_count: total_count.max(0),
            is_loading: is_snapshot_missing && is_pending,
        }
    }

    fn info(&self, playlist_dto: Option<&RiseStoredMusicPlaylist>, cx: &mut App) -> Option<PlaylistDetailInfo> {
        if self.collection_id == LIKED_SONGS_ID {
            let total = MusicServicesStore::shared(cx).update(cx, |service, cx| service.liked_tracks_total_count(cx));
            return Some(PlaylistDetailInfo {
                title: tr("music_liked_songs"),
                subtitle: tr_plural("music_track_count", total),
                colors: LIKED_COLORS.iter().map(|hex| (*hex).to_owned()).collect(),
                image_url: String::new(),
                tracks: Rc::new(Self::liked_tracks_from_cache(cx)),
            });
        }
        if self.is_album {
            let album = MusicActionsStore::slices(cx).get_album_sai.data()?.album?.as_album_model()?;
            if album.id != self.collection_id {
                return None;
            }
            return Some(PlaylistDetailInfo {
                title: album.title,
                subtitle: format!("{} \u{2022} {}", album.artist_name, album.year),
                colors: album.cover_colors,
                image_url: album.cover_image_url,
                tracks: Rc::new(album.tracks),
            });
        }
        let playlist = playlist_dto?.as_playlist_model()?;
        if playlist.id != self.collection_id {
            return None;
        }
        Some(PlaylistDetailInfo {
            title: playlist.title,
            subtitle: playlist.subtitle,
            colors: playlist.cover_colors,
            image_url: playlist.cover_image_url,
            tracks: Rc::new(playlist.tracks),
        })
    }

    fn is_loading(&self, has_info: bool, cx: &mut App) -> bool {
        if has_info {
            return false;
        }
        if self.collection_id == LIKED_SONGS_ID {
            let is_pending = MusicActionsStore::slices(cx).get_liked_sai.is_pending();
            return is_pending && Self::liked_tracks_from_cache(cx).is_empty();
        }
        let actions = MusicActionsStore::shared(cx);
        let store = actions.read(cx);
        if self.is_album {
            return store.get_album_sai.is_pending();
        }
        store.get_playlist_sai.is_pending()
    }

    fn liked_tracks_from_cache(cx: &mut App) -> Vec<TrackModel> {
        MusicActionsStore::slices(cx)
            .get_liked_sai
            .data()
            .map(|data| data.tracks)
            .unwrap_or_default()
            .iter()
            .filter_map(ProtoTrackRecordExt::as_track_model)
            .collect()
    }

    fn playback_scope_id(&self) -> String {
        if self.collection_id == LIKED_SONGS_ID {
            return LIKED_SONGS_ID.to_owned();
        }
        if self.is_album {
            return format!("album:{}", self.collection_id);
        }
        format!("playlist:{}", self.collection_id)
    }

    fn start_listening_observation_if_collaborative(collection_id: &str, is_collaborative: bool, cx: &mut App) {
        if !is_collaborative {
            return;
        }
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.load_playlist_settings(collection_id, false, cx);
            store.start_playlist_listening_observation(collection_id, None, cx);
        });
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.refresh_playlist_listening_presence(cx));
    }

    fn load_on_appear(collection_id: &str, is_album: bool, cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            if collection_id == LIKED_SONGS_ID {
                store.load_liked_if_needed(cx);
            } else if is_album {
                store.open_album(collection_id, cx);
            } else {
                store.open_playlist(collection_id, cx);
            }
        });
    }

    fn refresh(collection_id: &str, is_album: bool, cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            if collection_id == LIKED_SONGS_ID {
                store.load_liked_if_needed(cx);
            } else if is_album {
                store.open_album(collection_id, cx);
            } else {
                store.open_playlist(collection_id, cx);
            }
        });
    }
}

impl RenderOnce for PlaylistDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| PlaylistDetailState::new(cx));
        hear_player_through_observation_while_covered(&MusicPlayerState::shared(cx), cx);

        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let services = MusicServicesStore::shared(cx);
        let playlist_dto = services.update(cx, |service, cx| service.playlist(&self.collection_id, cx));
        let is_own_playlist = self.is_own_playlist(playlist_dto.as_ref(), current_user_id.as_deref());
        let is_collaborative_playlist = self.is_collaborative_playlist(playlist_dto.as_ref());
        let is_own_local_playlist = is_own_playlist && is_collaborative_playlist;
        let playlist_settings: Option<MusicPlaylistSettings> =
            services.update(cx, |service, cx| service.playlist_settings(&self.collection_id, cx));
        let pending_join_request_count = playlist_settings.as_ref().map_or(0, |settings| settings.pending_request_count);
        let collaboration_settings = playlist_settings.filter(|_| is_collaborative_playlist);
        let can_add_local_tracks = is_collaborative_playlist
            && (is_own_playlist
                || collaboration_settings.as_ref().is_some_and(|settings| settings.viewer_permissions.can_add_tracks));
        let local_tracks_visible_to_members = is_collaborative_playlist
            && (!is_own_playlist
                || collaboration_settings.as_ref().map_or(0, |settings| settings.active_participant_count) > 1);
        let can_remove_local_tracks = is_own_playlist
            || collaboration_settings.as_ref().is_some_and(|settings| settings.viewer_permissions.can_remove_tracks);
        let active_listening_session: Option<MusicPlaylistListeningSession> = if is_collaborative_playlist {
            services.update(cx, |service, cx| service.active_playlist_listening_session(&self.collection_id, cx))
        } else {
            None
        };
        let visible_listeners = self.visible_playlist_listeners(current_user_id.as_deref(), cx);
        let info = self.info(playlist_dto.as_ref(), cx);
        let is_loading = self.is_loading(info.is_some(), cx);
        let hold_is_active = HoldContextMenuPresenter::has_active_request(cx);
        let reportable_owner_id = self.reportable_playlist_owner_id(playlist_dto.as_ref(), current_user_id.as_deref());
        let can_reorder_tracks =
            self.can_reorder_tracks(info.as_ref().map_or(0, |info| info.tracks.len()), can_remove_local_tracks);
        let savable_tracks: Rc<Vec<TrackModel>> = Rc::new(
            info.as_ref()
                .map(|info| info.tracks.iter().filter(|track| MusicTrackSavePolicy::can_save(track)).cloned().collect())
                .unwrap_or_default(),
        );
        let (is_selecting_tracks, is_reordering_tracks) = {
            let state = state.read(cx);
            (state.is_selecting_tracks, state.is_reordering_tracks)
        };

        let collection_id = self.collection_id.clone();
        let is_album = self.is_album;

        on_change(named_child(&self.id, "own-local"), is_own_local_playlist, true, window, cx, |_, own, _, cx| {
            GuideFacts::set(RiseGuideSignal::LocalPlaylistOpened, *own, cx);
        });
        let reorder_state = state.clone();
        on_change(named_child(&self.id, "can-reorder"), can_reorder_tracks, false, window, cx, move |_, can, _, cx| {
            if !*can {
                reorder_state.update(cx, |state, cx| {
                    state.is_reordering_tracks = false;
                    cx.notify();
                });
            }
        });
        let observed_id = collection_id.clone();
        on_change(
            named_child(&self.id, "collaborative"),
            is_collaborative_playlist,
            false,
            window,
            cx,
            move |_, is_collaborative, _, cx| {
                Self::start_listening_observation_if_collaborative(&observed_id, *is_collaborative, cx);
            },
        );
        let session_id = collection_id.clone();
        let session_user_id = current_user_id.clone();
        on_change(
            named_child(&self.id, "active-session"),
            active_listening_session.clone(),
            true,
            window,
            cx,
            move |_, session, _, cx| {
                let Some(session) = session.clone() else {
                    return;
                };
                let viewer = session_user_id.as_deref();
                if Some(session.host_user_id.as_str()) != viewer
                    && !session.participants.iter().any(|participant| Some(participant.user_id.as_str()) == viewer)
                {
                    return;
                }
                let server_time_ms = playlist_listening_server_time_ms(&session_id, cx);
                MusicPlayerState::shared(cx).update(cx, |player, cx| {
                    player.attach_playlist_listening_session(session, server_time_ms, None, cx);
                });
            },
        );

        let parts = PlaylistDetailParts {
            id: self.id.clone(),
            collection_id: collection_id.clone(),
            is_album,
            playback_scope_id: self.playback_scope_id(),
            state: state.clone(),
            is_collaborative_playlist,
            can_add_local_tracks,
            can_remove_local_tracks,
            is_selecting_tracks,
            visible_listeners,
            top_inset: 0.0,
        };

        let right_jsx = header_right_actions(HeaderRightActions {
            id: self.id.clone(),
            collection_id: collection_id.clone(),
            state: state.clone(),
            nav_store: nav_store.clone(),
            theme: theme.clone(),
            shareable_playlist: playlist_dto.as_ref().and_then(RiseStoredMusicPlaylist::as_playlist_model),
            can_add_local_tracks,
            is_own_playlist,
            is_collaborative_playlist,
            pending_join_request_count,
            reportable_owner_id,
            playlist_title: playlist_dto.as_ref().and_then(|playlist| playlist.title.clone()).unwrap_or_default(),
            savable_tracks: savable_tracks.clone(),
            can_reorder_tracks,
            is_reordering_tracks,
        });

        let title = info.as_ref().map_or_else(String::new, |info| info.title.clone());
        let back_store = nav_store.clone();
        let mut wrapper = SettingsWrapperUi::new(title, move |_, cx| {
            if let Some(nav_store) = back_store.as_ref() {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        });
        if let Some(right_jsx) = right_jsx {
            wrapper = wrapper.right_jsx(right_jsx);
        }

        let body: AnyElement = if is_reordering_tracks && let Some(info) = info.as_ref() {
            PlaylistReorderList::new(collection_id.clone(), (*info.tracks).clone()).into_any_element()
        } else if let Some(info) = info {
            PlaylistDetailContent { parts, info, is_loading, hold_is_active }.into_any_element()
        } else if is_loading {
            div()
                .size_full()
                .bg(theme.bg._100)
                .child(
                    scroll_view(named_child(&self.id, "skeleton"))
                        .scroll_disabled(true)
                        .shows_indicators(false)
                        .list_shell_scroll_under_chrome(cx)
                        .child(PlaylistHeroSkeleton::new()),
                )
                .into_any_element()
        } else {
            let (retry_id, pull_id) = (collection_id.clone(), collection_id.clone());
            div()
                .size_full()
                .bg(theme.bg._100)
                .child(
                    scroll_view(named_child(&self.id, "failed"))
                        .shows_indicators(false)
                        .list_shell_scroll_under_chrome(cx)
                        .refreshable(move |_, cx| {
                            Self::refresh(&pull_id, is_album, cx);
                            Task::ready(())
                        })
                        .child(
                            vstack(None::<f32>)
                                .w_full()
                                .child(
                                    div()
                                        .w_full()
                                        .pt(px(60.0))
                                        .child(SecondaryText::new(tr("music_load_failed")).px(15.0).tac(TextAlignT::Center)),
                                )
                                .child(
                                    div().w_full().max_w(px(ACTION_BUTTON_MAX_WIDTH)).pt(px(12.0)).child(
                                        ButtonUi::new(named_child(&self.id, "retry"))
                                            .on_press(move |_, cx| Self::refresh(&retry_id, is_album, cx))
                                            .child(MainText::new(tr("music_retry"))),
                                    ),
                                ),
                        ),
                )
                .into_any_element()
        };

        let keys_id = collection_id.clone();
        let content = PageRefreshKeys::new(
            named_child(&self.id, "refresh-keys"),
            move |_, cx| Self::refresh(&keys_id, is_album, cx),
            body,
        );

        let session_state = state.clone();
        let listeners = MusicPlaylistListenersScreen::new(
            named_child(&self.id, "listeners"),
            Binding::entity(
                state.clone(),
                |state: &PlaylistDetailState| state.show_live_listeners,
                |state, value, _| state.show_live_listeners = value,
            ),
            collection_id.clone(),
        )
        .owns_observation(false)
        .on_open_session(move |cx| open_listening_session(&session_state, cx));

        let edit_sheet = edit_sheet(&self.id, &collection_id, &state, &theme);
        let add_local_track_sheet =
            add_local_track_sheet(&self.id, &collection_id, &state, local_tracks_visible_to_members, &theme);

        let selection_bar =
            is_selecting_tracks.then(|| track_selection_bar(&self.id, &state, &savable_tracks, &theme, cx));
        let export_progress = {
            let export = MusicTrackBatchExport::shared(cx);
            let export = export.read(cx);
            export.is_running().then(|| export.progress.clone())
        };
        let export_overlay = export_progress.map(|progress| {
            div().absolute().inset_0().child(MediaPickerExportProgressOverlay::new(progress, ThemeStore::shared(cx)))
        });

        let appear_id = collection_id.clone();
        let disappear_id = collection_id;
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().w_full().flex_1().min_h_0().child(wrapper.content(content)))
                    .children(selection_bar),
            )
            .children(export_overlay)
            .child(listeners)
            .child(edit_sheet)
            .child(add_local_track_sheet)
            .on_appear(named_child(&self.id, "appear"), move |_, cx| {
                Self::load_on_appear(&appear_id, is_album, cx);
                GuideFacts::set(RiseGuideSignal::LocalPlaylistOpened, is_own_local_playlist, cx);
                Self::start_listening_observation_if_collaborative(&appear_id, is_collaborative_playlist, cx);
            })
            .on_disappear(named_child(&self.id, "disappear"), move |_, cx| {
                GuideFacts::set(RiseGuideSignal::LocalPlaylistOpened, false, cx);
                if !is_collaborative_playlist {
                    return;
                }
                let player = MusicPlayerState::shared(cx);
                if player.read(cx).listening_playlist_id() == Some(disappear_id.as_str()) {
                    return;
                }
                MusicInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.stop_playlist_listening_observation(&disappear_id, None, cx));
            })
    }
}

fn playlist_listening_server_time_ms(collection_id: &str, cx: &mut App) -> i64 {
    MusicServicesStore::shared(cx)
        .update(cx, |service, cx| service.playlist_listening_snapshot(collection_id, None, cx))
        .map_or_else(|| RiseClock::now().ms(), |snapshot| snapshot.server_time_ms)
}

fn open_listening_session(state: &Entity<PlaylistDetailState>, cx: &mut App) {
    state.update(cx, |state, cx| {
        state.show_live_listeners = false;
        cx.notify();
    });
    MusicPlayerState::shared(cx).update(cx, |player, cx| {
        player.show_listening_session_sheet = true;
        cx.notify();
    });
}

fn edit_sheet(id: &ElementId, collection_id: &str, state: &Entity<PlaylistDetailState>, theme: &AppTheme) -> BottomSheetUi {
    let binding = Binding::entity(
        state.clone(),
        |state: &PlaylistDetailState| state.show_edit_sheet,
        |state, value, _| state.show_edit_sheet = value,
    );
    let content_id = named_child(id, "edit-content");
    let collection_id = collection_id.to_owned();
    let dismissed = state.clone();
    BottomSheetUi::new(
        named_child(id, "edit-sheet"),
        binding,
        move |window, cx| {
            let dto = MusicServicesStore::shared(cx).update(cx, |service, cx| service.playlist(&collection_id, cx));
            let Some(dto) = dto else {
                return div().into_any_element();
            };
            let dismissed = dismissed.clone();
            let on_dismiss: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
                dismissed.update(cx, |state, cx| {
                    state.show_edit_sheet = false;
                    cx.notify();
                });
            });
            window
                .use_keyed_state(content_id.clone(), cx, move |_, cx| EditPlaylistSheet::new(dto, Some(on_dismiss), cx))
                .into_any_element()
        },
        |_, _| Empty,
    )
    .snap_fractions(vec![EDIT_SHEET_FRACTION])
    .dismisses_keyboard_before_backdrop_dismiss(true)
    .footer_shows_top_border(false)
    .fits_content(true)
    .sheet_background(Some(theme.bg._100))
}

fn add_local_track_sheet(
    id: &ElementId,
    collection_id: &str,
    state: &Entity<PlaylistDetailState>,
    is_visible_to_members: bool,
    theme: &AppTheme,
) -> BottomSheetUi {
    let binding = Binding::entity(
        state.clone(),
        |state: &PlaylistDetailState| state.show_add_local_track_sheet,
        |state, value, _| state.show_add_local_track_sheet = value,
    );
    let content_id = named_child(id, "add-local-track-content");
    let collection_id = collection_id.to_owned();
    let dismissed = state.clone();
    BottomSheetUi::new(
        named_child(id, "add-local-track-sheet"),
        binding,
        move |_, _| {
            let dismissed = dismissed.clone();
            AddLocalTrackSheet::new(content_id.clone(), collection_id.clone())
                .is_visible_to_members(is_visible_to_members)
                .on_dismiss(move |cx| {
                    dismissed.update(cx, |state, cx| {
                        state.show_add_local_track_sheet = false;
                        cx.notify();
                    });
                })
        },
        |_, _| Empty,
    )
    .snap_fractions(vec![ADD_LOCAL_TRACK_SHEET_FRACTION])
    .dismisses_keyboard_before_backdrop_dismiss(true)
    .footer_shows_top_border(false)
    .fits_content(true)
    .sheet_background(Some(theme.bg._100))
}

fn push_route(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

struct HeaderRightActions {
    id: ElementId,
    collection_id: String,
    state: Entity<PlaylistDetailState>,
    nav_store: Option<Entity<NavigationStore>>,
    theme: AppTheme,
    shareable_playlist: Option<PlaylistModel>,
    can_add_local_tracks: bool,
    is_own_playlist: bool,
    is_collaborative_playlist: bool,
    pending_join_request_count: i64,
    reportable_owner_id: Option<String>,
    playlist_title: String,
    savable_tracks: Rc<Vec<TrackModel>>,
    can_reorder_tracks: bool,
    is_reordering_tracks: bool,
}

fn header_right_actions(actions: HeaderRightActions) -> Option<AnyElement> {
    let can_share = actions
        .shareable_playlist
        .as_ref()
        .is_some_and(|playlist| RiseonlyUniversalLink::playlist(&playlist.id, AppEnvironment::current()).is_some());
    let can_report = actions.reportable_owner_id.is_some();
    let can_save_tracks = !actions.savable_tracks.is_empty();
    if actions.is_reordering_tracks {
        let state = actions.state.clone();
        return Some(
            div()
                .id(named_child(&actions.id, "reorder-done-hint"))
                .tooltip_label(tr("done"))
                .child(ChatsReorderToggleButton::new(named_child(&actions.id, "reorder-done"), true, move |_, cx| {
                    let _ = selection_changed();
                    state.update(cx, |state, cx| {
                        state.is_reordering_tracks = false;
                        cx.notify();
                    });
                }))
                .into_any_element(),
        );
    }
    let shows_menu =
        can_share || actions.is_own_playlist || actions.is_collaborative_playlist || can_report || can_save_tracks;
    if !actions.can_add_local_tracks && !shows_menu {
        return None;
    }

    let mut row = hstack(8.0);
    if actions.can_add_local_tracks {
        let state = actions.state.clone();
        row = row.child(
            div()
                .id(named_child(&actions.id, "add-local-track-hint"))
                .tooltip_label(tr("music_local_upload_add"))
                .child(
                    PageHeaderIconButton::new(named_child(&actions.id, "add-local-track"), "plus", move |_, cx| {
                        state.update(cx, |state, cx| {
                            state.show_add_local_track_sheet = true;
                            cx.notify();
                        });
                    })
                    .font_size(17.0),
                )
                .guide_anchor(RiseGuideAnchor::PlaylistAddLocalTrack),
        );
    }
    if shows_menu {
        row = row.child(header_menu(&actions, can_share, can_report));
    }
    Some(row.into_any_element())
}

fn playlist_menu_glyph(theme: &AppTheme) -> AnyElement {
    PageHeaderToolbarSlot::new(div().children(IconUi::sized("ellipsis", px(16.0), theme.text.primary, false)))
        .into_any_element()
}

fn playlist_menu_actions(actions: &HeaderRightActions, can_share: bool, can_report: bool) -> Vec<HoldContextMenuAction> {
    let mut items = Vec::new();
    if can_share && actions.shareable_playlist.is_some() {
        items.push(
            HoldContextMenuAction::new("share", tr("music_playlist_share")).system_image("square.and.arrow.up"),
        );
    }
    if actions.is_own_playlist {
        items.push(HoldContextMenuAction::new("edit", tr("music_edit_playlist")).system_image("pencil"));
    }
    if actions.is_own_playlist || actions.is_collaborative_playlist {
        items.push(HoldContextMenuAction::new("settings", tr("music_playlist_settings_title")).system_image("gearshape"));
    }
    if actions.can_reorder_tracks {
        items.push(HoldContextMenuAction::new("reorder", tr("ctx_reorder")).system_image("arrow.up.arrow.down"));
    }
    let savable_count = actions.savable_tracks.len();
    if savable_count > 0 {
        items.push(
            HoldContextMenuAction::new(
                "save_all",
                format_printf(&tr("music_save_all"), &[FormatArg::Int(savable_count as i64)]),
            )
            .system_image("square.and.arrow.down")
            .section_break(true),
        );
        items.push(HoldContextMenuAction::new("select_tracks", tr("music_select_tracks")).system_image("checkmark.circle"));
    }
    if actions.is_own_playlist {
        items.push(
            HoldContextMenuAction::new("delete", tr("music_delete_playlist"))
                .system_image("trash")
                .destructive(true)
                .section_break(savable_count > 0),
        );
    }
    if can_report {
        items.push(
            HoldContextMenuAction::new("report", tr("ctx_report")).system_image("exclamationmark.bubble").destructive(true),
        );
    }
    items
}

fn confirm_delete_playlist(nav_store: Option<Entity<NavigationStore>>, playlist_id: String, cx: &mut App) {
    NativeConfirmAlert::present_delete(
        &tr("music_delete_playlist"),
        &tr("music_confirm_delete_playlist"),
        Some(tr("music_delete_playlist")),
        None,
        false,
        None,
        None,
        move |cx| {
            let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
                if let Some(nav_store) = nav_store.as_ref() {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            });
            MusicInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.delete_playlist(&playlist_id, Some(on_success), None, cx));
        },
        cx,
    );
}

fn header_menu(actions: &HeaderRightActions, can_share: bool, can_report: bool) -> AnyElement {
    let items = playlist_menu_actions(actions, can_share, can_report);
    let menu_actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| items.clone());

    let state = actions.state.clone();
    let nav_store = actions.nav_store.clone();
    let playlist_id = actions.collection_id.clone();
    let shareable_playlist = actions.shareable_playlist.clone();
    let savable_tracks = actions.savable_tracks.clone();
    let report_owner = actions.reportable_owner_id.clone();
    let report_note = format!("playlist {} {}", actions.collection_id, actions.playlist_title);
    let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
        match id.as_str() {
            "share" => {
                if let Some(playlist) = shareable_playlist.clone() {
                    ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_playlist(playlist, cx));
                }
            }
            "edit" => state.update(cx, |state, cx| {
                state.show_edit_sheet = true;
                cx.notify();
            }),
            "settings" => {
                push_route(&nav_store, RootRoute::MusicPlaylistSettings { playlist_id: playlist_id.clone() }, cx);
            }
            "save_all" => {
                let tracks = savable_tracks.clone();
                MusicTrackBatchExport::shared(cx).update(cx, |export, cx| export.save(&tracks, cx));
            }
            "select_tracks" => state.update(cx, |state, cx| {
                state.selected_track_ids.clear();
                state.is_selecting_tracks = true;
                cx.notify();
            }),
            "reorder" => state.update(cx, |state, cx| {
                state.is_selecting_tracks = false;
                state.selected_track_ids.clear();
                state.is_reordering_tracks = true;
                cx.notify();
            }),
            "delete" => confirm_delete_playlist(nav_store.clone(), playlist_id.clone(), cx),
            "report" => {
                if let Some(owner) = report_owner.clone() {
                    let target = ReportTarget::User(owner);
                    let note = report_note.clone();
                    ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, None, Some(note), cx));
                }
            }
            _ => {}
        }
    });

    let is_presented = Binding::entity(
        actions.state.clone(),
        |state: &PlaylistDetailState| state.is_playlist_menu_presented,
        |state: &mut PlaylistDetailState, value: bool, cx| {
            state.is_playlist_menu_presented = value;
            cx.notify();
        },
    );
    let source_theme = actions.theme.clone();
    let source: HoldContextMenuTapSource =
        Rc::new(move |_: &mut Window, _: &mut App| playlist_menu_glyph(&source_theme));
    let opener = actions.state.clone();
    let button = plain(named_child(&actions.id, "menu-button"))
        .on_press(move |_, cx| {
            opener.update(cx, |state, cx| {
                state.is_playlist_menu_presented = true;
                cx.notify();
            });
        })
        .child(playlist_menu_glyph(&actions.theme))
        .tap_context_menu(
            named_child(&actions.id, "menu"),
            is_presented,
            HoldContextMenuAlignment::Trailing,
            false,
            false,
            false,
            None,
            menu_actions,
            on_action,
            source,
        );
    let menu = div()
        .id(named_child(&actions.id, "menu-hint"))
        .tooltip_label(tr("more"))
        .child(button)
        .guide_anchor(RiseGuideAnchor::PlaylistMenu);

    let count = actions.pending_join_request_count;
    div()
        .relative()
        .child(menu)
        .when(actions.is_collaborative_playlist && count > 0, |slot| {
            slot.child(pending_join_request_badge(count, &actions.theme))
        })
        .into_any_element()
}

fn track_selection_bar(
    id: &ElementId,
    state: &Entity<PlaylistDetailState>,
    savable_tracks: &Rc<Vec<TrackModel>>,
    theme: &AppTheme,
    cx: &App,
) -> AnyElement {
    let picked: Vec<TrackModel> = {
        let state = state.read(cx);
        savable_tracks.iter().filter(|track| state.selected_track_ids.contains(&track.id)).cloned().collect()
    };
    let picked_count = picked.len();
    let (cancel_state, save_state) = (state.clone(), state.clone());
    hstack(10.0)
        .w_full()
        .px(px(16.0))
        .pt(px(10.0))
        .pb(px(10.0))
        .bg(theme.bg._100)
        .child(
            div().flex_1().min_w_0().child(
                ButtonUi::new(named_child(id, "selection-cancel"))
                    .background_color(theme.bg._300)
                    .on_press(move |_, cx| {
                        cancel_state.update(cx, |state, cx| {
                            state.is_selecting_tracks = false;
                            state.selected_track_ids.clear();
                            cx.notify();
                        });
                    })
                    .child(MainText::new(tr("common_cancel")).color(theme.text.primary)),
            ),
        )
        .child(
            div().flex_1().min_w_0().child(
                ButtonUi::new(named_child(id, "selection-save"))
                    .disabled(picked.is_empty())
                    .on_press(move |_, cx| {
                        let picked = picked.clone();
                        MusicTrackBatchExport::shared(cx).update(cx, |export, cx| export.save(&picked, cx));
                        save_state.update(cx, |state, cx| {
                            state.is_selecting_tracks = false;
                            state.selected_track_ids.clear();
                            cx.notify();
                        });
                    })
                    .child(
                        MainText::new(format_printf(
                            &tr("music_save_selected"),
                            &[FormatArg::Int(picked_count as i64)],
                        ))
                        .color(white()),
                    ),
            ),
        )
        .into_any_element()
}

fn pending_join_request_badge(count: i64, theme: &AppTheme) -> impl IntoElement {
    let text = if count > 99 { "99+".to_owned() } else { count.to_string() };
    div()
        .absolute()
        .top(px(-4.0))
        .right(px(-4.0))
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(17.0))
        .min_h(px(17.0))
        .when(count > 9, |badge| badge.px(px(4.0)))
        .bg(UIColor::system_red().resolved_color(theme.appearance))
        .capsule()
        .swiftui_font(Font::caption2().weight(FontWeight::BOLD))
        .text_color(white())
        .child(text)
        .child(centred_stroke(UIColor::system_background().resolved_color(theme.appearance), 2.0, Shape::Capsule))
}

/// one entry of the page scroller
/// gpui lays out a long column only through list so the hero is the first entry and the tracks follow
#[derive(Clone, Copy)]
enum PlaylistDetailRow {
    Hero,
    Skeleton,
    Empty,
    Track(usize),
    Foot,
}

impl PlaylistDetailRow {
    fn rows(track_count: usize, is_loading: bool) -> Vec<PlaylistDetailRow> {
        let mut rows = Vec::with_capacity(track_count + 2);
        rows.push(PlaylistDetailRow::Hero);
        if track_count == 0 && is_loading {
            rows.push(PlaylistDetailRow::Skeleton);
        } else if track_count == 0 {
            rows.push(PlaylistDetailRow::Empty);
        } else {
            rows.extend((0..track_count).map(PlaylistDetailRow::Track));
        }
        rows.push(PlaylistDetailRow::Foot);
        rows
    }
}

/// what a row of the list needs, the list asks for rows from outside the page render
struct PlaylistDetailParts {
    id: ElementId,
    collection_id: String,
    is_album: bool,
    playback_scope_id: String,
    state: Entity<PlaylistDetailState>,
    is_collaborative_playlist: bool,
    can_add_local_tracks: bool,
    can_remove_local_tracks: bool,
    is_selecting_tracks: bool,
    visible_listeners: VisiblePlaylistListeners,
    top_inset: f32,
}

impl PlaylistDetailParts {
    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        named_child(&self.id, name)
    }

    fn show_now_playing(cx: &mut App) {
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
    }

    fn row(
        &self,
        row: PlaylistDetailRow,
        info: &PlaylistDetailInfo,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        match row {
            PlaylistDetailRow::Hero => self.hero_section(info, theme),
            PlaylistDetailRow::Skeleton => TrackListSkeleton::new().rows(6).into_any_element(),
            PlaylistDetailRow::Empty => self.empty_tracks(theme),
            PlaylistDetailRow::Track(index) => self.track_row(index, &info.tracks, theme, cx),
            PlaylistDetailRow::Foot => div().w_full().music_mini_player_bottom_padding(40.0, 0.0).into_any_element(),
        }
    }

    fn cover(&self, info: &PlaylistDetailInfo) -> AnyElement {
        if !info.image_url.is_empty() {
            return MusicCoverImage::new(self.child_id("cover"), info.image_url.clone())
                .fallback_colors(info.colors.clone())
                .size(COVER_SIDE)
                .corner_radius(COVER_RADIUS)
                .into_any_element();
        }
        if info.colors.is_empty() {
            return MusicCoverPlaceholder::new().icon_scale(0.34).corner_radius(COVER_RADIUS).into_any_element();
        }
        let gradient = LinearGradient::colors(
            info.colors.iter().map(|hex| color(hex)),
            UnitPoint::TOP_LEADING,
            UnitPoint::BOTTOM_TRAILING,
        );
        let is_liked = self.collection_id == LIKED_SONGS_ID;
        let symbol = if is_liked { "heart.fill" } else { "music.note.list" };
        zstack(Alignment::Center)
            .relative()
            .size_full()
            .child(div().absolute().inset_0().child(gradient.view()))
            .children(IconUi::sized(symbol, px(60.0), white(), is_liked))
            .into_any_element()
    }

    fn hero_section(&self, info: &PlaylistDetailInfo, theme: &AppTheme) -> AnyElement {
        let cover = div()
            .size(px(COVER_SIDE))
            .shadow(vec![BoxShadow {
                color: black().opacity(0.25),
                offset: point(px(0.0), px(8.0)),
                blur_radius: px(20.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .clip_shape(Shape::continuous(COVER_RADIUS))
            .child(self.cover(info))
            .music_cover_media_viewer(
                &format!("playlist-{}", self.collection_id),
                &info.image_url,
                &info.title,
                COVER_RADIUS,
            );

        let titles = vstack(4.0)
            .w_full()
            .px(px(24.0))
            .child(
                MainText::new(info.title.clone())
                    .px(20.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .color(theme.text.primary)
                    .number_of_lines(3),
            )
            .child(
                ExpandableTextUi::new(
                    self.child_id("subtitle"),
                    info.subtitle.clone(),
                    theme.text.secondary,
                    theme.text.primary,
                )
                .font_size(14.0)
                .line_limit(3)
                .collapse_control_insets(EdgeInsets::new(6.0, 16.0, 6.0, 16.0))
                .alignment(TextAlignT::Center),
            );

        let playback = zstack(Alignment::Center)
            .w_full()
            .min_h(px(PLAYBACK_BUTTON_SIZE))
            .child(
                MusicScopePlaybackButton::new(self.playback_scope_id.clone(), (*info.tracks).clone())
                    .size(PLAYBACK_BUTTON_SIZE)
                    .on_play_started(Self::show_now_playing)
                    .guide_anchor(RiseGuideAnchor::PlaylistPlay),
            )
            .when(self.is_collaborative_playlist, |playback| {
                playback.child(
                    hstack(0.0)
                        .w_full()
                        .max_w(px(LISTENERS_ROW_MAX_WIDTH))
                        .px(px(24.0))
                        .child(spacer_min_w(0.0))
                        .child(self.playlist_listeners_button()),
                )
            });

        vstack(16.0)
            .w_full()
            .pt(px(self.top_inset + 24.0))
            .pb(px(24.0))
            .child(cover)
            .child(titles)
            .child(playback)
            .into_any_element()
    }

    fn playlist_listeners_button(&self) -> AnyElement {
        let state = self.state.clone();
        let presentation = &self.visible_listeners;
        div()
            .id(self.child_id("listeners-hint"))
            .tooltip_label(tr("music_live_listeners_title"))
            .child(
                plain(self.child_id("listeners-button"))
                    .on_press(move |_, cx| {
                        state.update(cx, |state, cx| {
                            state.show_live_listeners = true;
                            cx.notify();
                        });
                    })
                    .child(
                        MusicPlaylistListenerCluster::new(
                            self.child_id("listeners-cluster"),
                            presentation.listeners.clone(),
                            presentation.total_count,
                        )
                        .size(LISTENER_CLUSTER_SIZE)
                        .is_loading(presentation.is_loading),
                    ),
            )
            .guide_anchor(RiseGuideAnchor::PlaylistListeners)
            .into_any_element()
    }

    fn empty_tracks(&self, theme: &AppTheme) -> AnyElement {
        let state = self.state.clone();
        vstack(12.0)
            .w_full()
            .px(px(24.0))
            .py(px(32.0))
            .children(IconUi::sized("music.note.list", px(38.0), theme.text.secondary, false))
            .child(
                MainText::new(tr("music_local_playlist_empty"))
                    .px(16.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.primary),
            )
            .when(self.can_add_local_tracks, |column| {
                column.child(
                    div().w_full().max_w(px(ACTION_BUTTON_MAX_WIDTH)).child(
                        ButtonUi::new(self.child_id("empty-add"))
                            .on_press(move |_, cx| {
                                state.update(cx, |state, cx| {
                                    state.show_add_local_track_sheet = true;
                                    cx.notify();
                                });
                            })
                            .child(MainText::new(tr("music_local_upload_add"))),
                    ),
                )
            })
            .into_any_element()
    }

    fn selectable_track_row(&self, track: &TrackModel, cx: &App) -> AnyElement {
        let can_save = MusicTrackSavePolicy::can_save(track);
        let is_selected = self.state.read(cx).selected_track_ids.contains(&track.id);
        let state = self.state.clone();
        let track_id = track.id.clone();
        let content = hstack(12.0)
            .w_full()
            .child(DownloadSelectionBadge::new(self.child_id(format!("select-badge-{}", track.id)), is_selected))
            .child(
                div().flex_1().min_w_0().child(
                    TrackRow::new(self.child_id(format!("select-track-{}", track.id)), track.clone()).is_playing(false),
                ),
            );
        div()
            .relative()
            .w_full()
            .opacity(if can_save { 1.0 } else { 0.4 })
            .child(content)
            .child(
                div().absolute().inset_0().block_mouse_except_scroll().child(
                    plain(self.child_id(format!("select-{}", track.id)))
                        .size_full()
                        .disabled(!can_save)
                        .on_press(move |_, cx| {
                            if !can_save {
                                return;
                            }
                            state.update(cx, |state, cx| {
                                if !state.selected_track_ids.remove(&track_id) {
                                    state.selected_track_ids.insert(track_id.clone());
                                }
                                cx.notify();
                            });
                        }),
                ),
            )
            .into_any_element()
    }

    fn track_row(&self, index: usize, tracks: &Rc<Vec<TrackModel>>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let Some(track) = tracks.get(index) else {
            return div().into_any_element();
        };
        if self.is_selecting_tracks {
            let is_last = tracks.last().is_some_and(|last| last.id == track.id);
            return vstack(0.0)
                .w_full()
                .child(div().w_full().px(px(16.0)).child(self.selectable_track_row(track, cx)))
                .when(!is_last, |column| {
                    column.child(div().w_full().pl(px(76.0)).child(div().w_full().h(px(0.5)).bg(theme.border._100)))
                })
                .into_any_element();
        }
        let is_playing = {
            let player = MusicPlayerState::shared(cx);
            let player = player.read(cx);
            player.current_track.as_ref().is_some_and(|current| current.id == track.id) && player.is_playing
        };
        let context = if self.can_remove_local_tracks {
            TrackContextSource::OwnPlaylist { playlist_id: self.collection_id.clone() }
        } else {
            TrackContextSource::External
        };
        let share_playlist_id =
            if self.is_album || self.collection_id == LIKED_SONGS_ID { None } else { Some(self.collection_id.clone()) };
        let tapped = track.clone();
        let queue = tracks.clone();
        let scope_id = self.playback_scope_id.clone();
        let row = TrackContextMenuRow::new(self.child_id(format!("track-{}", track.id)), track.clone())
            .context(context)
            .is_playing(is_playing)
            .share_playlist_id(share_playlist_id)
            .allows_local_track_editing(self.is_collaborative_playlist)
            .guide_anchor((index == 0).then_some(RiseGuideAnchor::PlaylistFirstTrack))
            .on_tap(move |_, cx| {
                let (track, queue) = (tapped.clone(), (*queue).clone());
                let scope_id = scope_id.clone();
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.play(track, queue, Some(scope_id.as_str()), true, Some(Box::new(Self::show_now_playing)), cx);
                });
            });
        let scoped = env_scope::<MusicPlaylistLiveScopeIdKey>(
            self.is_collaborative_playlist.then(|| self.collection_id.clone()),
        )
        .child(row);
        let prefetched = scoped.music_pagination_prefetch(
            self.child_id(format!("track-prefetch-{}", track.id)),
            index,
            tracks.len(),
            &MusicServicesStore::liked_cache_key(),
            self.collection_id == LIKED_SONGS_ID,
            PAGINATION_THRESHOLD,
            |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_liked(cx));
            },
            cx,
        );

        let is_last = tracks.last().is_some_and(|last| last.id == track.id);
        vstack(0.0)
            .w_full()
            .child(div().w_full().px(px(16.0)).child(prefetched))
            .when(!is_last, |column| {
                column.child(div().w_full().pl(px(76.0)).child(div().w_full().h(px(0.5)).bg(theme.border._100)))
            })
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct PlaylistDetailContent {
    parts: PlaylistDetailParts,
    info: PlaylistDetailInfo,
    is_loading: bool,
    hold_is_active: bool,
}

impl RenderOnce for PlaylistDetailContent {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut parts = self.parts;
        parts.top_inset = list_shell_floating_top_inset(cx);
        let info = self.info;
        let theme = rise_ui::theme(cx).clone();

        let rows = PlaylistDetailRow::rows(info.tracks.len(), self.is_loading);
        let ids: Vec<SharedString> = rows
            .iter()
            .map(|row| match row {
                PlaylistDetailRow::Hero => SharedString::new_static("hero"),
                PlaylistDetailRow::Skeleton => SharedString::new_static("skeleton"),
                PlaylistDetailRow::Empty => SharedString::new_static("empty"),
                PlaylistDetailRow::Track(index) => SharedString::from(info.tracks[*index].id.clone()),
                PlaylistDetailRow::Foot => SharedString::new_static("foot"),
            })
            .collect();
        let list_state: ListState = parts.state.update(cx, |state, _| {
            state.list_model.sync(ids);
            state.list_model.list().state().clone()
        });

        let background = theme.bg._100;
        let scrollbar =
            ListScrollbar::new(named_child(&parts.id, "scrollbar"), list_state.clone()).top_inset(parts.top_inset);
        let parts = Rc::new(parts);
        div()
            .relative()
            .size_full()
            .bg(background)
            .child(
                list(list_state, move |index, _, cx| match rows.get(index) {
                    Some(row) => parts.row(*row, &info, &theme, cx),
                    None => div().into_any_element(),
                })
                .size_full(),
            )
            .child(scrollbar)
            .when(self.hold_is_active, |content| content.child(div().absolute().inset_0().occlude()))
    }
}
