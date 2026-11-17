use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, ElementId, Empty, Entity, FontWeight, Hsla, ListAlignment, SharedString,
    StyleRefinement, Subscription, Window, list, px,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::modal_sheet_surface::ModalSheetFrame;
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::switch_ui::SwitchRowUi;

use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::ui::input_ui::InputOutlinedRowUi;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::chat::pages::chats::chat::chat_settings::chat_settings_shared::ChatModerationRow;
use crate::modules::chat::shared::chat_notifications_debounce::ChatNotificationsDebounce;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatMember, ChatMuteScopes, ChatTopic, ChatType};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const SHEET_HEADER_HEIGHT: f32 = 52.0;
const TRANSFER_PREFETCH_THRESHOLD: usize = 8;
const TRANSFER_LOADER_PREFETCH_THRESHOLD: usize = 1;

fn sheet_id(name: &str, scope: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("chat-profile-more-{name}-{scope}")))
}

fn scopes_card() -> gpui::Div {
    vstack(0.0)
}

fn divider(theme: &AppTheme) -> gpui::Div {
    div().w_full().pl(px(16.0)).child(div().w_full().h(px(0.5)).bg(theme.border._200.opacity(0.55)))
}

fn toolbar_button(id: ElementId, title: String, color: Hsla, weight: FontWeight) -> PlainButton {
    plain(id).child(MainText::new(title).px(17.0).font_weight(weight).color(color).number_of_lines(1))
}

