use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, PromptButton, PromptLevel, RenderOnce, SharedString, Subscription,
    Window,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{FormatArg, tr, tr_args, tr_plural};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_screen::{
    BottomSheetNavigationState, BottomSheetScreenItem, bottom_sheet_navigation,
};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_pages_ui::BottomSheetPagesUi;
use crate::core::ui::premium_badge::UserNameWithBadges;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::PlaylistCollaborationErrorState;
use crate::modules::music::pages::playlist_detail::music_playlist_listener_cluster::MusicPlaylistListenerCluster;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::{
    MusicPlaylistListener, MusicPlaylistListeningSession, MusicPlaylistListeningSnapshot,
};
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_mini_player_bottom_padding::{
    hear_player_through_observation_while_covered, observe_player_unless_covered,
};

const ROOT_PAGE_ID: &str = "listeners";
const ACTIVE_SESSIONS_PAGE_ID: &str = "activeSessions";
const PAGE_FRACTION: f32 = 0.66;
const LISTENER_LOGO_SIZE: f32 = 42.0;
const TRAILING_SIDE: f32 = 28.0;
const TRAILING_ICON_SIDE: f32 = 17.0;

type ListenersAction = Rc<dyn Fn(&mut Window, &mut App)>;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

struct MusicPlaylistListeningGroup {
    id: String,
    listeners: Vec<MusicPlaylistListener>,
}

/// state of the sheet and the observed stores
struct MusicPlaylistListenersState {
    joining_user_id: Option<String>,
    join_error: Option<String>,
    _subscriptions: [Subscription; 4],
}

impl MusicPlaylistListenersState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let player = MusicPlayerState::shared(cx);
        Self {
            joining_user_id: None,
            join_error: None,
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&auth, |_, _, cx| cx.notify()),
                observe_player_unless_covered(&player, cx),
            ],
        }
    }
}

/// sheet page deck instead of a navigation stack
/// current page is the decks own fact, read through on_screen_change
#[derive(IntoElement)]
pub struct MusicPlaylistListenersScreen {
    id: ElementId,
    is_presented: Binding<bool>,
    playlist_id: String,
    track_id: Option<String>,
    owns_observation: bool,
    on_open_session: Rc<dyn Fn(&mut App)>,
    on_dismiss: ListenersAction,
    on_did_dismiss: ListenersAction,
}

impl MusicPlaylistListenersScreen {
    pub fn new(id: impl Into<ElementId>, is_presented: Binding<bool>, playlist_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            is_presented,
            playlist_id: playlist_id.into(),
            track_id: None,
            owns_observation: true,
            on_open_session: Rc::new(|_: &mut App| {}),
            on_dismiss: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_did_dismiss: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn track_id(mut self, track_id: Option<String>) -> Self {
        self.track_id = track_id;
        self
    }

    pub fn owns_observation(mut self, owns_observation: bool) -> Self {
        self.owns_observation = owns_observation;
        self
    }

    /// runs from the join success callback so it gets no window
    pub fn on_open_session(mut self, on_open_session: impl Fn(&mut App) + 'static) -> Self {
        self.on_open_session = Rc::new(on_open_session);
        self
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(on_dismiss);
        self
    }

    pub fn on_did_dismiss(mut self, on_did_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_did_dismiss = Rc::new(on_did_dismiss);
        self
    }
}

impl RenderOnce for MusicPlaylistListenersScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state =
            window.use_keyed_state(named_child(&self.id, "state"), cx, |_, cx| MusicPlaylistListenersState::new(cx));
        hear_player_through_observation_while_covered(&MusicPlayerState::shared(cx), cx);

