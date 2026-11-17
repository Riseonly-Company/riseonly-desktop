use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, SharedString, Subscription, Window};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState, PlaylistCollaborationUserLabel,
    PlaylistMemberStatusBadge, PlaylistPermissionToggleRow,
};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::{MusicPlaylistMember, MusicPlaylistMemberPermissions};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

struct PlaylistMemberRightsScreenState {
    can_edit_playlist: bool,
    can_add_tracks: bool,
    can_remove_tracks: bool,
    loaded_version: Option<i64>,
    saving: bool,
    removing: bool,
    _subscriptions: [Subscription; 3],
}

impl PlaylistMemberRightsScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = MusicServicesStore::shared(cx);
        let actions = MusicActionsStore::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        Self {
            can_edit_playlist: false,
            can_add_tracks: false,
            can_remove_tracks: false,
            loaded_version: None,
            saving: false,
            removing: false,
            _subscriptions: [
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&sandbox, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn current_permissions(&self) -> MusicPlaylistMemberPermissions {
        MusicPlaylistMemberPermissions {
            can_edit_playlist: self.can_edit_playlist,
            can_add_tracks: self.can_add_tracks,
            can_remove_tracks: self.can_remove_tracks,
        }
    }
}

/// what the actions of the screen need
#[derive(Clone)]
struct PlaylistMemberRightsScreenParts {
    playlist_id: String,
    member_user_id: String,
    state: Entity<PlaylistMemberRightsScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PlaylistMemberRightsScreenParts {
    fn member(&self, cx: &mut App) -> Option<MusicPlaylistMember> {
        MusicServicesStore::shared(cx)
            .update(cx, |services, cx| services.playlist_members(&self.playlist_id, true, cx))
            .into_iter()
            .find(|member| member.user_id == self.member_user_id)
    }

    fn can_manage(&self, member: Option<&MusicPlaylistMember>, cx: &mut App) -> bool {
        let settings =
            MusicServicesStore::shared(cx).update(cx, |services, cx| services.playlist_settings(&self.playlist_id, cx));
        settings.is_some_and(|settings| settings.is_owner()) && !member.is_some_and(MusicPlaylistMember::is_owner)
    }

    fn has_changes(&self, member: Option<&MusicPlaylistMember>, cx: &App) -> bool {
        member.is_some_and(|member| self.state.read(cx).current_permissions() != member.permissions)
    }

    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn load(&self, force_refresh: bool, cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_playlist_settings(&self.playlist_id, force_refresh, cx);
            interactions.load_playlist_members(&self.playlist_id, true, force_refresh, cx);
        });
    }

    fn hydrate_from_member(&self, cx: &mut App) {
        let Some(member) = self.member(cx) else {
            return;
        };
        self.state.update(cx, |state, cx| {
            if state.loaded_version == Some(member.version) {
                return;
            }
            state.loaded_version = Some(member.version);
            state.can_edit_playlist = member.permissions.can_edit_playlist;
            state.can_add_tracks = member.permissions.can_add_tracks;
            state.can_remove_tracks = member.permissions.can_remove_tracks;
            cx.notify();
        });
    }

    fn save(&self, cx: &mut App) {
        let Some(member) = self.member(cx) else {
            return;
        };
        if !self.can_manage(Some(&member), cx) || !self.has_changes(Some(&member), cx) || self.state.read(cx).saving {
            return;
        }
        self.state.update(cx, |state, cx| {
            state.saving = true;
            cx.notify();
        });
        let permissions = self.state.read(cx).current_permissions();
        let success_state = self.state.clone();
        let failure = self.clone();
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.update_playlist_member_permissions(
                &self.playlist_id,
                &member.user_id,
                permissions,
                Some(member.version),
                Some(Rc::new(move |_: MusicPlaylistMember, cx: &mut App| {
                    success_state.update(cx, |state, cx| {
                        state.saving = false;
                        cx.notify();
                    });
                    notification(NotificationKind::Success);
                })),
                Some(Rc::new(move |error: String, cx: &mut App| {
                    failure.state.update(cx, |state, cx| {
                        state.saving = false;
                        cx.notify();
                    });
                    failure.load(true, cx);
                    Notifier::send(
                        tr("music_playlist_member_save_error"),
                        NotifierSendOptions {
                            content: Some(error),
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                })),
                cx,
            );
        });
    }

    fn confirm_removal(&self, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("music_playlist_member_remove_title"),
            &tr("music_playlist_member_remove_message"),
            Some(tr("music_playlist_member_remove_confirm")),
            None,
            false,
            None,
            None,
            move |cx| parts.remove(cx),
            cx,
        );
    }

    fn remove(&self, cx: &mut App) {
        let Some(member) = self.member(cx) else {
            return;
        };
        if !self.can_manage(Some(&member), cx) || self.state.read(cx).removing {
            return;
        }
        self.state.update(cx, |state, cx| {
            state.removing = true;
            cx.notify();
        });
        let success = self.clone();
        let error_state = self.state.clone();
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.remove_playlist_member(
                &self.playlist_id,
                &member.user_id,
                Some(Rc::new(move |cx: &mut App| {
                    success.state.update(cx, |state, cx| {
                        state.removing = false;
                        cx.notify();
                    });
                    notification(NotificationKind::Success);
                    success.pop(cx);
                })),
                Some(Rc::new(move |error: String, cx: &mut App| {
                    error_state.update(cx, |state, cx| {
                        state.removing = false;
                        cx.notify();
                    });
                    Notifier::send(
                        tr("music_playlist_load_error"),
                        NotifierSendOptions {
                            content: Some(error),
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                })),
                cx,
            );
        });
    }
}

