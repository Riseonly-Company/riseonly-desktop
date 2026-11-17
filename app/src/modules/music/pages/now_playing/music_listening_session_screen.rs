use std::rc::Rc;

use gpui::{App, Context, Empty, Entity, FontWeight, Hsla, Render, SharedString, Subscription, Window, white};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_screen::{
    BottomSheetNavigationState, BottomSheetScreenItem, bottom_sheet_navigation,
};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_pages_ui::BottomSheetPagesUi;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::music::shared::utils::music::music_player_state::{
    MusicListeningBlockedControl, MusicListeningBlockedControlExt, MusicListeningPermissionRequestNotice,
    MusicPlayerState,
};
use crate::modules::music::stores::music_actions::music_types::{
    MusicListeningParticipant, MusicListeningParticipantPermissions, MusicPlaylistListeningSession, MusicUserReference,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

const ROOT_PAGE_ID: &str = "session";
const PERMISSIONS_PAGE_ID: &str = "permissions";
const PAGE_FRACTION: f32 = 0.72;

type SelectParticipant = Rc<dyn Fn(String, &mut Window, &mut App)>;
type CloseAction = Rc<dyn Fn(&mut App)>;

fn glyph(name: &str, side: f32, color: Hsla) -> AnyElement {
    match IconUi::sized(name, px(side), color, name.ends_with(".fill")) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}

fn set_show_listening_session_sheet(player: &Entity<MusicPlayerState>, show: bool, cx: &mut App) {
    player.update(cx, |player, cx| {
        player.show_listening_session_sheet = show;
        cx.notify();
    });
}

/// what the sheet draws from the player
/// host sits at the window root and the player publishes position 2.5 times a second
/// so only a change of these wakes the root
#[derive(Clone, PartialEq)]
struct MusicListeningSessionObservedState {
    show_listening_session_sheet: bool,
    listening_session: Option<MusicPlaylistListeningSession>,
    listening_permission_request_notice: Option<MusicListeningPermissionRequestNotice>,
}

impl MusicListeningSessionObservedState {
    fn read(player: &MusicPlayerState) -> Self {
        Self {
            show_listening_session_sheet: player.show_listening_session_sheet,
            listening_session: player.listening_session.clone(),
            listening_permission_request_notice: player.listening_permission_request_notice.clone(),
        }
    }
}

struct MusicListeningSessionSheetHostState {
    permissions_participant_id: Option<String>,
    observed: MusicListeningSessionObservedState,
    _player_observation: Subscription,
}

#[derive(Default, IntoElement)]
pub struct MusicListeningSessionSheetHost;

impl MusicListeningSessionSheetHost {
    pub fn new() -> Self {
        Self
    }
}

/// desktop only, root draws this layer as its own cached view
/// so the player it reads dirties this layer and not the whole root
pub struct MusicListeningSessionSheetLayer;

impl Render for MusicListeningSessionSheetLayer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(MusicListeningSessionSheetHost::new())
    }
}

