use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, FontWeight, Pixels, Point, SharedString, Subscription, Task, Window};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::system_share::ShareItem;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::page_refresh::PageRefreshKeys;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::share_sheet::SystemShare;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState,
};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistInviteLink;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

struct PlaylistInviteLinkScreenState {
    _subscriptions: [Subscription; 3],
}

impl PlaylistInviteLinkScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        Self {
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
            ],
        }
    }
}

fn load(playlist_id: &str, force_refresh: bool, cx: &mut App) {
    MusicInteractionsStore::shared(cx)
        .update(cx, |store, cx| store.load_playlist_invite_link(playlist_id, force_refresh, cx));
}

fn copy(playlist_id: &str, value: &str, cx: &mut App) {
    UIPasteboard::general().set_string(value, cx);
    MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.note_playlist_invite_link_copied(playlist_id, cx));
    let _ = ImpactFeedbackGenerator::new(ImpactStyle::Light).impact_occurred();
    Notifier::send(
        tr("music_playlist_invite_copied"),
        NotifierSendOptions {
            r#type: NotifierType::Success,
            position: NotifierPosition::Bottom,
            ..NotifierSendOptions::default()
        },
        cx,
    );
}

fn invite_share_url(invite: &MusicPlaylistInviteLink) -> String {
    RiseonlyPublicLink::Playlist { reference: invite.token.clone() }
        .web_url(AppEnvironment::current())
        .map(|url| url.absolute_string().to_owned())
        .unwrap_or_else(|| invite.url.clone())
}

fn action_label(icon: &'static str, title: String, theme: &AppTheme) -> Div {
    hstack(7.0)
        .w_full()
        .min_h(px(46.0))
        .justify_center()
        .bg(theme.primary._100)
        .capsule()
        .children(IconUi::sized(icon, px(15.0), theme.content_on(theme.primary._100), false))
        .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100)))
}

#[derive(IntoElement)]
struct PlaylistInviteLinkScroll {
    id: ElementId,
    playlist_id: String,
    content: AnyElement,
}

impl RenderOnce for PlaylistInviteLinkScroll {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let scroll_disabled = HoldContextMenuPresenter::has_active_request(cx);
        let playlist_id = self.playlist_id;
        scroll_view(self.id)
            .scroll_disabled(scroll_disabled)
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
pub struct PlaylistInviteLinkScreen {
    id: ElementId,
    playlist_id: String,
}

impl PlaylistInviteLinkScreen {
    pub fn new(playlist_id: impl Into<String>) -> Self {
        let playlist_id = playlist_id.into();
        Self { id: ElementId::Name(SharedString::from(format!("playlist-invite-link-{playlist_id}"))), playlist_id }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn invite_content(&self, invite: &MusicPlaylistInviteLink, theme: &AppTheme) -> Div {
        let url = invite_share_url(invite);

        let action_playlist_id = self.playlist_id.clone();
        let action_url = url.clone();
        let tap_playlist_id = self.playlist_id.clone();
        let tap_url = url.clone();
        let copy_playlist_id = self.playlist_id.clone();
        let copy_url = url.clone();
        let share_url = url.clone();
        let source_theme = theme.clone();

        let hold_source = HoldContextMenuContainer::new(
            self.child_id("hold"),
            HoldContextMenuAlignment::Leading,
            Rc::new(|_: &App| {
                vec![HoldContextMenuAction::new("copy", tr("music_playlist_invite_copy")).system_image("doc.on.doc")]
            }),
            Rc::new(move |_: &mut Window, _: &mut App| {
                div()
                    .w_full()
                    .px(px(12.0))
                    .child(
                        div()
                            .w_full()
                            .min_h(px(50.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .px(px(14.0))
                            .bg(source_theme.bg._200)
                            .rounded_rect(16.0)
                            .child(
                                MainText::new(url.clone())
                                    .px(15.0)
                                    .tac(TextAlignT::Center)
                                    .color(source_theme.text.primary)
                                    .number_of_lines(2),
                            ),
                    )
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |_: String, _: &mut Window, cx: &mut App| copy(&action_playlist_id, &action_url, cx)))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| copy(&tap_playlist_id, &tap_url, cx)));

        vstack(22.0)
            .w_full()
            .child(
                vstack(10.0)
                    .w_full()
                    .children(IconUi::sized("link.circle.fill", px(58.0), theme.primary._100, true))
                    .child(
                        MainText::new(tr("music_playlist_invite_secure"))
                            .px(17.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary),
                    )
                    .child(
                        div().px(px(24.0)).child(
                            MainText::new(tr("music_playlist_invite_hint"))
                                .px(14.0)
                                .tac(TextAlignT::Center)
                                .color(theme.text.secondary)
                                .number_of_lines(4),
                        ),
                    ),
            )
            .child(div().w_full().child(hold_source))
            .child(
                hstack(10.0)
                    .w_full()
                    .px(px(12.0))
                    .child(
                        div().flex_1().min_w_0().child(
                            plain(self.child_id("copy"))
                                .w_full()
                                .on_press(move |_, cx| copy(&copy_playlist_id, &copy_url, cx))
                                .child(action_label("doc.on.doc", tr("music_playlist_invite_copy"), theme))
                                .guide_anchor(RiseGuideAnchor::InviteLinkCopy),
                        ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            plain(self.child_id("share"))
                                .w_full()
                                .on_press(move |_, cx| {
                                    SystemShare::present(vec![ShareItem::Link(share_url.clone())], None, cx);
                                })
                                .child(action_label("square.and.arrow.up", tr("music_playlist_invite_share"), theme)),
                        ),
                    ),
            )
    }
}

impl RenderOnce for PlaylistInviteLinkScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let _state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| PlaylistInviteLinkScreenState::new(cx));

        let task_playlist_id = self.playlist_id.clone();
        use_task(self.child_id("task"), (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| load(&task_playlist_id, false, cx));
        });

        let theme = rise_ui::theme(cx).clone();
        let invite = MusicServicesStore::shared(cx).update(cx, |services, cx| services.playlist_invite_link(&self.playlist_id, cx));
        let load_error = MusicActionsStore::slices(cx)
            .get_playlist_invite_link_sai
            .error()
            .filter(|error| !error.is_empty());

        let body = if let Some(invite) = invite.as_ref() {
            self.invite_content(invite, &theme).into_any_element()
        } else if load_error.is_some() {
            let retry_playlist_id = self.playlist_id.clone();
            let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| load(&retry_playlist_id, false, cx));
            PlaylistCollaborationErrorState::new(self.child_id("error"), tr("music_playlist_load_error"), Some(retry))
                .into_any_element()
        } else {
            PlaylistCollaborationLoadingState::new().into_any_element()
        };

        let keys_playlist_id = self.playlist_id.clone();
        let shell = SettingsWrapperUi::new(tr("music_playlist_invite_title"), move |_, cx| {
            if let Some(nav_store) = nav_store.as_ref() {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(PlaylistInviteLinkScroll {
            id: self.child_id("scroll"),
            playlist_id: self.playlist_id.clone(),
            content: div().w_full().pt(px(18.0)).pb(px(80.0)).child(body).into_any_element(),
        });

        PageRefreshKeys::new(self.child_id("keys"), move |_, cx| load(&keys_playlist_id, true, cx), shell)
    }
}