#[derive(IntoElement)]
pub struct PlaylistMemberRightsScreen {
    playlist_id: String,
    member_user_id: String,
}

impl PlaylistMemberRightsScreen {
    pub fn new(playlist_id: impl Into<String>, member_user_id: impl Into<String>) -> Self {
        Self { playlist_id: playlist_id.into(), member_user_id: member_user_id.into() }
    }
}

impl RenderOnce for PlaylistMemberRightsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let screen_id = format!("playlist-member-rights-{}-{}", self.playlist_id, self.member_user_id);
        let state = window.use_keyed_state(SharedString::from(format!("{screen_id}-state")), cx, |_, cx| {
            PlaylistMemberRightsScreenState::new(cx)
        });
        let parts = PlaylistMemberRightsScreenParts {
            playlist_id: self.playlist_id,
            member_user_id: self.member_user_id,
            state,
            nav_store: env::<NavigationStoreKey>(cx),
        };
        let theme = rise_ui::theme(cx).clone();
        let member = parts.member(cx);
        let can_manage = parts.can_manage(member.as_ref(), cx);
        let has_changes = parts.has_changes(member.as_ref(), cx);

        let hydrating = parts.clone();
        on_change(
            SharedString::from(format!("{screen_id}-version")),
            member.as_ref().map(|member| member.version),
            true,
            window,
            cx,
            move |_, _, _, cx| hydrating.hydrate_from_member(cx),
        );
        on_change(SharedString::from(format!("{screen_id}-dirty")), has_changes, true, window, cx, |_, dirty, _, cx| {
            GuideFacts::set(RiseGuideSignal::PlaylistMemberRightsDirty, *dirty, cx);
        });

        let back = parts.clone();
        let mut wrapper = SettingsWrapperUi::new(tr("music_playlist_member_rights_title"), move |_, cx| back.pop(cx));
        if let Some(action) = save_action(member.is_some(), can_manage, has_changes, &parts, &theme, cx) {
            wrapper = wrapper.right_jsx(action);
        }
        let appearing = parts.clone();
        wrapper
            .content(PlaylistMemberRightsScreenBody { parts, member, can_manage })
            .on_appear(SharedString::from(format!("{screen_id}-task")), move |_, cx| appearing.load(false, cx))
            .and_on_disappear(|_, cx| GuideFacts::set(RiseGuideSignal::PlaylistMemberRightsDirty, false, cx))
    }
}