fn sheet_header(title: String, leading: Option<AnyElement>, trailing: Option<AnyElement>) -> impl IntoElement {
    hstack(8.0)
        .w_full()
        .h(px(SHEET_HEADER_HEIGHT))
        .px(px(16.0))
        .child(div().flex_1().min_w_0().flex().justify_start().children(leading))
        .child(MainText::new(title).px(17.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
        .child(div().flex_1().min_w_0().flex().justify_end().children(trailing))
}

struct ChatNotificationScopesState {
    chat_id: String,
    stored_scopes: ChatMuteScopes,
    shows_events: bool,
    shows_replies: bool,
    messages: bool,
    mentions: bool,
    replies: bool,
    reactions: bool,
    events: bool,
    has_user_edited: bool,
    chat_mute_overrides_did_load: bool,
    mute_all_chats_enabled: bool,
    is_override: bool,
    _user_services: Subscription,
}

impl ChatNotificationScopesState {
    fn new(chat_id: String, current: Option<ChatMuteScopes>, cx: &mut Context<Self>) -> Self {
        let stored_scopes = current.unwrap_or(ChatMuteScopes::ALL_ENABLED);
        let services = ChatServicesStore::shared(cx);
        let chat_type = services.read(cx).chat_info(&chat_id, cx).map(|chat| chat.r#type);
        let shows_events = chat_type != Some(ChatType::Direct);
        let shows_replies = chat_type != Some(ChatType::Channel);
        let effective = services.read(cx).effective_notification_scopes_for_chat(&chat_id, stored_scopes, cx);

        let user_services = UserServicesStore::shared(cx);
        let user = user_services.read(cx);
        let chat_mute_overrides_did_load = user.chat_mute_overrides_did_load();
        let mute_all_chats_enabled = user.mute_all_chats_enabled();
        let is_override = user.chat_mute_override_ids().contains(&chat_id);
        let subscription = cx.observe(&user_services, |state, _, cx| state.user_services_changed(cx));

        Self {
            chat_id,
            stored_scopes,
            shows_events,
            shows_replies,
            messages: effective.messages,
            mentions: effective.mentions,
            replies: if shows_replies { effective.replies } else { effective.mentions },
            reactions: effective.reactions,
            events: if shows_events { effective.events } else { effective.messages },
            has_user_edited: false,
            chat_mute_overrides_did_load,
            mute_all_chats_enabled,
            is_override,
            _user_services: subscription,
        }
    }

    fn current_scopes(&self) -> ChatMuteScopes {
        ChatMuteScopes {
            messages: self.messages,
            mentions: self.mentions,
            replies: if self.shows_replies { self.replies } else { self.mentions },
            reactions: self.reactions,
            events: if self.shows_events { self.events } else { self.messages },
        }
    }

    fn scopes_hint_key(&self) -> &'static str {
        if !self.mute_all_chats_enabled {
            return "chat_mute_scopes_hint";
        }
        if self.is_override { "chat_mute_all_exception_hint" } else { "chat_mute_all_active_hint" }
    }

    fn user_services_changed(&mut self, cx: &mut Context<Self>) {
        let user_services = UserServicesStore::shared(cx);
        let user = user_services.read(cx);
        let did_load = user.chat_mute_overrides_did_load();
        let mute_all = user.mute_all_chats_enabled();
        let is_override = user.chat_mute_override_ids().contains(&self.chat_id);
        let became_loaded = did_load && !self.chat_mute_overrides_did_load;
        let mute_changed = mute_all != self.mute_all_chats_enabled || is_override != self.is_override;
        if !became_loaded && !mute_changed && did_load == self.chat_mute_overrides_did_load {
            return;
        }
        self.chat_mute_overrides_did_load = did_load;
        self.mute_all_chats_enabled = mute_all;
        self.is_override = is_override;
        if became_loaded || mute_changed {
            self.apply_effective_scopes_if_unedited(cx);
        }
        cx.notify();
    }

    fn apply_effective_scopes_if_unedited(&mut self, cx: &mut Context<Self>) {
        if self.has_user_edited {
            return;
        }
        let services = ChatServicesStore::shared(cx);
        let effective = services.read(cx).effective_notification_scopes_for_chat(&self.chat_id, self.stored_scopes, cx);
        self.messages = effective.messages;
        self.mentions = effective.mentions;
        self.replies = if self.shows_replies { effective.replies } else { effective.mentions };
        self.reactions = effective.reactions;
        self.events = if self.shows_events { effective.events } else { effective.messages };
        cx.notify();
    }

    fn set_master(&mut self, on: bool, cx: &mut Context<Self>) {
        self.has_user_edited = true;
        self.messages = on;
        self.mentions = on;
        self.replies = on;
        self.reactions = on;
        self.events = on;
        self.schedule_send(cx);
    }

    fn edited_scope(&mut self, cx: &mut Context<Self>) {
        self.has_user_edited = true;
        self.schedule_send(cx);
    }

    fn schedule_send(&self, cx: &mut Context<Self>) {
        let chat_id = self.chat_id.clone();
        let scopes = self.current_scopes();
        ChatInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.update_notifications_debounced(&chat_id, scopes, cx));
    }
}

/// body of the chat page of the notifications hub
/// host owns the sheet and title and scroll
/// this view owns the four scopes and the send
#[derive(IntoElement)]
pub struct ChatNotificationScopesContent {
    chat_id: String,
    current: Option<ChatMuteScopes>,
}

impl ChatNotificationScopesContent {
    pub fn new(chat_id: impl Into<String>, current: Option<ChatMuteScopes>) -> Self {
        Self { chat_id: chat_id.into(), current }
    }
}

impl RenderOnce for ChatNotificationScopesContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let chat_id = self.chat_id;
        let current = self.current;
        let state_chat_id = chat_id.clone();
        let state = window.use_keyed_state(sheet_id("scopes-state", &chat_id), cx, move |_, cx| {
            ChatNotificationScopesState::new(state_chat_id, current, cx)
        });
        let (shows_events, shows_replies) = {
            let state = state.read(cx);
            (state.shows_events, state.shows_replies)
        };
        let hint_key = state.read(cx).scopes_hint_key();

        let master = Binding::entity(
            state.clone(),
            |state: &ChatNotificationScopesState| state.current_scopes().is_all_enabled(),
            |state, on, cx| state.set_master(on, cx),
        );
        let messages = Binding::entity(
            state.clone(),
            |state: &ChatNotificationScopesState| state.messages,
            |state, on, cx| {
                state.messages = on;
                state.edited_scope(cx);
            },
        );
        let mentions = Binding::entity(
            state.clone(),
            |state: &ChatNotificationScopesState| state.mentions,
            |state, on, cx| {
                state.mentions = on;
                state.edited_scope(cx);
            },
        );
        let replies = Binding::entity(
            state.clone(),
            |state: &ChatNotificationScopesState| state.replies,
            |state, on, cx| {
                state.replies = on;
                state.edited_scope(cx);
            },
        );
        let reactions = Binding::entity(
            state.clone(),
            |state: &ChatNotificationScopesState| state.reactions,
            |state, on, cx| {
                state.reactions = on;
                state.edited_scope(cx);
            },
        );
        let events = Binding::entity(
            state.clone(),
            |state: &ChatNotificationScopesState| state.events,
            |state, on, cx| {
                state.events = on;
                state.edited_scope(cx);
            },
        );

        let appear_state = state.clone();
        let flush_chat_id = chat_id.clone();

        vstack(16.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                scopes_card()
                    .w_full()
                    .bg(theme.bg._200)
                    .rounded_rect(16.0)
                    .overflow_hidden()
                    .child(SwitchRowUi::new(sheet_id("scopes-master", &chat_id), tr("chat_mute_master_all"), master)),
            )
            .child(
                scopes_card()
                    .w_full()
                    .bg(theme.bg._200)
                    .rounded_rect(16.0)
                    .overflow_hidden()
                    .child(SwitchRowUi::new(sheet_id("scopes-messages", &chat_id), tr("chat_mute_scope_messages"), messages))
                    .child(divider(&theme))
                    .child(SwitchRowUi::new(sheet_id("scopes-mentions", &chat_id), tr("chat_mute_scope_mentions"), mentions))
                    .when(shows_replies, |card| {
                        card.child(divider(&theme)).child(SwitchRowUi::new(
                            sheet_id("scopes-replies", &chat_id),
                            tr("chat_mute_scope_replies"),
                            replies,
                        ))
                    })
                    .child(divider(&theme))
                    .child(SwitchRowUi::new(
                        sheet_id("scopes-reactions", &chat_id),
                        tr("chat_mute_scope_reactions"),
                        reactions,
                    ))
                    .when(shows_events, |card| {
                        card.child(divider(&theme)).child(SwitchRowUi::new(
                            sheet_id("scopes-events", &chat_id),
                            tr("chat_mute_scope_events"),
                            events,
                        ))
                    }),
            )
            .child(div().px(px(4.0)).child(MainText::new(tr(hint_key)).px(13.0).color(theme.text.secondary)))
            .on_appear(sheet_id("scopes-life", &chat_id), move |_, cx| {
                UserInteractionsStore::shared(cx).update(cx, |store, cx| store.ensure_chat_mute_overrides_loaded(cx));
                appear_state.update(cx, |state, cx| {
                    if state.chat_mute_overrides_did_load {
                        state.apply_effective_scopes_if_unedited(cx);
                    }
                });
            })
            .and_on_disappear(move |_, cx| {
                SaiDebouncer::flush(&flush_chat_id, ChatNotificationsDebounce::GROUP_KEY, cx);
            })
    }
}