impl RenderOnce for MusicListeningSessionSheetHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let player = MusicPlayerState::shared(cx);
        let observed_player = player.clone();
        let state = window.use_keyed_state("music-listening-session-sheet-host", cx, move |_, cx| {
            let observed = MusicListeningSessionObservedState::read(observed_player.read(cx));
            MusicListeningSessionSheetHostState {
                permissions_participant_id: None,
                observed,
                _player_observation: cx.observe(&observed_player, |state: &mut MusicListeningSessionSheetHostState, player, cx| {
                    let observed = MusicListeningSessionObservedState::read(player.read(cx));
                    if state.observed != observed {
                        state.observed = observed;
                        cx.notify();
                    }
                }),
            }
        });

        let (session_id, is_open) = {
            let player = player.read(cx);
            (player.listening_session.as_ref().map(|session| session.session_id.clone()), player.show_listening_session_sheet)
        };
        let session_player = player.clone();
        on_change(
            "music-listening-session-sheet-session-id",
            session_id,
            false,
            window,
            cx,
            move |_, session_id, _, cx| {
                if session_id.is_none() {
                    set_show_listening_session_sheet(&session_player, false, cx);
                }
            },
        );
        on_change("music-listening-session-sheet-open", is_open, true, window, cx, |_, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::ListeningSessionSheetOpened, *open, cx);
            if !*open {
                GuideFacts::set(RiseGuideSignal::SessionPermissionsOpened, false, cx);
            }
        });

        let select_state = state.clone();
        let close_player = player.clone();
        let root_page = BottomSheetScreenItem::new(ROOT_PAGE_ID, tr("music_live_session_title"), PAGE_FRACTION, move |_, _| {
            let select_state = select_state.clone();
            let close_player = close_player.clone();
            MusicListeningSessionScreen::new()
                .permissions_page_id(PERMISSIONS_PAGE_ID)
                .on_select_participant(move |participant_user_id, _, cx| {
                    select_state.update(cx, |state, cx| {
                        state.permissions_participant_id = Some(participant_user_id);
                        cx.notify();
                    });
                })
                .on_close(move |cx| set_show_listening_session_sheet(&close_player, false, cx))
        });
        let selected_state = state;
        let permissions_page = BottomSheetScreenItem::new(
            PERMISSIONS_PAGE_ID,
            tr("chat_settings_permissions"),
            PAGE_FRACTION,
            move |_, cx| match selected_state.read(cx).permissions_participant_id.clone() {
                Some(participant_user_id) => {
                    MusicListeningParticipantPermissionsScreen::new(participant_user_id).into_any_element()
                }
                None => Empty.into_any_element(),
            },
        );

        let refused_player = player.clone();
        BottomSheetPagesUi::new(
            "music-listening-session-sheet",
            Binding::entity(
                player,
                |player: &MusicPlayerState| player.show_listening_session_sheet,
                |player, show, cx| {
                    player.show_listening_session_sheet = show;
                    cx.notify();
                },
            ),
            vec![root_page, permissions_page],
            ROOT_PAGE_ID,
        )
        .sheet_background(Some(theme.bg._100))
        .on_presentation_refused(move |_, cx| set_show_listening_session_sheet(&refused_player, false, cx))
        .on_screen_change(Some(|screen_id: &str, _: &mut Window, cx: &mut App| {
            GuideFacts::set(RiseGuideSignal::SessionPermissionsOpened, screen_id == PERMISSIONS_PAGE_ID, cx);
        }))
    }
}

#[derive(IntoElement)]
pub struct MusicListeningSessionScreen {
    permissions_page_id: String,
    on_select_participant: SelectParticipant,
    on_close: CloseAction,
}

impl Default for MusicListeningSessionScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicListeningSessionScreen {
    pub fn new() -> Self {
        Self {
            permissions_page_id: String::new(),
            on_select_participant: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
            on_close: Rc::new(|_: &mut App| {}),
        }
    }

    pub fn permissions_page_id(mut self, permissions_page_id: impl Into<String>) -> Self {
        self.permissions_page_id = permissions_page_id.into();
        self
    }

    pub fn on_select_participant(mut self, on_select_participant: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        self.on_select_participant = Rc::new(on_select_participant);
        self
    }

    pub fn on_close(mut self, on_close: impl Fn(&mut App) + 'static) -> Self {
        self.on_close = Rc::new(on_close);
        self
    }

    fn host_item(user: &MusicUserReference) -> GroupedBtn {
        GroupedBtn::new("host", Self::display_name(user))
            .leading_slot(Self::user_avatar(user, 44.))
            .subtitle(tr("music_live_host"))
            .show_chevron(false)
    }

    fn participant_items(
        &self,
        session: &MusicPlaylistListeningSession,
        player: &Entity<MusicPlayerState>,
        navigation: Option<&Entity<BottomSheetNavigationState>>,
        theme: &AppTheme,
        cx: &App,
    ) -> Vec<GroupedBtn> {
        if session.participants.is_empty() {
            return vec![
                GroupedBtn::new("empty-participants", tr("music_live_participants_empty"))
                    .icon("person.2.fill")
                    .icon_background(ChatTileColor::indigo())
                    .show_chevron(false),
            ];
        }

        let player_state = player.read(cx);
        let is_host = player_state.is_listening_session_host();
        let request_notice = player_state.listening_permission_request_notice.as_ref();
        let first_participant_id = session.participants.first().map(|participant| participant.user_id.as_str());

        session
            .participants
            .iter()
            .map(|participant| {
                let has_pending_request = request_notice.is_some_and(|notice| {
                    notice.session_id == session.session_id && notice.participant_user_id == participant.user_id
                });
                let summary = Self::permission_summary(participant.permissions);
                let subtitle = if has_pending_request {
                    format!("{} \u{b7} {summary}", tr("music_live_permission_requested_badge"))
                } else {
                    summary
                };
                let mut item = GroupedBtn::new(participant.user_id.clone(), Self::display_name(&participant.user))
                    .leading_slot(Self::user_avatar(&participant.user, 40.))
                    .subtitle(subtitle)
                    .guide_anchor(
                        (Some(participant.user_id.as_str()) == first_participant_id)
                            .then_some(RiseGuideAnchor::SessionFirstParticipant),
                    )
                    .show_chevron(is_host)
                    .show_chevron_when_trailing_slot(has_pending_request && is_host);
                if has_pending_request {
                    item = item.trailing_slot(glyph("person.badge.key.fill", 15., theme.primary._100));
                }
                let participant_user_id = participant.user_id.clone();
                let permissions_page_id = self.permissions_page_id.clone();
                let on_select_participant = self.on_select_participant.clone();
                let navigation = navigation.cloned();
                item.callback(move |window, cx| {
                    if !is_host {
                        return;
                    }
                    on_select_participant(participant_user_id.clone(), window, cx);
                    if let Some(navigation) = &navigation {
                        BottomSheetNavigationState::navigate(navigation, permissions_page_id.clone(), window, cx);
                    }
                })
            })
            .collect()
    }