        let is_open = self.is_presented.get(cx);
        let changed_state = state.clone();
        let observed_playlist_id = self.playlist_id.clone();
        let owns_observation = self.owns_observation;
        on_change(named_child(&self.id, "presented"), is_open, true, window, cx, move |was_open, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::ListenersSheetOpened, *open, cx);
            if !*open {
                GuideFacts::set(RiseGuideSignal::ActiveSessionsOpened, false, cx);
                changed_state.update(cx, |state, cx| {
                    state.joining_user_id = None;
                    cx.notify();
                });
            }
            if !owns_observation || was_open == open {
                return;
            }
            MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                if *open {
                    store.start_playlist_listening_observation(&observed_playlist_id, None, cx);
                } else {
                    store.stop_playlist_listening_observation(&observed_playlist_id, None, cx);
                }
            });
        });

        let parts = Rc::new(MusicPlaylistListenersParts {
            id: self.id.clone(),
            state,
            playlist_id: self.playlist_id.clone(),
            track_id: self.track_id.clone(),
            on_open_session: self.on_open_session.clone(),
        });

        let refused = self.is_presented.clone();
        let on_dismiss = self.on_dismiss.clone();
        let on_did_dismiss = self.on_did_dismiss.clone();
        BottomSheetPagesUi::new(named_child(&self.id, "pages"), self.is_presented, parts.pages(), ROOT_PAGE_ID)
            .sheet_background(Some(theme.bg._100))
            .on_dismiss(move |window, cx| on_dismiss(window, cx))
            .on_did_dismiss(move |window, cx| on_did_dismiss(window, cx))
            .on_presentation_refused(move |window, cx| refused.set(false, window, cx))
            .on_screen_change(Some(|screen_id: &str, _: &mut Window, cx: &mut App| {
                GuideFacts::set(RiseGuideSignal::ActiveSessionsOpened, screen_id == ACTIVE_SESSIONS_PAGE_ID, cx);
            }))
    }
}

struct MusicPlaylistListenersParts {
    id: ElementId,
    state: Entity<MusicPlaylistListenersState>,
    playlist_id: String,
    track_id: Option<String>,
    on_open_session: Rc<dyn Fn(&mut App)>,
}

impl MusicPlaylistListenersParts {
    fn snapshot(&self, cx: &mut App) -> Option<MusicPlaylistListeningSnapshot> {
        MusicServicesStore::shared(cx)
            .update(cx, |service, cx| service.playlist_listening_snapshot(&self.playlist_id, None, cx))
    }

    fn current_user_id(cx: &mut App) -> String {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default()
    }

    fn listeners(&self, all_listeners: &[MusicPlaylistListener], current_user_id: &str) -> Vec<MusicPlaylistListener> {
        let mut scoped: Vec<MusicPlaylistListener> = all_listeners
            .iter()
            .filter(|listener| listener.user_id != current_user_id)
            .filter(|listener| self.track_id.as_ref().is_none_or(|track_id| listener.track_id == *track_id))
            .cloned()
            .collect();
        scoped.sort_by(|left, right| right.is_playing.cmp(&left.is_playing).then_with(|| left.user_id.cmp(&right.user_id)));
        scoped
    }

    fn active_session_groups(all_listeners: &[MusicPlaylistListener]) -> Vec<MusicPlaylistListeningGroup> {
        let mut grouped: BTreeMap<String, Vec<MusicPlaylistListener>> = BTreeMap::new();
        for listener in all_listeners {
            let Some(session_id) = listener.session_id.as_deref() else {
                continue;
            };
            if trimming_whitespaces_and_newlines(session_id).is_empty() {
                continue;
            }
            grouped.entry(session_id.to_owned()).or_default().push(listener.clone());
        }
        grouped
            .into_iter()
            .filter_map(|(id, values)| {
                let mut seen = HashSet::new();
                let mut unique: Vec<MusicPlaylistListener> =
                    values.into_iter().filter(|listener| seen.insert(listener.user_id.clone())).collect();
                if unique.len() < 2 {
                    return None;
                }
                unique.sort_by(|left, right| left.user_id.cmp(&right.user_id));
                Some(MusicPlaylistListeningGroup { id, listeners: unique })
            })
            .collect()
    }