fn save_action(
    has_member: bool,
    can_manage: bool,
    has_changes: bool,
    parts: &PlaylistMemberRightsScreenParts,
    theme: &AppTheme,
    cx: &App,
) -> Option<AnyElement> {
    if !has_member || !can_manage {
        return None;
    }
    if parts.state.read(cx).saving {
        return Some(progress_view().tint(theme.primary._100).into_any_element());
    }
    let saving = parts.clone();
    Some(
        PageHeaderIconButton::new("playlist-member-save", "checkmark", move |_, cx| saving.save(cx))
            .disabled(!has_changes)
            .guide_anchor(RiseGuideAnchor::PlaylistMemberSave)
            .into_any_element(),
    )
}

#[derive(IntoElement)]
struct PlaylistMemberRightsScreenBody {
    parts: PlaylistMemberRightsScreenParts,
    member: Option<MusicPlaylistMember>,
    can_manage: bool,
}

impl RenderOnce for PlaylistMemberRightsScreenBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let content = match self.member {
            Some(member) => member_content(&member, self.can_manage, &self.parts, &theme, cx).into_any_element(),
            None => {
                let error = MusicActionsStore::slices(cx).get_playlist_members_sai.error();
                if error.is_some_and(|error| !error.is_empty()) {
                    let retry = self.parts.clone();
                    let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| retry.load(false, cx));
                    PlaylistCollaborationErrorState::new(
                        "playlist-member-rights-error",
                        tr("music_playlist_load_error"),
                        Some(retry),
                    )
                    .into_any_element()
                } else {
                    PlaylistCollaborationLoadingState::new().into_any_element()
                }
            }
        };
        scroll_view("playlist-member-rights-scroll")
            .list_shell_scroll_under_chrome_resetting(true, cx)
            .bg(theme.bg._100)
            .child(div().w_full().pt(px(18.0)).pb(px(90.0)).child(content))
    }
}