    fn viewer_permission_items(
        permissions: MusicListeningParticipantPermissions,
        player: &Entity<MusicPlayerState>,
        theme: &AppTheme,
    ) -> Vec<GroupedBtn> {
        vec![
            Self::permission_status_item(
                "control-playback",
                MusicListeningBlockedControl::Playback,
                "play.fill",
                ChatTileColor::blue(),
                permissions.can_control_playback,
                player,
                theme,
            ),
            Self::permission_status_item(
                "seek",
                MusicListeningBlockedControl::Seek,
                "slider.horizontal.3",
                ChatTileColor::orange(),
                permissions.can_seek,
                player,
                theme,
            ),
            Self::permission_status_item(
                "change-tracks",
                MusicListeningBlockedControl::ChangeTracks,
                "music.note.list",
                ChatTileColor::indigo(),
                permissions.can_change_tracks,
                player,
                theme,
            ),
            Self::permission_status_item(
                "apply-effects",
                MusicListeningBlockedControl::ApplyEffects,
                "waveform.path.ecg",
                ChatTileColor::purple(),
                permissions.can_apply_effects,
                player,
                theme,
            ),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn permission_status_item(
        id: &'static str,
        control: MusicListeningBlockedControl,
        icon: &'static str,
        icon_background: Hsla,
        enabled: bool,
        player: &Entity<MusicPlayerState>,
        theme: &AppTheme,
    ) -> GroupedBtn {
        let mut item = GroupedBtn::new(id, control.localized_name())
            .icon(icon)
            .icon_background(icon_background)
            .show_chevron(false)
            .trailing_slot(glyph(
                if enabled { "checkmark.circle.fill" } else { "minus.circle.fill" },
                18.,
                if enabled { ChatTileColor::green() } else { theme.text.secondary },
            ));
        if !enabled {
            item = item.subtitle(tr("music_live_request_permissions"));
        }
        let player = player.clone();
        item.callback(move |_, cx| {
            if enabled {
                return;
            }
            player.update(cx, |player, cx| player.request_listening_permission(control, cx));
        })
    }

    fn user_avatar(user: &MusicUserReference, size: f32) -> UserLogoUi {
        UserLogoUi::new(SharedString::from(format!("music-session-avatar-{}", user.id)))
            .source(SharedString::from(user.logo.clone()))
            .name(SharedString::from(user.name.clone()))
            .user_id(SharedString::from(user.id.clone()))
            .size(size)
            .show_story_ring(false)
    }

    fn display_name(user: &MusicUserReference) -> String {
        if user.name.is_empty() { user.tag.clone() } else { user.name.clone() }
    }

    fn permission_summary(permissions: MusicListeningParticipantPermissions) -> String {
        format_printf(
            &tr("music_live_permissions_summary"),
            &[
                FormatArg::Int(permissions.granted_controls().len() as i64),
                FormatArg::Int(MusicListeningBlockedControl::ALL.len() as i64),
            ],
        )
    }

    fn leave_item(&self, is_host: bool, player: &Entity<MusicPlayerState>, theme: &AppTheme) -> GroupedBtn {
        let player = player.clone();
        let on_close = self.on_close.clone();
        GroupedBtn::new("leave-session", tr(if is_host { "music_live_close" } else { "music_live_leave" }))
            .icon(if is_host { "stop.circle.fill" } else { "rectangle.portrait.and.arrow.right" })
            .icon_background(theme.semantic.error_100.opacity(0.14))
            .icon_foreground(theme.semantic.error_100)
            .destructive(true)
            .show_chevron(false)
            .callback(move |_, cx| {
                if is_host {
                    let confirmed_player = player.clone();
                    let confirmed_close = on_close.clone();
                    NativeConfirmAlert::present_delete(
                        &tr("music_live_close_confirmation_title"),
                        &tr("music_live_close_confirmation_message"),
                        Some(tr("music_live_close")),
                        None,
                        false,
                        None,
                        None,
                        move |cx: &mut App| {
                            confirmed_player.update(cx, |player, cx| player.leave_current_playlist_listening_session(cx));
                            confirmed_close(cx);
                        },
                        cx,
                    );
                } else {
                    player.update(cx, |player, cx| player.leave_current_playlist_listening_session(cx));
                    on_close(cx);
                }
            })
    }
}

impl RenderOnce for MusicListeningSessionScreen {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let navigation = bottom_sheet_navigation(cx);
        let player = MusicPlayerState::shared(cx);
        let session = player.read(cx).listening_session.clone();
        let is_host = player.read(cx).is_listening_session_host();

        let content: AnyElement = if let Some(session) = session {
            let mut body = vstack(18.)
                .w_full()
                .pt(px(4.))
                .pb(px(28.))
                .child(GroupedBtns::new("music-session-host", vec![Self::host_item(&session.host_user)]))
                .child(
                    GroupedBtns::new(
                        "music-session-participants",
                        self.participant_items(&session, &player, navigation.as_ref(), &theme, cx),
                    )
                    .title(tr("music_live_participants_title")),
                );
            if !is_host {
                body = body.child(
                    GroupedBtns::new(
                        "music-session-viewer-permissions",
                        Self::viewer_permission_items(session.viewer_permissions, &player, &theme),
                    )
                    .title(tr("chat_settings_permissions")),
                );
            }
            body = body.child(GroupedBtns::new("music-session-leave", vec![self.leave_item(is_host, &player, &theme)]));
            scroll_view("music-listening-session-scroll").shows_indicators(false).size_full().child(body).into_any_element()
        } else {
            vstack(14.)
                .size_full()
                .justify_center()
                .child(glyph("person.2.slash", 38., theme.text.secondary))
                .child(
                    MainText::new(tr("music_live_session_ended_state"))
                        .px(16.)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary),
                )
                .into_any_element()
        };

        div().size_full().child(content)
    }
}