    fn pages(self: &Rc<Self>) -> Vec<BottomSheetScreenItem> {
        let root = self.clone();
        let sessions = self.clone();
        vec![
            BottomSheetScreenItem::new(ROOT_PAGE_ID, tr("music_live_listeners_title"), PAGE_FRACTION, move |_, cx| {
                root.root_page(cx)
            }),
            BottomSheetScreenItem::new(
                ACTIVE_SESSIONS_PAGE_ID,
                tr("music_live_active_sessions"),
                PAGE_FRACTION,
                move |_, cx| sessions.active_sessions_page(cx),
            ),
        ]
    }

    fn root_page(self: &Rc<Self>, cx: &mut App) -> MusicPlaylistListenersRootPage {
        let theme = rise_ui::theme(cx).clone();
        let snapshot = self.snapshot(cx);
        let actions = MusicActionsStore::shared(cx);
        let (is_pending, error) = {
            let slice = &actions.read(cx).get_playlist_listening_snapshot_sai;
            (slice.is_pending(), slice.error())
        };
        let is_loading = snapshot.is_none() && is_pending;
        let has_load_error =
            snapshot.is_none() && !is_pending && error.is_some_and(|error| !error.is_empty());
        let current_user_id = Self::current_user_id(cx);
        let all_listeners = snapshot.as_ref().map(|snapshot| snapshot.listeners.clone()).unwrap_or_default();
        let listener_items = self.listener_items(&all_listeners, &current_user_id, &theme, cx);
        let state = self.state.clone();
        let reload = self.clone();

        MusicPlaylistListenersRootPage {
            id: named_child(&self.id, "root-page"),
            theme,
            is_loading,
            has_load_error,
            active_session_count: Self::active_session_groups(&all_listeners).len(),
            listener_items,
            list_title: tr(if self.track_id.is_none() { "music_live_listeners_title" } else { "music_live_on_this_track" }),
            join_error: Binding::entity(
                state,
                |state: &MusicPlaylistListenersState| state.join_error.clone(),
                |state, value, _| state.join_error = value,
            ),
            retry: Rc::new(move |_: &mut Window, cx: &mut App| reload.reload(cx)),
        }
    }

    fn active_sessions_page(self: &Rc<Self>, cx: &mut App) -> MusicPlaylistActiveSessionsPage {
        MusicPlaylistActiveSessionsPage {
            id: named_child(&self.id, "sessions-page"),
            theme: rise_ui::theme(cx).clone(),
            items: self.session_items(cx),
        }
    }

