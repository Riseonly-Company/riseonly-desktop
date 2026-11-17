use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, Hsla, SharedString, Subscription, Window};
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_canon::rise_proto::stickers::ProtoPackCoauthor;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState, PlaylistPermissionToggleRow,
};
use crate::modules::stickers::pages::pack_collaboration::pack_collaboration_components::PackCollaborationUserLabel;
use crate::modules::stickers::engine::rise_pack_collaboration_repository::RisePackRights;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_coauthors_interactions::PackCollaborationFailureCallback;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;


struct PackCoauthorRightsScreenState {
    rights: RisePackRights,
    hydrated_from: Option<RisePackRights>,
    saving: bool,
    removing: bool,
    transferring: bool,
    load_failure: Option<String>,
    _subscriptions: [Subscription; 2],
}

impl PackCoauthorRightsScreenState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = StickersServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        Self {
            rights: RisePackRights::default(),
            hydrated_from: None,
            saving: false,
            removing: false,
            transferring: false,
            load_failure: None,
            _subscriptions: [cx.observe(&services, |_, _, cx| cx.notify()), cx.observe(&auth, |_, _, cx| cx.notify())],
        }
    }
}

/// what the actions of the screen need
#[derive(Clone)]
struct PackCoauthorRightsScreenParts {
    pack_id: String,
    user_id: String,
    state: Entity<PackCoauthorRightsScreenState>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PackCoauthorRightsScreenParts {
    fn coauthor(&self, cx: &mut App) -> Option<ProtoPackCoauthor> {
        StickersServicesStore::shared(cx).read(cx).pack_coauthor(&self.pack_id, &self.user_id)
    }

    fn stored_rights(coauthor: Option<&ProtoPackCoauthor>) -> RisePackRights {
        coauthor.and_then(|coauthor| coauthor.rights.clone()).unwrap_or_default()
    }

    fn can_manage(&self, coauthor: Option<&ProtoPackCoauthor>, cx: &mut App) -> bool {
        let Some(current_user_id) = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire() else {
            return false;
        };
        if coauthor.is_some_and(|coauthor| coauthor.is_owner) {
            return false;
        }
        StickersServicesStore::shared(cx)
            .read(cx)
            .pack_coauthor(&self.pack_id, &current_user_id)
            .is_some_and(|me| me.is_owner)
    }

    fn has_changes(&self, coauthor: Option<&ProtoPackCoauthor>, cx: &App) -> bool {
        coauthor.is_some() && self.state.read(cx).rights != Self::stored_rights(coauthor)
    }

    fn pop(&self, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }

    fn open_profile(&self, coauthor: &ProtoPackCoauthor, cx: &mut App) {
        let Some(nav_store) = &self.nav_store else {
            return;
        };
        let tag = coauthor.user.as_ref().map(|user| user.tag.clone()).unwrap_or_default();
        let route = RootRoute::UserPage { user_id: Some(coauthor.user_id.clone()), tag: Some(tag) };
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }

    fn load(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.load_failure = None;
            cx.notify();
        });
        let failure_state = self.state.clone();
        let on_failure: PackCollaborationFailureCallback = Box::new(move |error: String, cx: &mut App| {
            failure_state.update(cx, |state, cx| {
                state.load_failure = Some(error);
                cx.notify();
            });
        });
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_coauthors(&self.pack_id, false, false, Some(on_failure), cx);
        });
    }

    fn hydrate(&self, cx: &mut App) {
        let coauthor = self.coauthor(cx);
        if coauthor.is_none() {
            return;
        }
        let stored = Self::stored_rights(coauthor.as_ref());
        self.state.update(cx, |state, cx| {
            if state.hydrated_from.as_ref().is_none_or(|hydrated| *hydrated == state.rights) {
                state.rights = stored.clone();
            }
            state.hydrated_from = Some(stored);
            cx.notify();
        });
    }

    fn save(&self, cx: &mut App) {
        let coauthor = self.coauthor(cx);
        if !self.has_changes(coauthor.as_ref(), cx) || self.state.read(cx).saving {
            return;
        }
        let draft = self.state.update(cx, |state, cx| {
            state.saving = true;
            cx.notify();
            state.rights.clone()
        });
        let success_state = self.state.clone();
        let saved = draft.clone();
        let failure = self.clone();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.update_pack_coauthor_rights(
                &self.pack_id,
                &self.user_id,
                draft,
                move |cx: &mut App| {
                    success_state.update(cx, |state, cx| {
                        state.saving = false;
                        state.hydrated_from = Some(saved.clone());
                        cx.notify();
                    });
                    notification(NotificationKind::Success);
                },
                move |error: String, cx: &mut App| failure.failed(error, cx),
                cx,
            );
        });
    }

    fn confirm_removal(&self, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("sticker_pack_coauthor_remove"),
            &tr("sticker_pack_coauthor_remove_confirm"),
            Some(tr("sticker_pack_coauthor_remove")),
            None,
            false,
            None,
            None,
            move |cx| parts.remove(cx),
            cx,
        );
    }

    fn confirm_transfer(&self, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("sticker_pack_transfer_ownership_confirm_title"),
            &tr("sticker_pack_transfer_ownership_confirm_message"),
            Some(tr("sticker_pack_transfer_ownership")),
            None,
            false,
            None,
            None,
            move |cx| parts.transfer(cx),
            cx,
        );
    }

    fn remove(&self, cx: &mut App) {
        if self.state.read(cx).removing {
            return;
        }
        self.state.update(cx, |state, cx| {
            state.removing = true;
            cx.notify();
        });
        let success = self.clone();
        let failure = self.clone();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.remove_pack_coauthor(
                &self.pack_id,
                &self.user_id,
                move |cx: &mut App| {
                    success.state.update(cx, |state, cx| {
                        state.removing = false;
                        cx.notify();
                    });
                    success.pop(cx);
                },
                move |error: String, cx: &mut App| failure.failed(error, cx),
                cx,
            );
        });
    }

    fn transfer(&self, cx: &mut App) {
        if self.state.read(cx).transferring {
            return;
        }
        self.state.update(cx, |state, cx| {
            state.transferring = true;
            cx.notify();
        });
        let success = self.clone();
        let failure = self.clone();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.transfer_pack_ownership(
                &self.pack_id,
                &self.user_id,
                move |cx: &mut App| {
                    success.state.update(cx, |state, cx| {
                        state.transferring = false;
                        cx.notify();
                    });
                    notification(NotificationKind::Success);
                    let ignore: PackCollaborationFailureCallback = Box::new(|_: String, _: &mut App| {});
                    StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.load_pack_coauthors(&success.pack_id, true, false, Some(ignore), cx);
                    });
                    success.pop(cx);
                },
                move |error: String, cx: &mut App| failure.failed(error, cx),
                cx,
            );
        });
    }

    fn failed(&self, error: String, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.saving = false;
            state.removing = false;
            state.transferring = false;
            cx.notify();
        });
        Notifier::send(
            tr("sticker_pack_request_error"),
            NotifierSendOptions {
                content: Some(error),
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }
}