struct TopicNotificationSheetState {
    chat_id: String,
    topic_id: String,
    inherited: bool,
    messages: bool,
    mentions: bool,
    replies: bool,
    reactions: bool,
    events: bool,
    has_user_edited: bool,
    observed_topic: Option<ChatTopic>,
    _chat_services: [Subscription; 2],
}

impl TopicNotificationSheetState {
    fn new(chat_id: String, topic_id: String, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let topic = services.update(cx, |services, cx| services.topic_from_any_cache(&chat_id, &topic_id, cx));
        let scopes = topic
            .as_ref()
            .map(|topic| services.read(cx).effective_topic_notification_scopes(&chat_id, topic, cx))
            .unwrap_or(ChatMuteScopes::ALL_ENABLED);
        let sheet = cx.weak_entity();
        let subscription = [
            cx.observe(&services, |state, _, cx| state.topic_may_have_changed(cx)),
            ChatServicesStore::subscribe_info_keys(Some(chat_id.clone()), cx, move |cx| {
                let _ = sheet.update(cx, |state, cx| state.topic_may_have_changed(cx));
            }),
        ];
        Self {
            inherited: topic.as_ref().is_none_or(|topic| topic.notification_inherited),
            messages: scopes.messages,
            mentions: scopes.mentions,
            replies: scopes.replies,
            reactions: scopes.reactions,
            events: scopes.events,
            has_user_edited: false,
            observed_topic: topic,
            chat_id,
            topic_id,
            _chat_services: subscription,
        }
    }

    fn topic(&self, cx: &mut Context<Self>) -> Option<ChatTopic> {
        let (chat_id, topic_id) = (self.chat_id.clone(), self.topic_id.clone());
        ChatServicesStore::shared(cx).update(cx, |services, cx| services.topic_from_any_cache(&chat_id, &topic_id, cx))
    }

    fn current_scopes(&self) -> ChatMuteScopes {
        ChatMuteScopes {
            messages: self.messages,
            mentions: self.mentions,
            replies: self.replies,
            reactions: self.reactions,
            events: self.events,
        }
    }

    fn topic_may_have_changed(&mut self, cx: &mut Context<Self>) {
        let topic = self.topic(cx);
        if topic == self.observed_topic {
            return;
        }
        self.observed_topic = topic;
        self.apply_store_state(false, cx);
    }

    fn set_master(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.has_user_edited = true;
        self.messages = enabled;
        self.mentions = enabled;
        self.replies = enabled;
        self.reactions = enabled;
        self.events = enabled;
        self.schedule_send(cx);
    }

    fn set_inherited(&mut self, should_inherit: bool, cx: &mut Context<Self>) {
        self.has_user_edited = true;
        self.inherited = should_inherit;
        if should_inherit {
            let scopes = self.inherited_chat_scopes(cx);
            self.apply(scopes);
        }
        self.schedule_send(cx);
    }