    fn reload(&self, cx: &mut App) {
        MusicInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.refresh_playlist_listening_snapshot(&self.playlist_id, None, cx));
    }

    fn session_items(self: &Rc<Self>, cx: &mut App) -> Vec<GroupedBtn> {
        let snapshot = self.snapshot(cx);
        let current_user_id = Self::current_user_id(cx);
        let all_listeners = snapshot.as_ref().map(|snapshot| snapshot.listeners.clone()).unwrap_or_default();
        let groups = Self::active_session_groups(&all_listeners);
        let first_group_id = groups.first().map(|group| group.id.clone());

        groups
            .into_iter()
            .map(|group| {
                let details = self.session_details(&group.id, snapshot.as_ref(), cx);
                let host_name = details
                    .map(|details| {
                        if details.host_user.name.is_empty() { details.host_user.tag } else { details.host_user.name }
                    })
                    .filter(|name| !name.is_empty())
                    .or_else(|| {
                        group
                            .listeners
                            .iter()
                            .map(|listener| {
                                if listener.user.name.is_empty() { listener.user.tag.clone() } else { listener.user.name.clone() }
                            })
                            .find(|name| !name.is_empty())
                    });

                let others_count =
                    group.listeners.iter().filter(|listener| listener.user_id != current_user_id).count() as i64;

                let text = match host_name {
                    Some(host_name) => tr_args("music_live_session_row_title", &[FormatArg::Text(host_name.into())]),
                    None => tr("music_live_session_title"),
                };
                let is_first = first_group_id.as_deref() == Some(group.id.as_str());
                let cluster = self.session_cluster(&group);
                let opened = self.clone();
                GroupedBtn::new(group.id.clone(), text)
                    .leading_slot(cluster)
                    .subtitle(tr_plural("music_live_participants_count", others_count))
                    .guide_anchor(is_first.then_some(RiseGuideAnchor::ActiveSessionsFirstSession))
                    .callback(move |_, cx| opened.open(&group, cx))
            })
            .collect()
    }

    fn listener_items(
        self: &Rc<Self>,
        all_listeners: &[MusicPlaylistListener],
        current_user_id: &str,
        theme: &AppTheme,
        cx: &mut App,
    ) -> Vec<GroupedBtn> {
        let listeners = self.listeners(all_listeners, current_user_id);
        let first_user_id = listeners.first().map(|listener| listener.user_id.clone());
        let joining_user_id = self.state.read(cx).joining_user_id.clone();
        let playlist_tracks = MusicServicesStore::shared(cx)
            .update(cx, |service, cx| service.playlist(&self.playlist_id, cx))
            .and_then(|playlist| playlist.tracks)
            .unwrap_or_default();
        let player = MusicPlayerState::shared(cx);
        let player = player.read(cx);
        let is_transitioning = player.is_listening_session_transitioning();
        let transition_target = player.listening_session_transition_target_user_id().map(str::to_owned);

        listeners
            .into_iter()
            .map(|listener| {
                let is_current_session = Self::is_in_current_session(&listener, player);
                let is_first = first_user_id.as_deref() == Some(listener.user_id.as_str());
                let is_joining = joining_user_id.as_deref() == Some(listener.user_id.as_str())
                    || transition_target.as_deref() == Some(listener.user_id.as_str());
                let logo = UserLogoUi::new(named_child(&self.id, format!("listener-logo-{}", listener.user_id)))
                    .source(SharedString::from(listener.user.logo.clone()))
                    .name(SharedString::from(listener.user.name.clone()))
                    .user_id(SharedString::from(listener.user_id.clone()))
                    .size(LISTENER_LOGO_SIZE)
                    .open_story_on_press(false)
                    .gift_worn_frame(
                        named_child(&self.id, format!("listener-frame-{}", listener.user_id)),
                        Some(listener.user_id.clone()),
                        LISTENER_LOGO_SIZE,
                    );
                let trailing = Self::listener_trailing(is_current_session, is_joining, listener.is_playing, theme);
                let text = if listener.user.name.is_empty() { listener.user.tag.clone() } else { listener.user.name.clone() };
                let subtitle = Self::track_title(&playlist_tracks, &listener.track_id);
                let joined = self.clone();
                let title = UserNameWithBadges::new(Some(SharedString::from(listener.user_id.clone())), text.clone())
                    .font(15.0, FontWeight::NORMAL)
                    .premium(
                        listener.user.is_premium,
                        listener.user.premium_badge_emoji.clone(),
                        listener.user.name_color.clone(),
                    );
                GroupedBtn::new(listener.user_id.clone(), text)
                    .leading_slot(logo)
                    .title_slot(title)
                    .subtitle(subtitle)
                    .disabled(is_current_session || joining_user_id.is_some() || is_transitioning)
                    .guide_anchor(is_first.then_some(RiseGuideAnchor::ListenersFirstListener))
                    .show_chevron(false)
                    .trailing_slot(trailing)
                    .callback(move |_, cx| {
                        if is_current_session {
                            return;
                        }
                        joined.join(&listener, false, cx);
                    })
            })
            .collect()
    }

    fn listener_trailing(is_current_session: bool, is_joining: bool, is_playing: bool, theme: &AppTheme) -> AnyElement {
        let content: AnyElement = if is_current_session {
            div()
                .children(IconUi::sized("checkmark.circle.fill", px(TRAILING_ICON_SIDE), ChatTileColor::green(), true))
                .into_any_element()
        } else if is_joining {
            progress_view().control_size(ControlSize::Small).tint(theme.primary._100).into_any_element()
        } else {
            let (symbol, tint) = if is_playing {
                ("person.2.wave.2.fill", theme.primary._100)
            } else {
                ("pause.fill", theme.text.secondary)
            };
            div().children(IconUi::sized(symbol, px(TRAILING_ICON_SIDE), tint, true)).into_any_element()
        };
        zstack(Alignment::Center).size(px(TRAILING_SIDE)).child(content).into_any_element()
    }

    fn session_cluster(&self, group: &MusicPlaylistListeningGroup) -> MusicPlaylistListenerCluster {
        MusicPlaylistListenerCluster::new(
            named_child(&self.id, format!("session-cluster-{}", group.id)),
            group.listeners.clone(),
            group.listeners.len() as i64,
        )
        .size(LISTENER_LOGO_SIZE)
    }

    fn track_title(tracks: &[RiseStoredMusicTrack], id: &str) -> String {
        tracks
            .iter()
            .find(|track| track.id.as_deref() == Some(id))
            .and_then(|track| track.title.clone())
            .unwrap_or_else(|| tr("music_live_listening_now"))
    }

    fn session_details(
        &self,
        session_id: &str,
        snapshot: Option<&MusicPlaylistListeningSnapshot>,
        cx: &mut App,
    ) -> Option<MusicPlaylistListeningSession> {
        let player = MusicPlayerState::shared(cx);
        if let Some(current) = player.read(cx).listening_session.as_ref()
            && current.session_id == session_id
        {
            return Some(current.clone());
        }
        snapshot
            .and_then(|snapshot| snapshot.active_session.as_ref())
            .filter(|active| active.session_id == session_id)
            .cloned()
    }

    fn open(self: &Rc<Self>, group: &MusicPlaylistListeningGroup, cx: &mut App) {
        let snapshot = self.snapshot(cx);
        let current_user_id = Self::current_user_id(cx);
        let Some(session) = self.session_details(&group.id, snapshot.as_ref(), cx) else {
            let Some(target) = group.listeners.iter().find(|listener| listener.user_id != current_user_id) else {
                return;
            };
            self.join(target, true, cx);
            return;
        };
        let is_current = {
            let player = MusicPlayerState::shared(cx);
            let player = player.read(cx);
            player.listening_session.as_ref().is_some_and(|current| current.session_id == session.session_id)
                && !player.is_listening_playback_displaced_by_ad_hoc()
        };
        if is_current {
            (self.on_open_session)(cx);
            return;
        }
        let Some(host) = group.listeners.iter().find(|listener| listener.user_id == session.host_user_id) else {
            return;
        };
        self.join(host, true, cx);
    }

    fn join(self: &Rc<Self>, listener: &MusicPlaylistListener, open_session_after_join: bool, cx: &mut App) {
        let state = self.state.clone();
        let (is_current_session, is_transitioning) = {
            let player = MusicPlayerState::shared(cx);
            let player = player.read(cx);
            (Self::is_in_current_session(listener, player), player.is_listening_session_transitioning())
        };
        if is_current_session {
            state.update(cx, |state, cx| {
                state.joining_user_id = None;
                cx.notify();
            });
            if open_session_after_join {
                (self.on_open_session)(cx);
            }
            return;
        }
        if state.read(cx).joining_user_id.is_some() || is_transitioning {
            return;
        }

        let success_state = state.clone();
        let error_state = state.clone();
        let cancelled_state = state.clone();
        let on_open_session = self.on_open_session.clone();
        let reloaded = self.clone();
        let player = MusicPlayerState::shared(cx);
        let accepted = player.update(cx, |player, cx| {
            player.request_join_playlist_listening_session(
                &self.playlist_id,
                &listener.user_id,
                listener.session_id.as_deref(),
                Some(Box::new(move |_: MusicPlaylistListeningSession, cx: &mut App| {
                    success_state.update(cx, |state, cx| {
                        state.joining_user_id = None;
                        cx.notify();
                    });
                    if open_session_after_join {
                        on_open_session(cx);
                    }
                })),
                Some(Box::new(move |error: String, cx: &mut App| {
                    error_state.update(cx, |state, cx| {
                        state.joining_user_id = None;
                        cx.notify();
                    });
                    if error == tr("music_live_session_ended_state") {
                        let options = NotifierSendOptions { r#type: NotifierType::Info, ..Default::default() };
                        Notifier::send(error, options, cx);
                        reloaded.reload(cx);
                        return;
                    }
                    error_state.update(cx, |state, cx| {
                        state.join_error = Some(error);
                        cx.notify();
                    });
                })),
                Some(Box::new(move |cx: &mut App| {
                    cancelled_state.update(cx, |state, cx| {
                        state.joining_user_id = None;
                        cx.notify();
                    });
                })),
                cx,
            )
        });
        let is_transitioning = player.read(cx).is_listening_session_transitioning();
        let target_user_id = listener.user_id.clone();
        state.update(cx, |state, cx| {
            state.joining_user_id = (accepted && is_transitioning).then_some(target_user_id);
            cx.notify();
        });
    }

    fn is_in_current_session(listener: &MusicPlaylistListener, player: &MusicPlayerState) -> bool {
        if player.is_listening_playback_displaced_by_ad_hoc() {
            return false;
        }
        match (player.listening_session.as_ref(), listener.session_id.as_deref()) {
            (Some(current), Some(listener_session_id)) => current.session_id == listener_session_id,
            _ => false,
        }
    }
}

