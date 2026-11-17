use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Empty, Entity, FontWeight, SharedString, Subscription, Task, WeakEntity, Window};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_music::PlaylistVisibility;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationCover, PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState,
    PlaylistMemberStatusBadge, PlaylistPendingBadge,
};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistSettings;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicPlaylist;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::playlists::edit_playlist_sheet::EditPlaylistSheet;

/// state of the screen
/// the two alerts present themselves through the native confirm
/// alert so the show confirmation flags have no twin
struct PlaylistSettingsScreenState {
    show_edit_sheet: bool,
    leaving: bool,
    deleting: bool,
    new_tracks_on_top_asked: Option<bool>,
    _subscriptions: [Subscription; 2],
}

impl PlaylistSettingsScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        Self {
            show_edit_sheet: false,
            leaving: false,
            deleting: false,
            new_tracks_on_top_asked: None,
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
            ],
        }
    }
}

fn load(playlist_id: &str, force_refresh: bool, cx: &mut App) {
    MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_playlist_settings(playlist_id, force_refresh, cx));
}

fn is_collaborative(settings: &MusicPlaylistSettings) -> bool {
    settings.playlist.visibility == PlaylistVisibility::LocalPlaylist.wire()
}

fn visibility_label(visibility: &str) -> String {
    if visibility == PlaylistVisibility::PublicPlaylist.wire() {
        tr("music_visibility_public")
    } else if visibility == PlaylistVisibility::LocalPlaylist.wire() {
        tr("music_visibility_local")
    } else {
        tr("music_visibility_private")
    }
}

fn edit_playlist_dto(playlist_id: &str, settings: Option<&MusicPlaylistSettings>, cx: &mut App) -> Option<RiseStoredMusicPlaylist> {
    if let Some(playlist) = MusicServicesStore::shared(cx).update(cx, |services, cx| services.playlist(playlist_id, cx)) {
        return Some(playlist);
    }
    let playlist = &settings?.playlist;
    Some(RiseStoredMusicPlaylist {
        id: Some(playlist.id.clone()),
        owner_user_id: Some(playlist.owner_user_id.clone()),
        title: Some(playlist.title.clone()),
        description: Some(playlist.description.clone()),
        cover_url: Some(playlist.cover_url.clone()),
        visibility: Some(playlist.visibility.clone()),
        tracks: None,
        created_at_ms: Some(playlist.created_at_ms),
        updated_at_ms: Some(playlist.updated_at_ms),
        ..RiseStoredMusicPlaylist::default()
    })
}

fn error_options(content: String) -> NotifierSendOptions {
    NotifierSendOptions {
        content: Some(content),
        r#type: NotifierType::Error,
        position: NotifierPosition::Bottom,
        ..NotifierSendOptions::default()
    }
}

/// what the rows and alerts capture from the view
#[derive(Clone)]
struct PlaylistSettingsActions {
    playlist_id: String,
    nav_store: Option<Entity<NavigationStore>>,
    state: WeakEntity<PlaylistSettingsScreenState>,
    new_tracks_on_top_asked: Option<bool>,
}

impl PlaylistSettingsActions {
    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = self.nav_store.as_ref() {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn pop_to_root(&self, cx: &mut App) {
        if let Some(nav_store) = self.nav_store.as_ref() {
            nav_store.update(cx, |nav_store, cx| nav_store.pop_to_root(cx));
        }
    }

    fn update_state(&self, update: impl FnOnce(&mut PlaylistSettingsScreenState), cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| {
            update(state);
            cx.notify();
        });
    }