    fn edited_scope(&mut self, cx: &mut Context<Self>) {
        self.has_user_edited = true;
        self.schedule_send(cx);
    }

    fn schedule_send(&self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let on_result: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |success: bool, cx: &mut App| {
            if !success {
                let _ = weak.update(cx, |state, cx| state.apply_store_state(true, cx));
            }
        });
        let (chat_id, topic_id) = (self.chat_id.clone(), self.topic_id.clone());
        let scopes = self.current_scopes();
        let inherit = self.inherited;
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_topic_notifications_debounced(&chat_id, &topic_id, scopes, inherit, Some(on_result), cx);
        });
    }

    fn inherited_chat_scopes(&self, cx: &mut Context<Self>) -> ChatMuteScopes {
        let services = ChatServicesStore::shared(cx);
        let Some(chat) = services.read(cx).chat_info(&self.chat_id, cx) else {
            return self.current_scopes();
        };
        services.read(cx).effective_notification_scopes_for_chat(&self.chat_id, chat.notification_scopes(), cx)
    }

    fn apply_store_state(&mut self, force: bool, cx: &mut Context<Self>) {
        if !force && self.has_user_edited {
            return;
        }
        let Some(topic) = self.topic(cx) else {
            return;
        };
        self.inherited = topic.notification_inherited;
        let services = ChatServicesStore::shared(cx);
        let scopes = services.read(cx).effective_topic_notification_scopes(&self.chat_id, &topic, cx);
        self.apply(scopes);
        if force {
            self.has_user_edited = false;
        }
        cx.notify();
    }

    fn apply(&mut self, scopes: ChatMuteScopes) {
        self.messages = scopes.messages;
        self.mentions = scopes.mentions;
        self.replies = scopes.replies;
        self.reactions = scopes.reactions;
        self.events = scopes.events;
    }
}

#[derive(IntoElement)]
pub struct TopicNotificationSheet {
    chat_id: String,
    topic_id: String,
}

impl TopicNotificationSheet {
    pub fn new(chat_id: impl Into<String>, topic_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into(), topic_id: topic_id.into() }
    }
}

