use std::collections::HashMap;
use std::time::Duration;

use gpui::{AnyElement, App, Context, Entity, FontWeight, SharedString, Subscription, Task, Window, relative};
use rise_errors::ErrorCode;
use rise_theme::color;
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::ui::premium_diamond::PremiumDiamond;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::list_shell_floating_top_inset;
pub use crate::modules::chat::pages::chats::chat::chat_topics::member_tag_chip_hooks::MemberTagChip;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatPerm, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const RIGHTS_LOST_CLOSE_DELAY: Duration = Duration::from_millis(350);

pub(crate) fn chat_settings_symbol_is_filled(symbol: &str) -> bool {
    symbol.ends_with(".fill")
}

#[derive(IntoElement)]
pub struct ChatSettingsPlaceholder {
    icon: String,
    text: String,
}

impl ChatSettingsPlaceholder {
    pub fn new(icon: impl Into<String>, text: impl Into<String>) -> Self {
        Self { icon: icon.into(), text: text.into() }
    }
}

impl RenderOnce for ChatSettingsPlaceholder {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let top_inset = list_shell_floating_top_inset(cx);
        let is_filled = chat_settings_symbol_is_filled(&self.icon);
        div().size_full().flex().items_center().justify_center().pt(px(top_inset)).bg(theme.bg._100).child(
            vstack(12.0)
                .children(IconUi::sized(&self.icon, px(40.0), theme.text.secondary.opacity(0.6), is_filled))
                .child(
                    MainText::new(self.text)
                        .max_w(px(280.0))
                        .px(15.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary),
                ),
        )
    }
}

/// role of a member on a roster row
/// the row may carry a nameplate banner so the label sits on its own surface
/// mostly opaque with a hairline outline
#[derive(IntoElement)]
pub struct MemberRolePill {
    title: SharedString,
    color_hex: SharedString,
}

impl MemberRolePill {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), color_hex: SharedString::default() }
    }

    pub fn color_hex(mut self, color_hex: impl Into<SharedString>) -> Self {
        self.color_hex = color_hex.into();
        self
    }
}

impl RenderOnce for MemberRolePill {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let tint = if self.color_hex.is_empty() { theme.primary._100 } else { theme.legible(color(&self.color_hex)) };
        div()
            .flex_none()
            .max_w(relative(0.6))
            .px(px(8.0))
            .py(px(3.0))
            .overflow_hidden()
            .bg(theme.bg._100.opacity(0.88))
            .border_1()
            .border_color(tint.opacity(0.45))
            .rounded_full()
            .child(MainText::new(self.title).px(11.0).font_weight(FontWeight::SEMIBOLD).color(tint).number_of_lines(1))
    }
}

#[derive(IntoElement)]
pub struct ChatModerationRow {
    logo: String,
    name: String,
    subtitle: String,
    badge: Option<String>,
    online: bool,
    is_official: bool,
    is_premium: bool,
    user_id: String,
}

impl ChatModerationRow {
    pub fn new(
        logo: impl Into<String>,
        name: impl Into<String>,
        subtitle: impl Into<String>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            logo: logo.into(),
            name: name.into(),
            subtitle: subtitle.into(),
            badge: None,
            online: false,
            is_official: false,
            is_premium: false,
            user_id: user_id.into(),
        }
    }

    pub fn badge(mut self, badge: impl Into<Option<String>>) -> Self {
        self.badge = badge.into();
        self
    }

    pub fn online(mut self, online: bool) -> Self {
        self.online = online;
        self
    }

    pub fn is_official(mut self, is_official: bool) -> Self {
        self.is_official = is_official;
        self
    }

    pub fn is_premium(mut self, is_premium: bool) -> Self {
        self.is_premium = is_premium;
        self
    }
}

impl RenderOnce for ChatModerationRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let logo = UserLogoUi::new(SharedString::from(format!("chat-moderation-row-logo-{}", self.user_id)))
            .source(SharedString::from(self.logo))
            .name(SharedString::from(self.name.clone()))
            .user_id(SharedString::from(self.user_id))
            .size(40.0);
        hstack(10.0)
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .bg(theme.bg._100)
            .child(
                div().relative().flex_none().size(px(40.0)).child(logo).when(self.online, |avatar| {
                    avatar.child(
                        div()
                            .absolute()
                            .left(px(29.0))
                            .top(px(29.0))
                            .size(px(10.0))
                            .rounded_full()
                            .bg(theme.semantic.success_100)
                            .child(centred_stroke(theme.bg._100, 1.5, Shape::Circle)),
                    )
                }),
            )
            .child(
                vstack(2.0)
                    .min_w_0()
                    .align(HorizontalAlignment::Leading)
                    .child(
                        hstack(4.0)
                            .w_full()
                            .child(
                                div().min_w_0().child(
                                    MainText::new(self.name)
                                        .px(15.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(theme.text.primary)
                                        .number_of_lines(1),
                                ),
                            )
                            .when(self.is_official, |line| line.child(OfficialBadge::new().size(px(14.0))))
                            .when(self.is_premium, |line| line.child(PremiumDiamond::new().size(14.0)))
                            .when_some(self.badge, |line, badge| {
                                line.child(MainText::new(badge).px(11.0).color(theme.primary._100))
                            }),
                    )
                    .when(!self.subtitle.is_empty(), |column| {
                        column.child(
                            MainText::new(self.subtitle).px(12.0).color(theme.text.secondary).number_of_lines(1),
                        )
                    }),
            )
            .child(spacer_min_w(4.0))
    }
}