    fn confirm_delete(&self, cx: &mut App) {
        let actions = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("music_delete_playlist"),
            &tr("music_confirm_delete_playlist"),
            Some(tr("music_delete_playlist")),
            None,
            false,
            None,
            None,
            move |cx| actions.delete(cx),
            cx,
        );
    }

    fn confirm_leave(&self, cx: &mut App) {
        let actions = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("music_playlist_leave_title"),
            &tr("music_playlist_leave_message"),
            Some(tr("music_playlist_leave_confirm")),
            None,
            false,
            None,
            None,
            move |cx| actions.leave(cx),
            cx,
        );
    }

    fn delete(&self, cx: &mut App) {
        let already_deleting = self.state.upgrade().is_some_and(|state| state.read(cx).deleting);
        if already_deleting {
            return;
        }
        self.update_state(|state| state.deleting = true, cx);
        let succeeded = self.clone();
        let failed = self.clone();
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            succeeded.update_state(|state| state.deleting = false, cx);
            succeeded.pop_to_root(cx);
        });
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |error: String, cx: &mut App| {
            failed.update_state(|state| state.deleting = false, cx);
            Notifier::send(tr("music_delete_playlist"), error_options(error), cx);
        });
        let playlist_id = self.playlist_id.clone();
        MusicInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.delete_playlist(&playlist_id, Some(on_success), Some(on_error), cx));
    }

    fn set_new_tracks_on_top(&self, is_on: bool, cx: &mut App) {
        let previous = self.state.upgrade().and_then(|state| state.read(cx).new_tracks_on_top_asked);
        self.update_state(|state| state.new_tracks_on_top_asked = Some(is_on), cx);
        let refreshed = self.playlist_id.clone();
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| load(&refreshed, true, cx));
        let failed = self.clone();
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |error: String, cx: &mut App| {
            failed.update_state(|state| state.new_tracks_on_top_asked = previous, cx);
            Notifier::send(tr("music_playlist_new_tracks_on_top"), error_options(error), cx);
        });
        let playlist_id = self.playlist_id.clone();
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.set_playlist_new_tracks_on_top(&playlist_id, is_on, Some(on_success), Some(on_error), cx);
        });
    }

    fn leave(&self, cx: &mut App) {
        let already_leaving = self.state.upgrade().is_some_and(|state| state.read(cx).leaving);
        if already_leaving {
            return;
        }
        self.update_state(|state| state.leaving = true, cx);
        let succeeded = self.clone();
        let failed = self.clone();
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            succeeded.update_state(|state| state.leaving = false, cx);
            succeeded.pop_to_root(cx);
        });
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |error: String, cx: &mut App| {
            failed.update_state(|state| state.leaving = false, cx);
            Notifier::send(tr("music_playlist_load_error"), error_options(error), cx);
        });
        let playlist_id = self.playlist_id.clone();
        MusicInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.leave_playlist_collaboration(&playlist_id, Some(on_success), Some(on_error), cx));
    }

    fn collaboration_items(&self, settings: &MusicPlaylistSettings) -> Vec<GroupedBtn> {
        let mut items = Vec::new();

        if settings.viewer_permissions.can_edit_playlist {
            let actions = self.clone();
            items.push(
                GroupedBtn::new("edit", tr("music_edit_playlist"))
                    .icon("pencil")
                    .icon_background(ChatTileColor::indigo())
                    .disabled(!settings.has_active_access())
                    .callback(move |_, cx| actions.update_state(|state| state.show_edit_sheet = true, cx)),
            );
            let is_on = self.new_tracks_on_top_asked.unwrap_or(settings.playlist.new_tracks_on_top);
            let has_active_access = settings.has_active_access();
            let toggled = self.clone();
            let tapped = self.clone();
            items.push(
                GroupedBtn::new("new_tracks_on_top", tr("music_playlist_new_tracks_on_top"))
                    .icon("arrow.up.to.line")
                    .icon_background(ChatTileColor::blue())
                    .subtitle(tr("music_playlist_new_tracks_on_top_hint"))
                    .subtitle_line_limit(2)
                    .disabled(!has_active_access)
                    .show_chevron(false)
                    .trailing_slot(SwitchUi::new(
                        ElementId::Name(SharedString::from(format!("playlist-settings-{}-new-tracks-on-top", self.playlist_id))),
                        Binding::new(
                            move |_: &App| is_on,
                            move |value: bool, _, cx| {
                                if has_active_access {
                                    toggled.set_new_tracks_on_top(value, cx);
                                }
                            },
                        ),
                    ))
                    .callback(move |_, cx| tapped.set_new_tracks_on_top(!is_on, cx)),
            );
        }

        if !is_collaborative(settings) {
            return items;
        }

        let actions = self.clone();
        items.push(
            GroupedBtn::new("invite", tr("music_playlist_invite_link"))
                .icon("link")
                .icon_background(ChatTileColor::blue())
                .subtitle(tr("music_playlist_invite_link_subtitle"))
                .subtitle_line_limit(2)
                .disabled(!settings.can_invite || !settings.has_active_access())
                .guide_anchor(RiseGuideAnchor::PlaylistSettingsInviteLink)
                .callback(move |_, cx| {
                    actions.push(RootRoute::MusicPlaylistInviteLink { playlist_id: actions.playlist_id.clone() }, cx);
                }),
        );

        if settings.can_review_requests {
            let actions = self.clone();
            items.push(
                GroupedBtn::new("requests", tr("music_playlist_join_requests"))
                    .icon("person.crop.circle.badge.clock")
                    .icon_background(ChatTileColor::orange())
                    .guide_anchor(RiseGuideAnchor::PlaylistSettingsJoinRequests)
                    .trailing_slot(PlaylistPendingBadge::new(settings.pending_request_count))
                    .show_chevron_when_trailing_slot(true)
                    .callback(move |_, cx| {
                        actions.push(RootRoute::MusicPlaylistJoinRequests { playlist_id: actions.playlist_id.clone() }, cx);
                    }),
            );
        }

        let actions = self.clone();
        items.push(
            GroupedBtn::new("members", tr("music_playlist_members"))
                .icon("person.2.fill")
                .icon_background(ChatTileColor::green())
                .trailing(settings.active_participant_count.to_string())
                .guide_anchor(RiseGuideAnchor::PlaylistSettingsMembers)
                .callback(move |_, cx| {
                    actions.push(RootRoute::MusicPlaylistMembers { playlist_id: actions.playlist_id.clone() }, cx);
                }),
        );
        items
    }

    fn plan_items(&self, settings: &MusicPlaylistSettings) -> Vec<GroupedBtn> {
        let mut items = vec![
            GroupedBtn::new(
                "plan",
                if settings.owner_is_premium { tr("music_playlist_plan_premium") } else { tr("music_playlist_plan_free") },
            )
            .icon(if settings.owner_is_premium { "diamond.fill" } else { "person.2" })
            .icon_background(if settings.owner_is_premium { ChatTileColor::indigo() } else { ChatTileColor::gray() })
            .trailing(format!("{} / {}", settings.active_participant_count, settings.effective_participant_limit))
            .show_chevron(false),
        ];

        if settings.is_owner() && !settings.owner_is_premium {
            let actions = self.clone();
            items.push(
                GroupedBtn::new("upgrade", tr("music_playlist_upgrade"))
                    .icon("sparkles")
                    .icon_background(ChatTileColor::indigo())
                    .callback(move |_, cx| actions.push(RootRoute::SubscriptionSettings, cx)),
            );
        }

        if settings.suspended_member_count > 0 {
            let actions = self.clone();
            items.push(
                GroupedBtn::new("suspended", tr("music_playlist_suspended_members"))
                    .icon("lock.fill")
                    .icon_background(ChatTileColor::gray())
                    .trailing(settings.suspended_member_count.to_string())
                    .callback(move |_, cx| {
                        actions.push(RootRoute::MusicPlaylistMembers { playlist_id: actions.playlist_id.clone() }, cx);
                    }),
            );
        }
        items
    }
}