/// rights of one coauthor, four switches saved by the header check and removal at the bottom
/// only the owner edits, everyone else reads
#[derive(IntoElement)]
pub struct PackCoauthorRightsScreen {
    pack_id: String,
    user_id: String,
}

impl PackCoauthorRightsScreen {
    pub fn new(pack_id: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self { pack_id: pack_id.into(), user_id: user_id.into() }
    }
}

impl RenderOnce for PackCoauthorRightsScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let screen_id = format!("pack-coauthor-rights-{}-{}", self.pack_id, self.user_id);
        let state = window.use_keyed_state(SharedString::from(format!("{screen_id}-state")), cx, |_, cx| {
            PackCoauthorRightsScreenState::new(cx)
        });
        let parts = PackCoauthorRightsScreenParts {
            pack_id: self.pack_id,
            user_id: self.user_id,
            state,
            nav_store: env::<NavigationStoreKey>(cx),
        };
        let theme = rise_ui::theme(cx).clone();
        let coauthor = parts.coauthor(cx);
        let can_manage = parts.can_manage(coauthor.as_ref(), cx);
        let has_changes = parts.has_changes(coauthor.as_ref(), cx);

        let hydrating = parts.clone();
        on_change(
            SharedString::from(format!("{screen_id}-rights")),
            coauthor.as_ref().map(|coauthor| coauthor.rights.clone()),
            true,
            window,
            cx,
            move |_, _, _, cx| hydrating.hydrate(cx),
        );

        let back = parts.clone();
        let mut wrapper = SettingsWrapperUi::new(tr("sticker_pack_rights_title"), move |_, cx| back.pop(cx));
        if let Some(action) = save_action(coauthor.is_some(), can_manage, has_changes, &parts, &theme, cx) {
            wrapper = wrapper.right_jsx(action);
        }
        let appearing = parts.clone();
        wrapper
            .content(PackCoauthorRightsScreenBody { parts, coauthor, can_manage })
            .on_appear(SharedString::from(format!("{screen_id}-task")), move |_, cx| appearing.load(cx))
    }
}