struct MusicListeningParticipantPermissionsScreenState {
    saving: bool,
}

#[derive(IntoElement)]
struct MusicListeningParticipantPermissionsScreen {
    participant_user_id: String,
}

impl MusicListeningParticipantPermissionsScreen {
    fn new(participant_user_id: String) -> Self {
        Self { participant_user_id }
    }

    fn participant(&self, player: &MusicPlayerState) -> Option<MusicListeningParticipant> {
        Self::participant_of(&self.participant_user_id, player)
    }

    fn participant_of(participant_user_id: &str, player: &MusicPlayerState) -> Option<MusicListeningParticipant> {
        player
            .listening_session
            .as_ref()?
            .participants
            .iter()
            .find(|participant| participant.user_id == participant_user_id)
            .cloned()
    }

    fn pending_requested_permissions(
        &self,
        player: &MusicPlayerState,
        participant: Option<&MusicListeningParticipant>,
    ) -> Option<MusicListeningParticipantPermissions> {
        let notice = player.listening_permission_request_notice.as_ref()?;
        let session_id = player.listening_session.as_ref().map(|session| session.session_id.as_str());
        if notice.participant_user_id != self.participant_user_id
            || Some(notice.session_id.as_str()) != session_id
            || Some(notice.permissions) == participant.map(|participant| participant.permissions)
        {
            return None;
        }
        Some(notice.permissions)
    }