impl RenderOnce for TopicNotificationSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let chat_id = self.chat_id;
        let topic_id = self.topic_id;
        let scope = format!("{chat_id}-{topic_id}");
        let (state_chat_id, state_topic_id) = (chat_id.clone(), topic_id.clone());
        let state = window.use_keyed_state(sheet_id("topic-state", &scope), cx, move |_, cx| {
            TopicNotificationSheetState::new(state_chat_id, state_topic_id, cx)
        });
        let inherited = state.read(cx).inherited;

        let inherit = Binding::entity(
            state.clone(),
            |state: &TopicNotificationSheetState| state.inherited,
            |state, should_inherit, cx| state.set_inherited(should_inherit, cx),
        );
        let master = Binding::entity(
            state.clone(),
            |state: &TopicNotificationSheetState| state.current_scopes().is_all_enabled(),
            |state, enabled, cx| {
                if !state.inherited {
                    state.set_master(enabled, cx);
                }
            },
        );
        let messages = Binding::entity(
            state.clone(),
            |state: &TopicNotificationSheetState| state.messages,
            |state, enabled, cx| {
                if !state.inherited {
                    state.messages = enabled;
                    state.edited_scope(cx);
                }
            },
        );
        let mentions = Binding::entity(
            state.clone(),
            |state: &TopicNotificationSheetState| state.mentions,
            |state, enabled, cx| {
                if !state.inherited {
                    state.mentions = enabled;
                    state.edited_scope(cx);
                }
            },
        );
        let replies = Binding::entity(
            state.clone(),
            |state: &TopicNotificationSheetState| state.replies,
            |state, enabled, cx| {
                if !state.inherited {
                    state.replies = enabled;
                    state.edited_scope(cx);
                }
            },
        );
        let reactions = Binding::entity(
            state.clone(),
            |state: &TopicNotificationSheetState| state.reactions,
            |state, enabled, cx| {
                if !state.inherited {
                    state.reactions = enabled;
                    state.edited_scope(cx);
                }
            },
        );
        let events = Binding::entity(
            state.clone(),
            |state: &TopicNotificationSheetState| state.events,
            |state, enabled, cx| {
                if !state.inherited {
                    state.events = enabled;
                    state.edited_scope(cx);
                }
            },
        );

        let appear_state = state.clone();
        let (flush_chat_id, flush_topic_id) = (chat_id.clone(), topic_id.clone());
        let hint_key = if inherited { "topic_notifications_inherit_hint" } else { "chat_mute_scopes_hint" };

        vstack(0.0)
            .w_full()
            .bg(theme.bg._100)
            .child(sheet_header(tr("chat_mute_advanced_title"), None, None))
            .child(
                div().w_full().child(
                        vstack(16.0)
                            .w_full()
                            .p(px(16.0))
                            .align(HorizontalAlignment::Leading)
                            .child(
                                scopes_card().w_full().bg(theme.bg._200).rounded_rect(16.0).overflow_hidden().child(
                                    SwitchRowUi::new(
                                        sheet_id("topic-inherit", &scope),
                                        tr("topic_notifications_inherit"),
                                        inherit,
                                    ),
                                ),
                            )
                            .child(
                                div()
                                    .relative()
                                    .w_full()
                                    .bg(theme.bg._200)
                                    .rounded_rect(16.0)
                                    .overflow_hidden()
                                    .child(
                                        scopes_card()
                                            .w_full()
                                            .when(inherited, |card| card.opacity(0.55))
                                            .child(SwitchRowUi::new(
                                                sheet_id("topic-master", &scope),
                                                tr("chat_mute_master_all"),
                                                master,
                                            ))
                                            .child(divider(&theme))
                                            .child(SwitchRowUi::new(
                                                sheet_id("topic-messages", &scope),
                                                tr("chat_mute_scope_messages"),
                                                messages,
                                            ))
                                            .child(divider(&theme))
                                            .child(SwitchRowUi::new(
                                                sheet_id("topic-mentions", &scope),
                                                tr("chat_mute_scope_mentions"),
                                                mentions,
                                            ))
                                            .child(divider(&theme))
                                            .child(SwitchRowUi::new(
                                                sheet_id("topic-replies", &scope),
                                                tr("chat_mute_scope_replies"),
                                                replies,
                                            ))
                                            .child(divider(&theme))
                                            .child(SwitchRowUi::new(
                                                sheet_id("topic-reactions", &scope),
                                                tr("chat_mute_scope_reactions"),
                                                reactions,
                                            ))
                                            .child(divider(&theme))
                                            .child(SwitchRowUi::new(
                                                sheet_id("topic-events", &scope),
                                                tr("chat_mute_scope_events"),
                                                events,
                                            )),
                                    )
                                    .when(inherited, |card| card.child(div().absolute().inset_0().occlude())),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .px(px(4.0))
                                    .child(MainText::new(tr(hint_key)).px(13.0).color(theme.text.secondary)),
                            ),
                ),
            )
            .on_appear(sheet_id("topic-life", &scope), move |_, cx| {
                appear_state.update(cx, |state, cx| state.apply_store_state(false, cx));
            })
            .and_on_disappear(move |_, cx| {
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.flush_topic_notifications(&flush_chat_id, &flush_topic_id, cx));
            })
    }
}

const AUTO_DELETE_OPTIONS: [(i64, &str); 4] = [
    (0, "chat_autodelete_off"),
    (86_400, "chat_autodelete_1d"),
    (604_800, "chat_autodelete_1w"),
    (2_592_000, "chat_autodelete_1m"),
];

#[derive(IntoElement)]
pub struct AutoDeleteSheet {
    on_submit: Option<Rc<dyn Fn(i64, &mut App)>>,
    chat_id: String,
    current: i64,
}

impl AutoDeleteSheet {
    pub fn new(chat_id: impl Into<String>, current: i64) -> Self {
        Self { on_submit: None, chat_id: chat_id.into(), current }
    }

    pub fn on_submit(mut self, on_submit: impl Fn(i64, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }
}

impl RenderOnce for AutoDeleteSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let chat_id = self.chat_id;
        let current = self.current;
        let selected_state: Entity<i64> =
            window.use_keyed_state(sheet_id("autodelete-selected", &chat_id), cx, move |_, _| current);
        let selected = *selected_state.read(cx);
        let dismiss_action = dismiss(cx);

        let count = AUTO_DELETE_OPTIONS.len();
        let rows = AUTO_DELETE_OPTIONS.iter().enumerate().map(|(index, (seconds, key))| {
            let seconds = *seconds;
            let select_state = selected_state.clone();
            vstack(0.0)
                .w_full()
                .child(
                    plain(sheet_id("autodelete-option", &format!("{chat_id}-{seconds}")))
                        .w_full()
                        .on_press(move |_, cx| {
                            select_state.update(cx, |selected, cx| {
                                *selected = seconds;
                                cx.notify();
                            });
                        })
                        .child(
                            hstack(8.0)
                                .w_full()
                                .px(px(16.0))
                                .py(px(12.0))
                                .bg(theme.bg._200)
                                .child(div().flex_1().min_w_0().child(MainText::new(tr(*key)).px(15.0)))
                                .when(selected == seconds, |row| {
                                    row.children(IconUi::sized("checkmark", px(14.0), theme.primary._100, false))
                                }),
                        ),
                )
                .when(index + 1 < count, |row| row.child(divider(&theme)))
        });