fn save_action(
    has_coauthor: bool,
    can_manage: bool,
    has_changes: bool,
    parts: &PackCoauthorRightsScreenParts,
    theme: &AppTheme,
    cx: &App,
) -> Option<AnyElement> {
    if !has_coauthor || !can_manage {
        return None;
    }
    if parts.state.read(cx).saving {
        return Some(progress_view().tint(theme.primary._100).into_any_element());
    }
    let saving = parts.clone();
    Some(
        PageHeaderIconButton::new("pack-coauthor-rights-save", "checkmark", move |_, cx| saving.save(cx))
            .disabled(!has_changes)
            .into_any_element(),
    )
}

#[derive(IntoElement)]
struct PackCoauthorRightsScreenBody {
    parts: PackCoauthorRightsScreenParts,
    coauthor: Option<ProtoPackCoauthor>,
    can_manage: bool,
}

impl RenderOnce for PackCoauthorRightsScreenBody {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let load_failure = self.parts.state.read(cx).load_failure.clone();
        let content = match (self.coauthor, load_failure) {
            (Some(coauthor), _) => coauthor_content(&coauthor, self.can_manage, &self.parts, &theme, cx).into_any_element(),
            (None, Some(failure)) => {
                let retry = self.parts.clone();
                let retry: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| retry.load(cx));
                PlaylistCollaborationErrorState::new("pack-coauthor-rights-error", failure, Some(retry)).into_any_element()
            }
            (None, None) => PlaylistCollaborationLoadingState::new().into_any_element(),
        };
        scroll_view("pack-coauthor-rights-scroll")
            .list_shell_scroll_under_chrome_resetting(true, cx)
            .bg(theme.bg._100)
            .child(div().w_full().pt(px(18.0)).pb(px(90.0)).child(content))
    }
}

fn coauthor_content(
    coauthor: &ProtoPackCoauthor,
    can_manage: bool,
    parts: &PackCoauthorRightsScreenParts,
    theme: &AppTheme,
    cx: &App,
) -> impl IntoElement {
    let opening = parts.clone();
    let opened = coauthor.clone();
    let (saving, removing, transferring) = {
        let state = parts.state.read(cx);
        (state.saving, state.removing, state.transferring)
    };

    let mut content = vstack(20.0).w_full().child(
        plain("pack-coauthor-rights-profile")
            .on_press(move |_, cx| opening.open_profile(&opened, cx))
            .child(coauthor_header(coauthor, theme)),
    );
    content = content.child(rights_section(can_manage, saving, parts, theme));

    if can_manage {
        let busy = transferring || removing || saving;
        let transferring_parts = parts.clone();
        content = content.child(capsule_button(
            "pack-coauthor-rights-transfer",
            "crown",
            tr("sticker_pack_transfer_ownership"),
            theme.primary._100,
            transferring,
            busy,
            Rc::new(move |_: &mut Window, cx: &mut App| transferring_parts.confirm_transfer(cx)),
            theme,
        ));
        let removing_parts = parts.clone();
        content = content.child(capsule_button(
            "pack-coauthor-rights-remove",
            "person.crop.circle.badge.minus",
            tr("sticker_pack_coauthor_remove"),
            theme.semantic.error_100,
            removing,
            busy,
            Rc::new(move |_: &mut Window, cx: &mut App| removing_parts.confirm_removal(cx)),
            theme,
        ));
    }
    content
}

