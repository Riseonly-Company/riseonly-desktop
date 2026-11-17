use std::rc::Rc;

use gpui::{AnyElement, App, Context, Entity, FontWeight, Hsla, SharedString, Subscription, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::switch_ui::SwitchUi;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatPerm, ChatPermExt, ChatPermKey, ChatRole, ChatSettingsModel,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const SLOW_MODE_OPTIONS: [i64; 8] = [0, 5, 10, 30, 60, 300, 900, 3600];
const MEDIA_EXPAND_DURATION: f64 = 0.18;

type PermissionSetter = Rc<dyn Fn(&str, bool, &mut App)>;

fn slow_mode_label(seconds: i64) -> String {
    if seconds == 0 {
        return tr("chat_perm_slowmode_off");
    }
    if seconds < 60 {
        return tr_args("slowmode_seconds_abbr", &[FormatArg::Int(seconds)]);
    }
    if seconds < 3600 {
        return tr_args("slowmode_minutes_abbr", &[FormatArg::Int(seconds / 60)]);
    }
    tr_args("slowmode_hours_abbr", &[FormatArg::Int(seconds / 3600)])
}

fn media_label_key(key: &str) -> &str {
    match key {
        ChatPermKey::PHOTO => "chat_perm_media_photo",
        ChatPermKey::VIDEO => "chat_perm_media_video",
        ChatPermKey::STICKERS => "chat_perm_media_stickers",
        ChatPermKey::GIF => "chat_perm_media_gif",
        ChatPermKey::MUSIC => "chat_perm_media_music",
        ChatPermKey::FILES => "chat_perm_media_files",
        ChatPermKey::VOICE => "chat_perm_media_voice",
        ChatPermKey::VIDEO_MESSAGES => "chat_perm_media_video_messages",
        _ => key,
    }
}

fn divider(theme: &AppTheme, inset: f32) -> impl IntoElement {
    div().flex_none().w_full().pl(px(inset)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
}

fn toggle_row(
    id: impl Into<SharedString>,
    title: String,
    is_on: bool,
    indent: f32,
    theme: &AppTheme,
    set: impl Fn(bool, &mut App) + 'static,
) -> AnyElement {
    let title_color: Hsla = if indent > 0.0 { theme.text.primary.opacity(0.9) } else { theme.text.primary };
    let id: SharedString = id.into();
    let value = Binding::new(move |_| is_on, move |value, _, cx| set(value, cx));
    let toggled = value.clone();
    plain(id.clone())
        .w_full()
        .on_press(move |window, cx| toggled.toggle(window, cx))
        .child(
            hstack(10.0)
                .w_full()
                .pl(px(16.0 + indent))
                .pr(px(16.0))
                .py(px(8.0))
                .bg(theme.bg._200)
                .child(div().flex_1().min_w_0().child(MainText::new(title).px(15.0).color(title_color)))
                .child(SwitchUi::new(SharedString::from(format!("{id}-switch")), value)),
        )
        .into_any_element()
}

struct ChatSettingsPermissionsState {
    media_expanded: bool,
    media_rows: Presence<()>,
    _subscriptions: Vec<Subscription>,
}

impl ChatSettingsPermissionsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        Self {
            media_expanded: false,
            media_rows: Presence::new(None).with_transition(Transition::opacity()),
            _subscriptions: vec![cx.observe(&services, |_, _, cx| cx.notify())],
        }
    }

    fn toggle_media_expanded(&mut self, cx: &mut Context<Self>) {
        self.media_expanded = !self.media_expanded;
        let value = self.media_expanded.then_some(());
        self.media_rows.set_in(value, ease_in_out(MEDIA_EXPAND_DURATION), RiseClock::monotonic(), cx);
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct ChatSettingsPermissions {
    chat_id: Option<String>,
}

impl ChatSettingsPermissions {
    pub fn new(chat_id: Option<String>) -> Self {
        Self { chat_id }
    }

    /// default role of a topic group, topic mentions are a right on it
    fn topic_mention_role(resolved_chat_id: &str, cx: &mut App) -> Option<ChatRole> {
        ChatServicesStore::shared(cx).update(cx, |store, cx| {
            if !store.chat_info(resolved_chat_id, cx).is_some_and(|chat| chat.topics_enabled) {
                return None;
            }
            store.everyone_role(resolved_chat_id, cx)
        })
    }

    fn set_everyone_right(resolved_chat_id: &str, everyone: &ChatRole, bit: i64, allowed: bool, cx: &mut App) {
        let permissions = if allowed { everyone.permissions | bit } else { everyone.permissions & !bit };
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_role(
                resolved_chat_id,
                &everyone.id,
                &everyone.name,
                everyone.color,
                permissions,
                everyone.mentionable,
                everyone.hoist,
                None,
                cx,
            );
        });
    }

    fn media_parent_row(
        resolved_chat_id: &str,
        model: &ChatSettingsModel,
        state: &Entity<ChatSettingsPermissionsState>,
        expanded: bool,
        theme: &AppTheme,
    ) -> impl IntoElement {
        let enabled_count = model.media_enabled_count();
        let all_chat_id = resolved_chat_id.to_owned();
        let all_media = Binding::new(
            move |_| enabled_count > 0,
            move |value, _, cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.set_all_media(&all_chat_id, value, cx));
            },
        );
        let toggled = state.clone();
        hstack(10.0)
            .w_full()
            .px(px(16.0))
            .py(px(8.0))
            .bg(theme.bg._200)
            .child(
                plain("chat-settings-permissions-media-expand")
                    .flex_1()
                    .min_w_0()
                    .on_press(move |_, cx| toggled.update(cx, |state, cx| state.toggle_media_expanded(cx)))
                    .child(
                        hstack(6.0)
                            .w_full()
                            .py(px(8.0))
                            .child(MainText::new(tr("chat_perm_media")).px(15.0).color(theme.text.primary))
                            .child(
                                MainText::new(format!("{enabled_count}/{}", ChatPermKey::MEDIA.len()))
                                    .px(14.0)
                                    .color(theme.text.secondary),
                            )
                            .children(IconUi::sized(
                                if expanded { "chevron.up" } else { "chevron.down" },
                                px(12.0),
                                theme.text.secondary.opacity(0.75),
                                false,
                            ))
                            .child(spacer_min_w(4.0)),
                    ),
            )
            .child(SwitchUi::new("chat-settings-permissions-media-switch", all_media))
    }

    fn slow_mode_card(resolved_chat_id: &str, model: &ChatSettingsModel, theme: &AppTheme) -> impl IntoElement {
        let chips = SLOW_MODE_OPTIONS.iter().copied().map(|seconds| {
            let selected = model.slow_mode == seconds;
            let chat_id = resolved_chat_id.to_owned();
            plain(SharedString::from(format!("chat-settings-slow-mode-{seconds}")))
                .flex_none()
                .h(px(36.0))
                .px(px(14.0))
                .flex()
                .items_center()
                .rounded(px(10.0))
                .bg(if selected { theme.primary._100 } else { theme.bg._200 })
                .on_press(move |_, cx| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.set_slow_mode(&chat_id, seconds, cx));
                })
                .child(
                    MainText::new(slow_mode_label(seconds))
                        .px(14.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(if selected { theme.content_on(theme.primary._100) } else { theme.text.primary }),
                )
        });
        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                div()
                    .px(px(16.0))
                    .child(MainText::new(tr("chat_perm_slowmode")).px(12.0).color(theme.text.secondary)),
            )
            .child(
                div()
                    .id("chat-settings-slow-mode-chips")
                    .w_full()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_x(px(8.0))
                    .gap_y(px(8.0))
                    .px(px(12.0))
                    .children(chips),
            )
    }
}