pub enum ChatSettingsAccess {}

impl ChatSettingsAccess {
    pub fn rights_lost(chat: &ChatInfo, cx: &mut App) -> bool {
        if (chat.r#type != ChatType::Group && chat.r#type != ChatType::Channel)
            || !ChatServicesStore::is_joined_member_role(chat.role.as_deref())
        {
            return false;
        }
        !ChatProfilePermissions::manages_chat(chat, cx)
            && !ChatProfilePermissions::can_edit_profile(chat, cx)
            && !ChatProfilePermissions::manages_reactions(chat, cx)
            && !ChatProfilePermissions::manages_forum_structure(chat, cx)
            && !ChatProfilePermissions::has(chat, ChatPerm::BAN_MEMBERS, cx)
            && !ChatProfilePermissions::has(chat, ChatPerm::VIEW_AUDIT_LOG, cx)
    }
}

/// closes the chat settings and every screen opened from them once the rights are gone
/// only the top screen acts since a covered one may not re render and two would close twice
#[derive(IntoElement)]
pub struct ChatSettingsRightsGuard {
    route: RootRoute,
    chat_id: Option<String>,
    content: AnyElement,
}

pub trait ChatSettingsRightsGuardModifiers: IntoElement + Sized {
    fn chat_settings_rights_guard(self, route: RootRoute, chat_id: Option<String>) -> ChatSettingsRightsGuard {
        ChatSettingsRightsGuard { route, chat_id, content: self.into_any_element() }
    }
}

impl<E: IntoElement> ChatSettingsRightsGuardModifiers for E {}

impl RenderOnce for ChatSettingsRightsGuard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let id = SharedString::from(format!("chat-settings-rights-guard-{:?}", self.route));
        let route = self.route;
        let chat_id = self.chat_id;
        window.use_keyed_state(id, cx, move |_, cx| ChatSettingsRightsGuardState::new(route, chat_id, nav_store, cx));
        self.content
    }
}

struct ChatSettingsRightsGuardState {
    route: RootRoute,
    chat_id: Option<String>,
    nav_store: Option<Entity<NavigationStore>>,
    close_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatSettingsRightsGuardState {
    fn new(
        route: RootRoute,
        chat_id: Option<String>,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let services = ChatServicesStore::shared(cx);
        let guard = cx.weak_entity();
        let mut subscriptions = vec![
            cx.observe(&services, |state, _, cx| state.evaluate(cx)),
            ChatServicesStore::subscribe_info_keys(chat_id.clone(), cx, move |cx| {
                let _ = guard.update(cx, |state, cx| state.evaluate(cx));
            }),
        ];
        if let Some(nav_store) = &nav_store {
            subscriptions.push(cx.observe(nav_store, |state, _, cx| state.evaluate(cx)));
        }
        let mut state = Self { route, chat_id, nav_store, close_task: None, _subscriptions: subscriptions };
        state.evaluate(cx);
        state
    }

    fn rights_lost(&self, cx: &mut App) -> bool {
        let Some(id) = self.chat_id.as_deref().filter(|id| !id.is_empty()) else {
            return false;
        };
        let services = ChatServicesStore::shared(cx);
        let Some(chat) = services.read(cx).chat_info(id, cx) else {
            return false;
        };
        ChatSettingsAccess::rights_lost(&chat, cx)
    }

    fn evaluate(&mut self, cx: &mut Context<Self>) {
        if self.close_task.is_none() && self.rights_lost(cx) {
            self.schedule_close(cx);
        }
    }

    /// the delay lets a running back transition finish, it would swallow the path change otherwise
    fn schedule_close(&mut self, cx: &mut Context<Self>) {
        self.close_task = Some(cx.spawn(async move |state, cx| {
            cx.background_executor().timer(RIGHTS_LOST_CLOSE_DELAY).await;
            let _ = state.update(cx, |state, cx| {
                state.close_task = None;
                if state.rights_lost(cx) {
                    state.close(cx);
                }
            });
        }));
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(nav_store)) = (self.chat_id.clone(), self.nav_store.clone()) else {
            return;
        };
        let settings = RootRoute::ChatSettings { chat_id: Some(id) };
        let path = nav_store.read(cx).path.clone();
        if path.last() != Some(&self.route) {
            return;
        }
        let Some(index) = path.iter().rposition(|route| *route == settings) else {
            return;
        };
        nav_store.update(cx, |nav_store, cx| {
            if index == 0 {
                nav_store.pop_to_root(cx);
            } else {
                nav_store.pop_to(path[index - 1].clone(), cx);
            }
        });
        if let Some(text) = CanonErrors::text(Some(ErrorCode::ChatPermissionDenied.wire()), &HashMap::new()) {
            let options = NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            };
            Notifier::send(text, options, cx);
        }
    }
}