/// transfer and removal capsules of the owner,
/// transfer sits here because people looked for it on the rights page
#[allow(clippy::too_many_arguments)]
fn capsule_button(
    id: &'static str,
    icon: &'static str,
    title: String,
    color: Hsla,
    in_progress: bool,
    disabled: bool,
    action: ButtonAction,
    theme: &AppTheme,
) -> impl IntoElement {
    let leading = if in_progress {
        progress_view().tint(color).into_any_element()
    } else {
        div().children(IconUi::sized(icon, px(17.0), color, false)).into_any_element()
    };
    div().w_full().px(px(12.0)).child(
        plain(id).disabled(disabled).on_press(move |window, cx| action(window, cx)).child(
            hstack(9.0)
                .w_full()
                .min_h(px(50.0))
                .justify_center()
                .bg(theme.bg._200)
                .capsule()
                .child(leading)
                .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD).color(color)),
        ),
    )
}

fn coauthor_header(coauthor: &ProtoPackCoauthor, theme: &AppTheme) -> impl IntoElement {
    let (logo, name) =
        coauthor.user.as_ref().map_or((String::new(), String::new()), |user| (user.logo.clone(), user.name.clone()));
    vstack(10.0)
        .w_full()
        .px(px(16.0))
        .child(
            UserLogoUi::new(SharedString::from(format!("pack-coauthor-rights-logo-{}", coauthor.user_id)))
                .source(SharedString::from(logo))
                .name(SharedString::from(name))
                .user_id(SharedString::from(coauthor.user_id.clone()))
                .size(76.0),
        )
        .child(
            MainText::new(PackCollaborationUserLabel::display_name(coauthor.user.as_ref()))
                .px(19.0)
                .font_weight(FontWeight::BOLD)
                .color(theme.text.primary)
                .number_of_lines(1),
        )
        .child(
            MainText::new(tr(if coauthor.is_owner { "sticker_pack_owner" } else { "sticker_pack_coauthor" }))
                .px(13.0)
                .color(theme.text.secondary),
        )
}

fn rights_section(
    can_manage: bool,
    saving: bool,
    parts: &PackCoauthorRightsScreenParts,
    theme: &AppTheme,
) -> impl IntoElement {
    let disabled = !can_manage || saving;
    let state = parts.state.clone();
    let card = vstack(0.0)
        .w_full()
        .align(HorizontalAlignment::Leading)
        .bg(theme.bg._200)
        .rounded_rect(20.0)
        .child(toggle(
            "sticker_pack_right_add",
            Binding::entity(
                state.clone(),
                |state: &PackCoauthorRightsScreenState| state.rights.can_add_stickers,
                |state, value, _| state.rights.can_add_stickers = value,
            ),
            disabled,
        ))
        .child(separator(theme))
        .child(toggle(
            "sticker_pack_right_edit",
            Binding::entity(
                state.clone(),
                |state: &PackCoauthorRightsScreenState| state.rights.can_edit_stickers,
                |state, value, _| state.rights.can_edit_stickers = value,
            ),
            disabled,
        ))
        .child(separator(theme))
        .child(toggle(
            "sticker_pack_right_delete",
            Binding::entity(
                state.clone(),
                |state: &PackCoauthorRightsScreenState| state.rights.can_delete_stickers,
                |state, value, _| state.rights.can_delete_stickers = value,
            ),
            disabled,
        ))
        .child(separator(theme))
        .child(toggle(
            "sticker_pack_right_edit_pack",
            Binding::entity(
                state,
                |state: &PackCoauthorRightsScreenState| state.rights.can_edit_pack,
                |state, value, _| state.rights.can_edit_pack = value,
            ),
            disabled,
        ));
    vstack(8.0)
        .w_full()
        .align(HorizontalAlignment::Leading)
        .child(
            div().px(px(16.0)).child(
                MainText::new(tr("sticker_pack_rights_title"))
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.text.secondary),
            ),
        )
        .child(div().w_full().px(px(12.0)).child(card))
}

fn toggle(key: &'static str, is_on: Binding<bool>, disabled: bool) -> impl IntoElement {
    PlaylistPermissionToggleRow::new(SharedString::from(format!("pack-right-{key}")), tr(key), tr(&format!("{key}_hint")), is_on)
        .disabled(disabled)
}

fn separator(theme: &AppTheme) -> impl IntoElement {
    div().w_full().pl(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
}