    fn permission_toggle_items(
        &self,
        state: &Entity<MusicListeningParticipantPermissionsScreenState>,
        player: &Entity<MusicPlayerState>,
        saving: bool,
        theme: &AppTheme,
    ) -> Vec<GroupedBtn> {
        [
            ("control-playback", "music_live_permission_playback", "play.fill", ChatTileColor::blue(), MusicListeningBlockedControl::Playback),
            ("seek", "music_live_permission_seek", "slider.horizontal.3", ChatTileColor::orange(), MusicListeningBlockedControl::Seek),
            ("change-tracks", "music_live_permission_change_tracks", "music.note.list", ChatTileColor::indigo(), MusicListeningBlockedControl::ChangeTracks),
            ("apply-effects", "music_live_permission_effects", "waveform.path.ecg", ChatTileColor::purple(), MusicListeningBlockedControl::ApplyEffects),
        ]
        .into_iter()
        .map(|(id, title_key, icon, icon_background, control)| {
            self.permission_toggle_item(id, tr(title_key), icon, icon_background, control, state, player, saving, theme)
        })
        .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn permission_toggle_item(
        &self,
        id: &'static str,
        title: String,
        icon: &'static str,
        icon_background: Hsla,
        control: MusicListeningBlockedControl,
        state: &Entity<MusicListeningParticipantPermissionsScreenState>,
        player: &Entity<MusicPlayerState>,
        saving: bool,
        theme: &AppTheme,
    ) -> GroupedBtn {
        GroupedBtn::new(id, title.clone()).disabled(saving).content_slot(
            hstack(12.)
                .w_full()
                .px(px(16.))
                .py(px(11.))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(29.))
                        .continuous_corners(8.)
                        .bg(icon_background)
                        .child(glyph(icon, 14., white())),
                )
                .child(div().flex_1().min_w_0().child(MainText::new(title).px(15.).color(theme.text.primary)))
                .child(div().flex_none().child(SwitchUi::new(
                    SharedString::from(format!("music-session-permission-{id}")),
                    self.permission_binding(control, state, player),
                ))),
        )
    }

    fn permission_binding(
        &self,
        control: MusicListeningBlockedControl,
        state: &Entity<MusicListeningParticipantPermissionsScreenState>,
        player: &Entity<MusicPlayerState>,
    ) -> Binding<bool> {
        let read_player = player.clone();
        let read_participant = self.participant_user_id.clone();
        let write_player = player.clone();
        let write_participant = self.participant_user_id.clone();
        let write_state = state.clone();
        Binding::new(
            move |cx| {
                Self::participant_of(&read_participant, read_player.read(cx))
                    .is_some_and(|participant| participant.permissions.allows(control))
            },
            move |enabled, _, cx| {
                if write_state.read(cx).saving {
                    return;
                }
                let Some(participant) = Self::participant_of(&write_participant, write_player.read(cx)) else {
                    Self::present_error(tr("retry_later_error"), cx);
                    return;
                };
                let current = participant.permissions;
                let updated = MusicListeningParticipantPermissions::new_with_apply_effects(
                    if control == MusicListeningBlockedControl::Playback { enabled } else { current.can_control_playback },
                    if control == MusicListeningBlockedControl::Seek { enabled } else { current.can_seek },
                    if control == MusicListeningBlockedControl::ChangeTracks { enabled } else { current.can_change_tracks },
                    if control == MusicListeningBlockedControl::ApplyEffects { enabled } else { current.can_apply_effects },
                );
                Self::update_permissions(&write_participant, updated, &write_state, &write_player, cx);
            },
        )
    }

    fn present_error(message: String, cx: &mut App) {
        NativeConfirmAlert::present_notice(
            &tr("music_live_permissions_update_failed_title"),
            &message,
            None,
            None,
            |_: &mut App| {},
            cx,
        );
    }

    fn update_permissions(
        participant_user_id: &str,
        permissions: MusicListeningParticipantPermissions,
        state: &Entity<MusicListeningParticipantPermissionsScreenState>,
        player: &Entity<MusicPlayerState>,
        cx: &mut App,
    ) {
        if state.read(cx).saving {
            return;
        }
        let Some(session_id) = player.read(cx).listening_session.as_ref().map(|session| session.session_id.clone()) else {
            Self::present_error(tr("retry_later_error"), cx);
            return;
        };
        state.update(cx, |state, cx| {
            state.saving = true;
            cx.notify();
        });

        let success_state = state.clone();
        let success_player = player.clone();
        let success_participant = participant_user_id.to_owned();
        let error_state = state.clone();
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.update_playlist_listening_participant_permissions(
                &session_id,
                participant_user_id,
                permissions,
                Some(Box::new(move |updated: MusicPlaylistListeningSession, cx: &mut App| {
                    success_state.update(cx, |state, cx| {
                        state.saving = false;
                        cx.notify();
                    });
                    let server_time_ms = MusicServicesStore::shared(cx)
                        .read(cx)
                        .estimated_playlist_server_time_ms(&updated.playlist_id, RiseClock::now().ms());
                    success_player.update(cx, |player, cx| {
                        let answers_notice = player
                            .listening_permission_request_notice
                            .as_ref()
                            .is_some_and(|notice| notice.participant_user_id == success_participant);
                        if answers_notice {
                            player.listening_permission_request_notice = None;
                            cx.notify();
                        }
                        player.attach_playlist_listening_session(updated, server_time_ms, None, cx);
                    });
                })),
                Some(Box::new(move |error: String, cx: &mut App| {
                    error_state.update(cx, |state, cx| {
                        state.saving = false;
                        cx.notify();
                    });
                    Self::present_error(error, cx);
                })),
                cx,
            );
        });
    }
}