#[derive(IntoElement)]
struct PlaylistSettingsScroll {
    id: ElementId,
    playlist_id: String,
    content: AnyElement,
}

impl RenderOnce for PlaylistSettingsScroll {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let playlist_id = self.playlist_id;
        scroll_view(self.id)
            .list_shell_scroll_under_chrome(cx)
            .bg(theme.bg._100)
            .refreshable(move |_, cx| {
                load(&playlist_id, true, cx);
                Task::ready(())
            })
            .child(self.content)
    }
}

#[derive(IntoElement)]
pub struct PlaylistSettingsScreen {
    id: ElementId,
    playlist_id: String,
}

impl PlaylistSettingsScreen {
    pub fn new(playlist_id: impl Into<String>) -> Self {
        let playlist_id = playlist_id.into();
        Self { id: ElementId::Name(SharedString::from(format!("playlist-settings-{playlist_id}"))), playlist_id }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn settings_content(
        &self,
        settings: &MusicPlaylistSettings,
        actions: &PlaylistSettingsActions,
        deleting: bool,
        leaving: bool,
        theme: &AppTheme,
    ) -> Div {
        let collaborative = is_collaborative(settings);
        let mut column = vstack(18.0).w_full().child(self.playlist_header(settings, theme));

        if !settings.has_active_access() {
            column = column.child(Self::suspended_card(theme));
        }

        let mut collaboration = GroupedBtns::new(self.child_id("collaboration"), actions.collaboration_items(settings));
        if collaborative {
            collaboration = collaboration.title(tr("music_playlist_collaboration_section"));
        }
        column = column.child(collaboration);

        if collaborative {
            let mut plan =
                GroupedBtns::new(self.child_id("plan"), actions.plan_items(settings)).title(tr("music_playlist_plan_section"));
            if !settings.owner_is_premium {
                plan = plan.end_group_title(tr("music_playlist_free_limit_hint"));
            }
            column = column.child(plan);
        }

        let row_actions = actions.clone();
        let closing_row = if settings.is_owner() {
            GroupedBtn::new("delete", tr("music_delete_playlist"))
                .icon("trash")
                .icon_background(ChatTileColor::red())
                .destructive(true)
                .disabled(deleting)
                .show_chevron(false)
                .callback(move |_, cx| row_actions.confirm_delete(cx))
        } else {
            GroupedBtn::new("leave", tr("music_playlist_leave"))
                .icon("rectangle.portrait.and.arrow.right")
                .icon_background(ChatTileColor::red())
                .destructive(true)
                .disabled(leaving)
                .show_chevron(false)
                .callback(move |_, cx| row_actions.confirm_leave(cx))
        };
        column.child(GroupedBtns::new(self.child_id("closing"), vec![closing_row]))
    }

    fn playlist_header(&self, settings: &MusicPlaylistSettings, theme: &AppTheme) -> Div {
        let playlist = &settings.playlist;
        let subtitle =
            if playlist.description.is_empty() { visibility_label(&playlist.visibility) } else { playlist.description.clone() };

        let mut badges = hstack(8.0)
            .child(PlaylistMemberStatusBadge::new(settings.viewer_role.clone(), settings.access_state.clone()));
        if is_collaborative(settings) {
            badges = badges.child(
                div().px(px(9.0)).py(px(5.0)).bg(theme.bg._200).capsule().child(
                    MainText::new(format!("{} / {}", settings.active_participant_count, settings.effective_participant_limit))
                        .px(12.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.secondary),
                ),
            );
        }

        vstack(12.0)
            .w_full()
            .px(px(18.0))
            .pb(px(4.0))
            .child(PlaylistCollaborationCover::new(self.child_id("cover"), playlist.cover_url.clone()).size(104.0))
            .child(
                vstack(5.0)
                    .child(
                        MainText::new(playlist.title.clone())
                            .px(22.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme.text.primary)
                            .number_of_lines(2),
                    )
                    .child(
                        MainText::new(subtitle)
                            .px(14.0)
                            .tac(TextAlignT::Center)
                            .color(theme.text.secondary)
                            .number_of_lines(3),
                    ),
            )
            .child(badges)
    }

    fn suspended_card(theme: &AppTheme) -> Div {
        div().w_full().px(px(12.0)).child(
            hstack(12.0)
                .align(VerticalAlignment::Top)
                .w_full()
                .p(px(14.0))
                .bg(theme.bg._200)
                .rounded_rect(18.0)
                .child(
                    div()
                        .size(px(30.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.semantic.error_100.opacity(0.14))
                        .rounded_full()
                        .children(IconUi::sized("lock.fill", px(17.0), theme.semantic.error_100, true)),
                )
                .child(
                    vstack(4.0)
                        .flex_1()
                        .min_w_0()
                        .align(HorizontalAlignment::Leading)
                        .child(
                            MainText::new(tr("music_playlist_suspended_title"))
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.text.primary),
                        )
                        .child(
                            MainText::new(tr("music_playlist_suspended_hint"))
                                .px(13.0)
                                .color(theme.text.secondary)
                                .number_of_lines(4),
                        ),
                ),
        )
    }
}

impl RenderOnce for PlaylistSettingsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| PlaylistSettingsScreenState::new(cx));

        let task_playlist_id = self.playlist_id.clone();
        use_task(self.child_id("task"), (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| load(&task_playlist_id, false, cx));
        });

        let theme = rise_ui::theme(cx).clone();
        let settings = MusicServicesStore::shared(cx).update(cx, |services, cx| services.playlist_settings(&self.playlist_id, cx));
        let load_error = MusicActionsStore::slices(cx)
            .get_playlist_settings_sai
            .error()
            .filter(|error| !error.is_empty());
        let (deleting, leaving, new_tracks_on_top_asked) = {
            let state = state.read(cx);
            (state.deleting, state.leaving, state.new_tracks_on_top_asked)
        };
        let actions = PlaylistSettingsActions {
            playlist_id: self.playlist_id.clone(),
            nav_store: nav_store.clone(),
            state: state.downgrade(),
            new_tracks_on_top_asked,
        };

        let body = if let Some(settings) = settings.as_ref() {
            self.settings_content(settings, &actions, deleting, leaving, &theme).into_any_element()
        } else if load_error.is_some() {
            let retry_playlist_id = self.playlist_id.clone();
            let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| load(&retry_playlist_id, false, cx));
            PlaylistCollaborationErrorState::new(self.child_id("error"), tr("music_playlist_load_error"), Some(retry))
                .into_any_element()
        } else {
            PlaylistCollaborationLoadingState::new().into_any_element()
        };

        let keys_playlist_id = self.playlist_id.clone();
        let shell = SettingsWrapperUi::new(tr("music_playlist_settings_title"), move |_, cx| {
            if let Some(nav_store) = nav_store.as_ref() {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PlaylistSettingsScroll {
            id: self.child_id("scroll"),
            playlist_id: self.playlist_id.clone(),
            content: div().w_full().pt(px(12.0)).pb(px(90.0)).child(body).into_any_element(),
        });

        let is_bottom_sheet = Binding::entity(
            state.clone(),
            |state: &PlaylistSettingsScreenState| state.show_edit_sheet,
            |state, value, _| {
                state.show_edit_sheet = value;
            },
        );
        let sheet_id = self.child_id("edit-sheet-content");
        let sheet_playlist_id = self.playlist_id.clone();
        let sheet_settings = settings.clone();
        let sheet_actions = actions.clone();
        let edit_sheet = BottomSheetUi::new(
            self.child_id("edit-sheet"),
            is_bottom_sheet,
            move |window: &mut Window, cx: &mut App| {
                let Some(playlist) = edit_playlist_dto(&sheet_playlist_id, sheet_settings.as_ref(), cx) else {
                    return Empty.into_any_element();
                };
                let dismissing = sheet_actions.clone();
                let on_dismiss: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
                    dismissing.update_state(|state| state.show_edit_sheet = false, cx);
                    load(&dismissing.playlist_id, false, cx);
                });
                window
                    .use_keyed_state(sheet_id.clone(), cx, move |_, cx| EditPlaylistSheet::new(playlist, Some(on_dismiss), cx))
                    .into_any_element()
            },
            |_: &mut Window, _: &mut App| Empty,
        )
        .snap_fractions(vec![0.72])
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .footer_shows_top_border(false)
        .fits_content(true)
        .sheet_background(Some(theme.bg._100));

        PageRefreshKeys::new(
            self.child_id("keys"),
            move |_, cx| load(&keys_playlist_id, true, cx),
            div().relative().size_full().child(shell).child(edit_sheet),
        )
    }
}