fn member_content(
    member: &MusicPlaylistMember,
    can_manage: bool,
    parts: &PlaylistMemberRightsScreenParts,
    theme: &AppTheme,
    cx: &App,
) -> impl IntoElement {
    let opening = parts.clone();
    let opened_user = member.user.clone();
    let (saving, removing) = {
        let state = parts.state.read(cx);
        (state.saving, state.removing)
    };

    let mut content = vstack(20.0).w_full().child(
        plain("playlist-member-rights-profile")
            .w_full()
            .on_press(move |_, cx| {
                if let Some(nav_store) = &opening.nav_store {
                    let route = RootRoute::UserPage { user_id: Some(opened_user.id.clone()), tag: Some(opened_user.tag.clone()) };
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .child(member_header(member, theme)),
    );
    content = content.child(permissions_section(can_manage, saving, parts, theme));

    if can_manage {
        let confirming = parts.clone();
        let error_color = theme.semantic.error_100;
        let leading = if removing {
            progress_view().tint(error_color).into_any_element()
        } else {
            div()
                .children(IconUi::sized("person.crop.circle.badge.minus", px(17.0), error_color, false))
                .into_any_element()
        };
        content = content.child(
            div().w_full().px(px(12.0)).child(
                plain("playlist-member-rights-remove")
                    .disabled(removing || saving)
                    .on_press(move |_, cx| confirming.confirm_removal(cx))
                    .child(
                        hstack(9.0)
                            .w_full()
                            .min_h(px(50.0))
                            .justify_center()
                            .bg(theme.bg._200)
                            .capsule()
                            .child(leading)
                            .child(
                                MainText::new(tr("music_playlist_member_remove"))
                                    .px(15.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(error_color),
                            ),
                    ),
            ),
        );
    }
    content
}

fn member_header(member: &MusicPlaylistMember, theme: &AppTheme) -> impl IntoElement {
    let user = &member.user;
    let name_color = user
        .is_premium
        .then(|| PremiumNameColor::parse(user.name_color.as_deref()))
        .flatten()
        .unwrap_or(theme.text.primary);
    let mut name = hstack(6.0).child(
        MainText::new(PlaylistCollaborationUserLabel::display_name(user))
            .px(19.0)
            .font_weight(FontWeight::BOLD)
            .color(name_color)
            .number_of_lines(1),
    );
    if user.is_premium {
        name = name.child(
            PremiumBadge::new()
                .user_id(Some(SharedString::from(user.id.clone())))
                .emoji(user.premium_badge_emoji.clone().map(SharedString::from))
                .size(16.0),
        );
    }
    let mut header = vstack(10.0)
        .w_full()
        .px(px(16.0))
        .child(
            UserLogoUi::new(SharedString::from(format!("playlist-member-rights-logo-{}", user.id)))
                .source(SharedString::from(user.logo.clone()))
                .name(SharedString::from(user.name.clone()))
                .user_id(SharedString::from(user.id.clone()))
                .size(76.0),
        )
        .child(name);
    if !user.tag.is_empty() {
        header = header.child(MainText::new(format!("@{}", user.tag)).px(13.0).color(theme.text.secondary));
    }
    header.child(PlaylistMemberStatusBadge::new(member.role.clone(), member.status.clone()))
}

fn permissions_section(
    can_manage: bool,
    saving: bool,
    parts: &PlaylistMemberRightsScreenParts,
    theme: &AppTheme,
) -> impl IntoElement {
    let disabled = !can_manage || saving;
    let state = parts.state.clone();
    let card = vstack(0.0)
        .w_full()
        .align(HorizontalAlignment::Leading)
        .bg(theme.bg._200)
        .rounded_rect(20.0)
        .child(
            PlaylistPermissionToggleRow::new(
                "playlist-permission-edit",
                tr("music_playlist_permission_edit"),
                tr("music_playlist_permission_edit_hint"),
                Binding::entity(
                    state.clone(),
                    |state: &PlaylistMemberRightsScreenState| state.can_edit_playlist,
                    |state, value, _| state.can_edit_playlist = value,
                ),
            )
            .disabled(disabled),
        )
        .child(separator(theme))
        .child(
            PlaylistPermissionToggleRow::new(
                "playlist-permission-add",
                tr("music_playlist_permission_add"),
                tr("music_playlist_permission_add_hint"),
                Binding::entity(
                    state.clone(),
                    |state: &PlaylistMemberRightsScreenState| state.can_add_tracks,
                    |state, value, _| state.can_add_tracks = value,
                ),
            )
            .disabled(disabled),
        )
        .child(separator(theme))
        .child(
            PlaylistPermissionToggleRow::new(
                "playlist-permission-remove",
                tr("music_playlist_permission_remove"),
                tr("music_playlist_permission_remove_hint"),
                Binding::entity(
                    state,
                    |state: &PlaylistMemberRightsScreenState| state.can_remove_tracks,
                    |state, value, _| state.can_remove_tracks = value,
                ),
            )
            .disabled(disabled),
        );
    vstack(8.0)
        .w_full()
        .align(HorizontalAlignment::Leading)
        .child(
            div().px(px(16.0)).child(
                MainText::new(tr("music_playlist_member_permissions_section"))
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.secondary),
            ),
        )
        .child(div().w_full().px(px(12.0)).child(card.guide_anchor(RiseGuideAnchor::PlaylistMemberPermissions)))
}

fn separator(theme: &AppTheme) -> impl IntoElement {
    div().w_full().pl(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
}