impl RenderOnce for ChatSettingsPermissions {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let resolved_chat_id = self.chat_id.unwrap_or_default();
        let state = window.use_keyed_state(
            SharedString::from(format!("chat-settings-permissions-{resolved_chat_id}")),
            cx,
            |_, cx| ChatSettingsPermissionsState::new(cx),
        );

        let task_chat_id = resolved_chat_id.clone();
        use_task(
            SharedString::from(format!("chat-settings-permissions-load-{resolved_chat_id}")),
            (),
            window,
            cx,
            async move |cx| {
                let _ = cx.update(|_, cx| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.load_chat_settings(&task_chat_id, cx);
                        store.load_roles(&task_chat_id, cx);
                    });
                });
            },
        );

        let services = ChatServicesStore::shared(cx);
        let model = services
            .read(cx)
            .chat_settings(&resolved_chat_id, cx)
            .cloned()
            .unwrap_or_else(|| ChatSettingsModel { chat_id: resolved_chat_id.clone(), ..ChatSettingsModel::default() });

        let message_setter = |chat_id: &str| -> PermissionSetter {
            let chat_id = chat_id.to_owned();
            Rc::new(move |key: &str, value: bool, cx: &mut App| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.set_message_perm(&chat_id, key, value, cx));
            })
        };
        let set_message = message_setter(&resolved_chat_id);

        let message_row = |key: &'static str, label: &str, theme: &AppTheme| {
            let set = set_message.clone();
            toggle_row(
                format!("chat-settings-permissions-{key}"),
                tr(label),
                model.msg(key),
                0.0,
                theme,
                move |value, cx| set(key, value, cx),
            )
        };

        let topic_mention_role = Self::topic_mention_role(&resolved_chat_id, cx);
        let mention_chat_id = resolved_chat_id.clone();
        let add_chat_id = resolved_chat_id.clone();
        let info_chat_id = resolved_chat_id.clone();
        let media_rows: Vec<AnyElement> = ChatPermKey::MEDIA
            .iter()
            .copied()
            .flat_map(|key| {
                let chat_id = resolved_chat_id.clone();
                [
                    divider(&theme, 32.0).into_any_element(),
                    toggle_row(
                        format!("chat-settings-permissions-media-{key}"),
                        tr(media_label_key(key)),
                        model.media(key),
                        16.0,
                        &theme,
                        move |value, cx| {
                            ChatInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.set_media_perm(&chat_id, key, value, cx));
                        },
                    ),
                ]
            })
            .collect();
        let expanded = state.read(cx).media_expanded;
        let transitioned_media_rows = state.read(cx).media_rows.render_transitioned(RiseClock::monotonic(), window, move |_| {
            div().w_full().flex().flex_col().children(media_rows)
        });

        let permissions_card = vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(div().px(px(16.0)).child(MainText::new(tr("chat_perm_section")).px(12.0).color(theme.text.secondary)))
            .child(
                div().w_full().px(px(12.0)).child(
                    vstack(0.0)
                        .w_full()
                        .bg(theme.bg._200)
                        .rounded(px(20.0))
                        .overflow_hidden()
                        .child(message_row(ChatPermKey::TEXT, "chat_perm_text", &theme))
                        .child(divider(&theme, 16.0))
                        .child(Self::media_parent_row(&resolved_chat_id, &model, &state, expanded, &theme))
                        .children(transitioned_media_rows)
                        .child(divider(&theme, 16.0))
                        .child(message_row(ChatPermKey::POLLS, "chat_perm_polls", &theme))
                        .child(divider(&theme, 16.0))
                        .child(message_row(ChatPermKey::WEB_PREVIEWS, "chat_perm_link_previews", &theme))
                        .child(divider(&theme, 16.0))
                        .child(toggle_row(
                            "chat-settings-permissions-add-members",
                            tr("chat_perm_add_members"),
                            model.members_can_add_members,
                            0.0,
                            &theme,
                            move |value, cx| {
                                ChatInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.set_add_members(&add_chat_id, value, cx));
                            },
                        ))
                        .child(divider(&theme, 16.0))
                        .child(toggle_row(
                            "chat-settings-permissions-change-info",
                            tr("chat_perm_change_info"),
                            model.members_can_change_pfp,
                            0.0,
                            &theme,
                            move |value, cx| {
                                ChatInteractionsStore::shared(cx)
                                    .update(cx, |store, cx| store.set_change_info(&info_chat_id, value, cx));
                            },
                        ))
                        .when_some(topic_mention_role, |card, everyone| {
                            let is_on = ChatPerm::has(everyone.permissions, ChatPerm::MENTION_TOPICS);
                            card.child(divider(&theme, 16.0)).child(toggle_row(
                                "chat-settings-permissions-mention-topics",
                                tr("perm_mention_topics"),
                                is_on,
                                0.0,
                                &theme,
                                move |allowed, cx| {
                                    Self::set_everyone_right(
                                        &mention_chat_id,
                                        &everyone,
                                        ChatPerm::MENTION_TOPICS,
                                        allowed,
                                        cx,
                                    );
                                },
                            ))
                        }),
                ),
            );

        let scroll = scroll_view("chat-settings-permissions-scroll")
            .size_full()
            .list_shell_scroll_under_chrome(cx)
            .child(
                vstack(18.0)
                    .w_full()
                    .pt(px(12.0))
                    .child(permissions_card)
                    .child(Self::slow_mode_card(&resolved_chat_id, &model, &theme))
                    .child(div().flex_none().h(px(60.0))),
            );

        SettingsWrapperUi::new(tr("chat_settings_permissions"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(div().size_full().bg(theme.bg._100).child(scroll))
    }
}