/// root page of the sheet
/// navigation to active sessions goes through the deck environment
/// join error alert lives here inside the presented sheet
#[derive(IntoElement)]
struct MusicPlaylistListenersRootPage {
    id: ElementId,
    theme: AppTheme,
    is_loading: bool,
    has_load_error: bool,
    active_session_count: usize,
    listener_items: Vec<GroupedBtn>,
    list_title: String,
    join_error: Binding<Option<String>>,
    retry: ButtonAction,
}

impl MusicPlaylistListenersRootPage {
    fn join_error_alert(&self, window: &mut Window, cx: &mut App) {
        let message = self.join_error.get(cx);
        let join_error = self.join_error.clone();
        on_change(named_child(&self.id, "join-error"), message, true, window, cx, move |_, message, window, cx| {
            let Some(message) = message else {
                return;
            };
            let answer = window.prompt(
                PromptLevel::Info,
                &tr("music_live_join_failed_title"),
                Some(message.as_str()),
                &[PromptButton::cancel(tr("ok"))],
                cx,
            );
            window
                .spawn(cx, async move |cx| {
                    let _ = answer.await;
                    let _ = cx.update(|window, cx| join_error.set(None, window, cx));
                })
                .detach();
        });
    }

    /// empty card sits in the same column as the grouped buttons so corner
    /// radius and side inset come from the shared card geometry
    fn empty_listeners(&self) -> AnyElement {
        let theme = &self.theme;
        div()
            .w_full()
            .px(px(GroupedCardMetrics::HORIZONTAL_PADDING))
            .child(GroupedCardMetrics::SHAPE.round(
                vstack(10.0)
                    .w_full()
                    .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                    .py(px(28.0))
                    .bg(theme.bg._200)
                    .children(IconUi::sized("person.2.slash", px(30.0), theme.text.secondary, false))
                    .child(
                        MainText::new(tr("music_live_no_other_listeners"))
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    ),
            ))
            .into_any_element()
    }
}