        let cancel_dismiss = dismiss_action.clone();
        let done_dismiss = dismiss_action;
        let done_state = selected_state.clone();
        let done_chat_id = chat_id.clone();
        let on_submit = self.on_submit;

        let cancel = toolbar_button(
            sheet_id("autodelete-cancel", &chat_id),
            tr("common_cancel"),
            theme.primary._100,
            FontWeight::NORMAL,
        )
        .on_press(move |window, cx| cancel_dismiss.call_as_function(window, cx))
        .into_any_element();
        let done = toolbar_button(
            sheet_id("autodelete-done", &chat_id),
            tr("common_done"),
            theme.primary._100,
            FontWeight::SEMIBOLD,
        )
        .on_press(move |window, cx| {
            let seconds = *done_state.read(cx);
            match &on_submit {
                Some(on_submit) => on_submit(seconds, cx),
                None => ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.set_auto_delete(&done_chat_id, seconds, cx)),
            }
            done_dismiss.call_as_function(window, cx);
        })
        .into_any_element();

        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(sheet_header(tr("chat_autodelete_title"), Some(cancel), Some(done)))
            .child(
                div().flex_1().min_h_0().child(
                    scroll_view(sheet_id("autodelete-scroll", &chat_id)).size_full().child(
                        vstack(8.0)
                            .w_full()
                            .p(px(16.0))
                            .align(HorizontalAlignment::Leading)
                            .child(
                                vstack(0.0)
                                    .w_full()
                                    .bg(theme.bg._200)
                                    .rounded_rect(16.0)
                                    .overflow_hidden()
                                    .children(rows),
                            )
                            .child(
                                div()
                                    .px(px(4.0))
                                    .child(MainText::new(tr("chat_autodelete_hint")).px(13.0).color(theme.text.secondary)),
                            ),
                    ),
                ),
            )
    }
}

#[derive(Clone)]
enum TransferOwnershipRow {
    Member { member: ChatMember, index: usize },
    NextPageLoader,
    Loading,
    Empty,
}

impl TransferOwnershipRow {
    fn key(&self) -> String {
        match self {
            Self::Member { member, .. } => member.user_id.clone(),
            Self::NextPageLoader => "\u{1}next-page".to_owned(),
            Self::Loading => "\u{1}loading".to_owned(),
            Self::Empty => "\u{1}empty".to_owned(),
        }
    }
}

/// desktop only, the sheet as its own cached view held in element state
struct TransferOwnershipSheetView {
    chat_id: String,
    height: f32,
    query: Entity<TransferOwnershipQuery>,
    rows_query: Option<String>,
    rows_stale: bool,
    list: ListUi,
    row_keys: Vec<String>,
    rows: Rc<Vec<TransferOwnershipRow>>,
    item_count: usize,
    last_id: Option<String>,
    has_more: bool,
    _observations: [Subscription; 3],
}

/// search text kept apart from the view so the field reads it while the view renders
struct TransferOwnershipQuery(String);

impl TransferOwnershipSheetView {
    fn new(chat_id: String, height: f32, theme: &AppTheme, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let query = cx.new(|_| TransferOwnershipQuery(String::new()));
        let sheet = cx.weak_entity();
        let observations = [
            cx.observe(&services, |view, _, cx| {
                view.rows_stale = true;
                cx.notify();
            }),
            ChatServicesStore::subscribe_info_keys(Some(chat_id.clone()), cx, move |cx| {
                let _ = sheet.update(cx, |view, cx| {
                    view.rows_stale = true;
                    cx.notify();
                });
            }),
            cx.observe(&query, |_, _, cx| cx.notify()),
        ];
        Self {
            chat_id,
            height,
            query,
            rows_query: None,
            rows_stale: true,
            list: ListUi::new(theme, 0, ListAlignment::Top),
            row_keys: Vec::new(),
            rows: Rc::new(Vec::new()),
            item_count: 0,
            last_id: None,
            has_more: false,
            _observations: observations,
        }
    }

    fn set_height(&mut self, height: f32, cx: &mut Context<Self>) {
        if (self.height - height).abs() > f32::EPSILON {
            self.height = height;
            cx.notify();
        }
    }