impl RenderOnce for MusicListeningParticipantPermissionsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let player = MusicPlayerState::shared(cx);
        let state = window.use_keyed_state(
            SharedString::from(format!("music-session-permissions-{}", self.participant_user_id)),
            cx,
            |_, _| MusicListeningParticipantPermissionsScreenState { saving: false },
        );

        let session_id = player.read(cx).listening_session.as_ref().map(|session| session.session_id.clone());
        let reset_state = state.clone();
        on_change("music-session-permissions-session-id", session_id, false, window, cx, move |_, _, _, cx| {
            reset_state.update(cx, |state, cx| {
                state.saving = false;
                cx.notify();
            });
        });

        let saving = state.read(cx).saving;
        let participant = self.participant(player.read(cx));
        let requested = self.pending_requested_permissions(player.read(cx), participant.as_ref());

        let content: AnyElement = if let Some(participant) = participant {
            let display_name = if participant.user.name.is_empty() {
                participant.user.tag.clone()
            } else {
                participant.user.name.clone()
            };
            let mut body = vstack(18.)
                .w_full()
                .pt(px(4.))
                .pb(px(28.))
                .child(
                    GroupedBtns::new(
                        "music-session-permissions-participant",
                        vec![
                            GroupedBtn::new("participant", display_name)
                                .leading_slot(
                                    UserLogoUi::new(SharedString::from(format!(
                                        "music-session-permissions-avatar-{}",
                                        participant.user_id
                                    )))
                                    .source(SharedString::from(participant.user.logo.clone()))
                                    .name(SharedString::from(participant.user.name.clone()))
                                    .user_id(SharedString::from(participant.user_id.clone()))
                                    .size(44.)
                                    .show_story_ring(false),
                                )
                                .subtitle(tr("chat_settings_permissions"))
                                .show_chevron(false),
                        ],
                    )
                    .surface_color(theme.bg._200),
                )
                .child(
                    GroupedBtns::new(
                        "music-session-permissions-toggles",
                        self.permission_toggle_items(&state, &player, saving, &theme),
                    )
                    .title(tr("chat_settings_permissions"))
                    .surface_color(theme.bg._200),
                );
            if let Some(requested) = requested {
                let granted_state = state.clone();
                let granted_player = player.clone();
                let granted_participant = self.participant_user_id.clone();
                body = body.child(
                    GroupedBtns::new(
                        "music-session-permissions-grant",
                        vec![
                            GroupedBtn::new("grant-requested", tr("music_live_grant_requested"))
                                .icon("person.badge.key.fill")
                                .icon_background(ChatTileColor::green())
                                .subtitle(requested.localized_granted_list())
                                .disabled(saving)
                                .guide_anchor(RiseGuideAnchor::SessionGrantRequested)
                                .show_chevron(false)
                                .callback(move |_, cx| {
                                    Self::update_permissions(
                                        &granted_participant,
                                        requested,
                                        &granted_state,
                                        &granted_player,
                                        cx,
                                    );
                                }),
                        ],
                    )
                    .surface_color(theme.bg._200),
                );
            }
            body.into_any_element()
        } else {
            vstack(14.)
                .w_full()
                .pt(px(72.))
                .child(glyph("person.crop.circle.badge.questionmark", 38., theme.text.secondary))
                .child(
                    MainText::new(tr("music_live_participant_not_found"))
                        .px(16.)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary),
                )
                .into_any_element()
        };

        div()
            .relative()
            .size_full()
            .child(scroll_view("music-session-permissions-scroll").shows_indicators(false).size_full().child(content))
            .when(saving, |screen| {
                screen.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(progress_view().control_size(ControlSize::Large)),
                )
            })
    }
}