impl RenderOnce for MusicPlaylistListenersRootPage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.join_error_alert(window, cx);
        let theme = self.theme.clone();

        if self.is_loading {
            return vstack(12.0)
                .size_full()
                .justify_center()
                .child(progress_view().control_size(ControlSize::Large).tint(theme.primary._100))
                .child(SecondaryText::new(tr("music_live_listeners_loading")).px(14.0).tac(TextAlignT::Center))
                .into_any_element();
        }
        if self.has_load_error {
            return PlaylistCollaborationErrorState::new(
                named_child(&self.id, "error"),
                tr("music_live_listeners_error"),
                Some(self.retry.clone()),
            )
            .into_any_element();
        }

        let navigation = bottom_sheet_navigation(cx);
        let active_sessions = GroupedBtn::new("active-sessions", tr("music_live_active_sessions"))
            .icon("person.2.wave.2.fill")
            .icon_background(ChatTileColor::green())
            .trailing(self.active_session_count.to_string())
            .guide_anchor(RiseGuideAnchor::ListenersActiveSessions)
            .callback(move |window, cx| {
                if let Some(navigation) = navigation.as_ref() {
                    BottomSheetNavigationState::navigate(navigation, ACTIVE_SESSIONS_PAGE_ID, window, cx);
                }
            });
        let active_sessions = if self.active_session_count == 0 {
            active_sessions.subtitle(tr("music_live_no_active_sessions"))
        } else {
            active_sessions
        };

        let separator = theme.border._200.opacity(0.55);
        let empty_listeners = self.listener_items.is_empty().then(|| self.empty_listeners());
        let listeners_id = named_child(&self.id, "listeners");
        let list_title = self.list_title;
        let listener_items = self.listener_items;

        scroll_view(named_child(&self.id, "scroll"))
            .shows_indicators(false)
            .child(
                vstack(20.0)
                    .w_full()
                    .pt(px(8.0))
                    .pb(px(28.0))
                    .child(
                        GroupedBtns::new(named_child(&self.id, "sessions-group"), vec![active_sessions])
                            .surface_color(theme.bg._200)
                            .primary_text_color(theme.text.primary)
                            .secondary_text_color(theme.text.secondary)
                            .separator_color(separator),
                    )
                    .children(empty_listeners)
                    .when(!listener_items.is_empty(), |column| {
                        column.child(
                            GroupedBtns::new(listeners_id, listener_items)
                                .title(list_title)
                                .surface_color(theme.bg._200)
                                .primary_text_color(theme.text.primary)
                                .secondary_text_color(theme.text.secondary)
                                .separator_color(separator),
                        )
                    }),
            )
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct MusicPlaylistActiveSessionsPage {
    id: ElementId,
    theme: AppTheme,
    items: Vec<GroupedBtn>,
}

impl RenderOnce for MusicPlaylistActiveSessionsPage {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let scroll = scroll_view(named_child(&self.id, "scroll")).shows_indicators(false);
        if self.items.is_empty() {
            return scroll.child(
                vstack(12.0)
                    .w_full()
                    .pt(px(56.0))
                    .children(IconUi::sized("person.2.slash", px(34.0), theme.text.secondary, false))
                    .child(
                        MainText::new(tr("music_live_no_active_sessions"))
                            .px(15.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary),
                    ),
            );
        }
        scroll.child(
            div().w_full().pt(px(8.0)).pb(px(28.0)).child(
                GroupedBtns::new(named_child(&self.id, "group"), self.items)
                    .title(tr("music_live_active_sessions"))
                    .surface_color(theme.bg._200)
                    .primary_text_color(theme.text.primary)
                    .secondary_text_color(theme.text.secondary)
                    .separator_color(theme.border._200.opacity(0.55)),
            ),
        )
    }
}