    fn candidates(query: &str, all: &[ChatMember], my_id: &str) -> Vec<(ChatMember, usize)> {
        let q = trimming_whitespaces_and_newlines(query).to_lowercase();
        all.iter()
            .enumerate()
            .filter(|(_, member)| !member.is_owner() && member.user_id != my_id)
            .filter(|(_, member)| {
                q.is_empty() || member.name.to_lowercase().contains(&q) || member.tag.to_lowercase().contains(&q)
            })
            .map(|(index, member)| (member.clone(), index))
            .collect()
    }

    fn rebuild_rows(&mut self, query: String, cx: &mut Context<Self>) {
        let chat_id = self.chat_id.clone();
        let services = ChatServicesStore::shared(cx);
        let all = services.update(cx, |services, cx| services.members(&chat_id, cx));
        let my_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        let candidates = Self::candidates(&query, &all, &my_id);
        let last_id = services.update(cx, |services, cx| services.members_last_id(&chat_id, cx));
        let has_more = services.update(cx, |services, cx| services.members_has_more(&chat_id, cx));
        let is_loading = services.read(cx).is_member_list_loading(&chat_id);

        let candidates_empty = candidates.is_empty();
        let mut rows: Vec<TransferOwnershipRow> =
            candidates.into_iter().map(|(member, index)| TransferOwnershipRow::Member { member, index }).collect();
        if has_more {
            rows.push(TransferOwnershipRow::NextPageLoader);
        } else if candidates_empty {
            rows.push(if is_loading { TransferOwnershipRow::Loading } else { TransferOwnershipRow::Empty });
        }
        let keys = rows.iter().map(TransferOwnershipRow::key).collect::<Vec<_>>();
        self.sync_list(keys);
        self.rows = Rc::new(rows);
        self.item_count = all.len();
        self.last_id = last_id;
        self.has_more = has_more;
        self.rows_query = Some(query);
        self.rows_stale = false;
    }

    fn sync_list(&mut self, keys: Vec<String>) {
        if keys == self.row_keys {
            return;
        }
        let start = keys.iter().zip(&self.row_keys).take_while(|(new, old)| new == old).count();
        let tail_limit = keys.len().min(self.row_keys.len()) - start;
        let end = keys
            .iter()
            .rev()
            .zip(self.row_keys.iter().rev())
            .take(tail_limit)
            .take_while(|(new, old)| new == old)
            .count();
        self.list.state().splice(start..self.row_keys.len() - end, keys.len() - start - end);
        self.row_keys = keys;
    }
}

#[derive(IntoElement)]
pub struct TransferOwnershipSheet {
    chat_id: String,
}

impl TransferOwnershipSheet {
    pub fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into() }
    }

    fn load_more(chat_id: &str, cx: &mut App) {
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_members(chat_id, None, cx));
    }

    fn confirm(chat_id: String, target: ChatMember, dismiss_action: DismissAction, window: &mut Window, cx: &mut App) {
        let handle = window.window_handle();
        let message = format!("{} {}", target.name, tr("chat_transfer_confirm_suffix"));
        NativeConfirmAlert::present_delete(
            &tr("chat_transfer_confirm_title"),
            &message,
            Some(tr("chat_transfer_confirm")),
            None,
            false,
            None,
            None,
            move |cx| {
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.transfer_ownership(&chat_id, &target.user_id, None, cx));
                let _ = handle.update(cx, |_, window, cx| dismiss_action.call_as_function(window, cx));
            },
            cx,
        );
    }
}

struct TransferOwnershipRowContext {
    chat_id: String,
    item_count: usize,
    cache_key: String,
    last_id: Option<String>,
    has_more: bool,
    dismiss_action: DismissAction,
    theme: AppTheme,
}

impl TransferOwnershipRowContext {
    fn row(&self, row: &TransferOwnershipRow) -> AnyElement {
        let loader =
            || div().w_full().flex().justify_center().py(px(24.0)).child(LoaderUi::new().size(LoaderUiSize::Small));
        let load_chat_id = self.chat_id.clone();
        match row {
            TransferOwnershipRow::Member { member, index } => {
                let subtitle = if member.tag.is_empty() { String::new() } else { format!("@{}", member.tag) };
                let press_chat_id = self.chat_id.clone();
                let press_member = member.clone();
                let press_dismiss = self.dismiss_action.clone();
                vstack(0.0)
                    .w_full()
                    .pb(px(1.0))
                    .child(
                        plain(sheet_id("transfer-row", &member.user_id))
                            .w_full()
                            .on_press(move |window, cx| {
                                TransferOwnershipSheet::confirm(
                                    press_chat_id.clone(),
                                    press_member.clone(),
                                    press_dismiss.clone(),
                                    window,
                                    cx,
                                );
                            })
                            .child(
                                ChatModerationRow::new(
                                    member.avatar_url.clone(),
                                    member.name.clone(),
                                    subtitle,
                                    member.user_id.clone(),
                                )
                                .online(member.is_online),
                            )
                            .sai_pagination_prefetch(
                                sheet_id("transfer-prefetch", &member.user_id),
                                SaiPaginationPrefetchOptions::new(
                                    *index,
                                    self.item_count,
                                    self.cache_key.clone(),
                                    self.last_id.clone(),
                                    self.has_more,
                                )
                                .threshold(TRANSFER_PREFETCH_THRESHOLD),
                                move |_, cx| TransferOwnershipSheet::load_more(&load_chat_id, cx),
                            ),
                    )
                    .child(div().w_full().h(px(0.5)).bg(self.theme.border._100))
                    .into_any_element()
            }
            TransferOwnershipRow::NextPageLoader => loader()
                .sai_pagination_prefetch(
                    sheet_id("transfer-prefetch-tail", &self.chat_id),
                    SaiPaginationPrefetchOptions::new(
                        0,
                        1,
                        self.cache_key.clone(),
                        self.last_id.clone(),
                        self.has_more,
                    )
                    .threshold(TRANSFER_LOADER_PREFETCH_THRESHOLD),
                    move |_, cx| TransferOwnershipSheet::load_more(&load_chat_id, cx),
                )
                .into_any_element(),
            TransferOwnershipRow::Loading => loader().into_any_element(),
            TransferOwnershipRow::Empty => {
                ChatProfileEmptyState::new("magnifyingglass", tr("people_picker_empty")).into_any_element()
            }
        }
    }
}

impl RenderOnce for TransferOwnershipSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let sheet_height = ModalSheetFrame::for_window(window, cx).fill_floor();
        let chat_id = self.chat_id;
        let view = window.with_global_id(sheet_id("transfer-view", &chat_id), |global_id, window| {
            window.with_element_state(global_id, |held: Option<Entity<TransferOwnershipSheetView>>, _| {
                let view = held.unwrap_or_else(|| {
                    cx.new(|cx| TransferOwnershipSheetView::new(chat_id.clone(), sheet_height, &theme, cx))
                });
                (view.clone(), view)
            })
        });
        view.update(cx, |view, cx| view.set_height(sheet_height, cx));
        AnyView::from(view).cached(StyleRefinement::default().w_full().h(px(sheet_height)))
    }
}

impl Render for TransferOwnershipSheetView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query_text = self.query.read(cx).0.clone();
        if self.rows_stale || self.rows_query.as_deref() != Some(query_text.as_str()) {
            self.rebuild_rows(query_text, cx);
        }
        let theme = rise_ui::theme(cx).clone();
        let chat_id = self.chat_id.clone();
        let dismiss_action = dismiss(cx);

        let list_state = self.list.state().clone();
        let rows = self.rows.clone();
        let row_context = TransferOwnershipRowContext {
            chat_id: chat_id.clone(),
            item_count: self.item_count,
            cache_key: ChatServicesStore::members_cache_key(&chat_id),
            last_id: self.last_id.clone(),
            has_more: self.has_more,
            dismiss_action: dismiss_action.clone(),
            theme: theme.clone(),
        };
        let members_list = list(list_state, move |index, _, _| match rows.get(index) {
            Some(row) => row_context.row(row),
            None => Empty.into_any_element(),
        })
        .size_full();

        let query = Binding::entity(
            self.query.clone(),
            |query: &TransferOwnershipQuery| query.0.clone(),
            |query, text, _| query.0 = text,
        );
        let cancel = toolbar_button(
            sheet_id("transfer-cancel", &chat_id),
            tr("common_cancel"),
            theme.primary._100,
            FontWeight::NORMAL,
        )
        .on_press(move |window, cx| dismiss_action.call_as_function(window, cx))
        .into_any_element();
        let appear_chat_id = chat_id.clone();

        vstack(0.0)
            .w_full()
            .h(px(self.height))
            .bg(theme.bg._100)
            .child(sheet_header(tr("chat_transfer_title"), Some(cancel), None))
            .child(div().w_full().p(px(12.0)).child(InputOutlinedRowUi::new(
                sheet_id("transfer-search", &chat_id),
                query,
                tr("chat_transfer_search"),
                div().children(IconUi::sized("magnifyingglass", px(17.0), theme.text.secondary, false)),
            )))
            .child(div().w_full().flex_1().min_h_0().child(members_list))
            .on_appear(sheet_id("transfer-life", &chat_id), move |_, cx| {
                ChatInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_members(&appear_chat_id, None, None, true, cx));
            })
    }
}
