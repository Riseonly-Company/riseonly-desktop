use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, Entity, Hsla, Render, SharedString, Subscription, Task, Window};
use rise_canon::RiseTimingCanon;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::input_ui::input_ui_state::{InputUiEvent, InputUiState};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::{ControlSize, progress_view};
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::stacks::{hstack, spacer_min_h, vstack};
use rise_ui::layer_keys::LayerKeysExt;
use rise_validate::CHARSET_TAG;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::stores::tag::tag_actions::TagActionsStore;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, ListShellUi};
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet::CreateChatForwardGroupedBtns;
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::ChatFieldRules;

const SELECTED_MARK: &str = "\u{2713}";
const TAG_AT_SIGN: char = '@';
const TAG_FIELD_FONT_SIZE: f32 = 16.0;
const PUBLIC_LINK_ROW_MINIMUM_HEIGHT: f32 = 58.0;
const TAG_STATUS_ICON_SIDE: f32 = 17.0;
const SAVE_ICON_SIDE: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TagState {
    Idle,
    Checking,
    Available,
    Taken,
    Invalid,
    Failed,
}

pub struct ChatSettingsType {
    chat_id: Option<String>,
    is_public: bool,
    original_is_public: bool,
    tag: String,
    original_tag: String,
    forward_blocked: bool,
    original_forward_blocked: bool,
    tag_state: TagState,
    is_saving: bool,
    tag_check_task: Option<Task<()>>,
    chat_services: Entity<ChatServicesStore>,
    tag_field: Entity<InputUiState>,
    _subscriptions: Vec<Subscription>,
}

impl ChatSettingsType {
    pub fn new(chat_id: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let chat_services = ChatServicesStore::shared(cx);
        let tag_field = cx.new(InputUiState::single_line);
        tag_field.update(cx, |field, cx| {
            field.set_chromeless(true, cx);
            field.set_font_size(Some(px(TAG_FIELD_FONT_SIZE)), cx);
            field.set_placeholder(tr("chat_settings_public_link_placeholder"), cx);
        });
        let subscriptions = vec![
            cx.observe(&chat_services, |_, _, cx| cx.notify()),
            cx.subscribe_in(&tag_field, window, |this, _, event, _, cx| {
                if matches!(event, InputUiEvent::Changed) {
                    this.tag_field_did_change(cx);
                }
            }),
        ];
        Self {
            chat_id,
            is_public: false,
            original_is_public: false,
            tag: String::new(),
            original_tag: String::new(),
            forward_blocked: false,
            original_forward_blocked: false,
            tag_state: TagState::Idle,
            is_saving: false,
            tag_check_task: None,
            chat_services,
            tag_field,
            _subscriptions: subscriptions,
        }
    }

    fn chat(&self, cx: &App) -> Option<ChatInfo> {
        let chat_id = self.chat_id.as_deref().filter(|chat_id| !chat_id.is_empty())?;
        self.chat_services.read(cx).chat_info(chat_id, cx)
    }

    fn normalized_tag(&self) -> String {
        match ChatFieldRules::tag().schema {
            Some(schema) => schema.normalize(&self.tag),
            None => self.tag.clone(),
        }
    }

    fn tag_is_unchanged(&self) -> bool {
        self.normalized_tag() == self.original_tag.to_lowercase()
    }

    fn tag_is_valid_for_save(&self, cx: &App) -> bool {
        if !self.is_public {
            return true;
        }
        let normalized_tag = self.normalized_tag();
        if normalized_tag.is_empty() {
            return !self.tag_is_required(cx);
        }
        if self.tag_is_unchanged() {
            return true;
        }
        if !Self::is_locally_valid(&normalized_tag) {
            return false;
        }
        !matches!(self.tag_state, TagState::Taken | TagState::Invalid | TagState::Checking)
    }

    /// same rule as channel creation, a public channel is found by its address
    fn tag_is_required(&self, cx: &App) -> bool {
        self.chat(cx).is_some_and(|chat| chat.r#type == ChatType::Channel)
    }

    fn visibility_changed(&self) -> bool {
        self.is_public != self.original_is_public || self.normalized_tag() != self.original_tag.to_lowercase()
    }

    fn has_changes(&self) -> bool {
        self.visibility_changed() || self.forward_blocked != self.original_forward_blocked
    }

    fn can_save(&self, cx: &App) -> bool {
        self.has_changes() && self.tag_is_valid_for_save(cx) && !self.is_saving
    }

    fn tag_field_did_change(&mut self, cx: &mut Context<Self>) {
        let value = self.tag_field.read(cx).text().to_owned();
        if value == self.tag {
            return;
        }
        let sanitized = Self::sanitize_tag_input(&value);
        self.tag = sanitized.clone();
        if sanitized != value {
            self.tag_field.update(cx, |field, cx| field.set_text(sanitized, cx));
        }
        self.schedule_tag_check(cx);
    }

    fn public_link_editor(&self, theme: &AppTheme, cx: &App) -> Div {
        vstack(8.0)
            .w_full()
            .items_stretch()
            .child(
                div().px(px(16.0)).child(
                    MainText::new(tr("chat_settings_public_link_title")).px(13.0).color(theme.text.secondary),
                ),
            )
            .child(
                div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).child(
                    GroupedCardMetrics::SHAPE.round(
                        hstack(0.0)
                            .w_full()
                            .min_h(px(PUBLIC_LINK_ROW_MINIMUM_HEIGHT))
                            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                            .bg(theme.bg._200)
                            .child(
                                MainText::new(format!("{}/", AppEnvironment::current().public_host()))
                                    .px(16.0)
                                    .color(theme.text.secondary),
                            )
                            .child(div().flex_1().min_w_0().child(self.tag_field.clone()))
                            .children(self.tag_status_view()),
                    ),
                ),
            )
            .child(
                div().px(px(16.0)).child(
                    MainText::new(self.tag_hint(cx)).px(13.0).color(self.tag_hint_color(theme)).number_of_lines(0),
                ),
            )
    }

    fn tag_status_view(&self) -> Option<AnyElement> {
        match self.tag_state {
            TagState::Checking => Some(progress_view().control_size(ControlSize::Small).into_any_element()),
            TagState::Available => IconUi::sized(
                "checkmark.circle.fill",
                px(TAG_STATUS_ICON_SIDE),
                ChatTileColor::green(),
                true,
            )
            .map(IntoElement::into_any_element),
            TagState::Taken | TagState::Invalid | TagState::Failed => {
                IconUi::sized("xmark.circle.fill", px(TAG_STATUS_ICON_SIDE), ChatTileColor::red(), true)
                    .map(IntoElement::into_any_element)
            }
            TagState::Idle => None,
        }
    }

    fn empty_tag_hint(&self, cx: &App) -> String {
        tr(if self.tag_is_required(cx) {
            "chat_settings_public_link_required_hint"
        } else {
            "chat_settings_public_link_optional_hint"
        })
    }

    fn tag_hint(&self, cx: &App) -> String {
        if self.normalized_tag().is_empty() {
            return self.empty_tag_hint(cx);
        }
        if self.tag_is_unchanged() {
            return tr("chat_settings_public_link_current");
        }
        match self.tag_state {
            TagState::Available => tr("chat_settings_public_link_available"),
            TagState::Taken => tr("chat_settings_public_link_taken"),
            TagState::Invalid => tr("chat_settings_public_link_invalid"),
            TagState::Failed => tr("tag_check_failed"),
            TagState::Checking => tr("chat_settings_public_link_checking"),
            TagState::Idle => self.empty_tag_hint(cx),
        }
    }

    fn tag_hint_color(&self, theme: &AppTheme) -> Hsla {
        match self.tag_state {
            TagState::Available => ChatTileColor::green(),
            TagState::Taken | TagState::Invalid | TagState::Failed => ChatTileColor::red(),
            TagState::Idle | TagState::Checking => theme.text.secondary,
        }
    }

    fn save_button(
        &self,
        navigation_store: Option<Entity<NavigationStore>>,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> LiquidGlassButton {
        let can_save = self.can_save(cx);
        let this = cx.entity();
        LiquidGlassButton::new("chat-settings-type-save", LiquidGlassButtonShape::Circle)
            .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
            .tint(can_save.then_some(theme.primary._100))
            .haptic_on_press(can_save)
            .disabled(!can_save)
            .accessibility_label(Some(tr("common_done")))
            .on_press(move |_: &mut Window, cx: &mut App| Self::save(&this, navigation_store.clone(), cx))
            .map(|button| {
                if self.is_saving {
                    button.child(progress_view().control_size(ControlSize::Small).tint(theme.content_on(theme.primary._100)))
                } else {
                    let tint = if can_save { theme.content_on(theme.primary._100) } else { theme.text.secondary };
                    button.children(IconUi::sized("checkmark", px(SAVE_ICON_SIDE), tint, false))
                }
            })
    }

    fn hydrate(&mut self, cx: &mut Context<Self>) {
        let Some(chat) = self.chat(cx) else {
            if let Some(chat_id) = self.chat_id.as_deref().filter(|chat_id| !chat_id.is_empty()) {
                ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_chat_by_id_action(chat_id, false, cx));
            }
            return;
        };
        self.is_public = chat.is_public;
        self.original_is_public = chat.is_public;
        let current_tag = if chat.is_public {
            chat.tag
                .as_deref()
                .map(|tag| trimming_whitespaces_and_newlines(tag).trim_matches(TAG_AT_SIGN).to_lowercase())
                .unwrap_or_default()
        } else {
            String::new()
        };
        self.tag = current_tag.clone();
        self.original_tag = current_tag.clone();
        self.tag_field.update(cx, |field, cx| field.set_text(current_tag, cx));
        self.tag_state = TagState::Idle;
        self.forward_blocked = !chat.forward_in_chat_enabled;
        self.original_forward_blocked = self.forward_blocked;
        cx.notify();
    }

    fn select_public(&mut self, cx: &mut Context<Self>) {
        self.is_public = true;
        self.schedule_tag_check(cx);
    }

    fn select_private(this: &Entity<Self>, cx: &mut App) {
        let state = this.read(cx);
        if !state.is_public {
            return;
        }
        if state.normalized_tag().is_empty() {
            this.update(cx, |state, cx| {
                state.is_public = false;
                cx.notify();
            });
            return;
        }
        let confirmed = this.clone();
        NativeConfirmAlert::present_delete(
            &tr("chat_settings_type_private_warning_title"),
            &tr("chat_settings_type_private_warning_body"),
            Some(tr("chat_settings_type_private_warning_action")),
            None,
            false,
            None,
            None,
            move |cx: &mut App| {
                confirmed.update(cx, |state, cx| {
                    state.is_public = false;
                    state.tag = String::new();
                    state.tag_field.update(cx, |field, cx| field.set_text("", cx));
                    state.schedule_tag_check(cx);
                });
            },
            cx,
        );
    }

    fn schedule_tag_check(&mut self, cx: &mut Context<Self>) {
        self.tag_check_task = None;
        let candidate = self.normalized_tag();
        if !self.is_public || candidate.is_empty() || self.tag_is_unchanged() {
            self.tag_state = TagState::Idle;
            cx.notify();
            return;
        }
        if !Self::is_locally_valid(&candidate) {
            self.tag_state = TagState::Invalid;
            cx.notify();
            return;
        }
        self.tag_state = TagState::Checking;
        cx.notify();
        let debounce_ms = RiseTimingCanon::debounce("chat_tag_availability").client_debounce_ms;
        self.tag_check_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(u64::try_from(debounce_ms).unwrap_or(0))).await;
            if !this.update(cx, |state, _| candidate == state.normalized_tag()).unwrap_or(false) {
                return;
            }
            cx.update(|cx| {
                let success_candidate = candidate.clone();
                let failure_candidate = candidate.clone();
                let on_success_state = this.clone();
                let on_error_state = this.clone();
                TagActionsStore::shared(cx).update(cx, |tags, cx| {
                    tags.check_tag_exist_action(
                        &candidate,
                        move |response, cx| {
                            let _ = on_success_state.update(cx, |state, cx| {
                                if success_candidate != state.normalized_tag() {
                                    return;
                                }
                                state.tag_state = if response.exists { TagState::Taken } else { TagState::Available };
                                cx.notify();
                            });
                        },
                        Some(Rc::new(move |_: &anyhow::Error, cx: &mut App| {
                            let _ = on_error_state.update(cx, |state, cx| {
                                if failure_candidate != state.normalized_tag() {
                                    return;
                                }
                                state.tag_state = TagState::Failed;
                                cx.notify();
                            });
                        })),
                        cx,
                    );
                });
            });
        }));
    }

    fn save(this: &Entity<Self>, navigation_store: Option<Entity<NavigationStore>>, cx: &mut App) {
        let request = this.update(cx, |state, cx| {
            if !state.can_save(cx) {
                return None;
            }
            let chat_id = state.chat_id.clone().filter(|chat_id| !chat_id.is_empty())?;
            state.is_saving = true;
            cx.notify();
            Some((
                chat_id,
                state.visibility_changed().then_some(state.is_public),
                if state.is_public { state.normalized_tag() } else { String::new() },
                (state.forward_blocked != state.original_forward_blocked).then_some(!state.forward_blocked),
            ))
        });
        let Some((chat_id, is_public, tag, forward_in_chat_enabled)) = request else {
            return;
        };
        let weak_this = this.downgrade();
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.update_chat_visibility(
                &chat_id,
                is_public,
                &tag,
                forward_in_chat_enabled,
                Box::new(move |success: bool, reason: Option<String>, cx: &mut App| {
                    let _ = weak_this.update(cx, |state, cx| {
                        state.is_saving = false;
                        cx.notify();
                    });
                    if success {
                        if let Some(navigation_store) = &navigation_store {
                            navigation_store.update(cx, |store, cx| store.pop(cx));
                        }
                    } else if let Some(reason) = reason {
                        Notifier::send(
                            reason,
                            NotifierSendOptions {
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Bottom,
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    } else {
                        Notifier::send(
                            tr("chat_settings_type_save_failed"),
                            NotifierSendOptions {
                                content: Some(tr("chat_settings_type_save_failed_hint")),
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Bottom,
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    }
                }),
                cx,
            );
        });
    }

    fn sanitize_tag_input(value: &str) -> String {
        let rule = ChatFieldRules::tag();
        let input = match rule.schema {
            Some(schema) => schema.input(value),
            None => value.to_owned(),
        };
        let allowed: String = input.chars().filter(|character| CHARSET_TAG.allows(*character)).collect();
        rule.clamped(&allowed)
    }

    fn is_locally_valid(value: &str) -> bool {
        ChatFieldRules::tag().is_satisfied(value)
    }
}

impl Render for ChatSettingsType {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let this = cx.entity();
        let navigation_store = env::<NavigationStoreKey>(cx);
        let chat = self.chat(cx);
        let is_channel = chat.as_ref().is_some_and(|chat| chat.r#type == ChatType::Channel);

        let updated_at = chat.as_ref().map(|chat| chat.updated_at);
        let hydrating = this.clone();
        on_change("chat-settings-type-updated-at", updated_at, false, window, cx, move |_, _, _, cx| {
            hydrating.update(cx, |state, cx| {
                if state.has_changes() || state.is_saving {
                    return;
                }
                state.hydrate(cx);
            });
        });

        self.tag_field.update(cx, |field, cx| {
            field.set_text_color(Some(theme.text.primary), cx);
            field.set_placeholder_color(Some(theme.text.secondary), cx);
        });

        let on_public = this.clone();
        let on_private = this.clone();
        let visibility_rows = GroupedBtns::new(
            "chat-settings-type-visibility",
            vec![
                GroupedBtn::new("public", tr("chat_settings_type_public"))
                    .icon("globe")
                    .icon_background(ChatTileColor::blue())
                    .trailing(if self.is_public { SELECTED_MARK } else { "" })
                    .show_chevron(false)
                    .callback(move |_: &mut Window, cx: &mut App| {
                        on_public.update(cx, |state, cx| state.select_public(cx));
                    }),
                GroupedBtn::new("private", tr("chat_settings_type_private"))
                    .icon("lock.fill")
                    .icon_background(ChatTileColor::green())
                    .trailing(if self.is_public { "" } else { SELECTED_MARK })
                    .show_chevron(false)
                    .callback(move |_: &mut Window, cx: &mut App| Self::select_private(&on_private, cx)),
            ],
        );

        let blocked_reader = this.clone();
        let blocked_writer = this.clone();
        let forward_blocked = Binding::new(
            move |cx: &App| blocked_reader.read(cx).forward_blocked,
            move |value: bool, _: &mut Window, cx: &mut App| {
                blocked_writer.update(cx, |state, cx| {
                    state.forward_blocked = value;
                    cx.notify();
                });
            },
        );

        let can_save = self.can_save(cx);
        let (submit, submit_store) = (this.clone(), navigation_store.clone());
        let content = scroll_view(SharedString::from(format!(
            "chat-settings-type-{}",
            self.chat_id.as_deref().unwrap_or_default()
        )))
        .list_shell_scroll_under_chrome(cx)
        .size_full()
        .bg(theme.bg._100)
        .child(
            vstack(14.0)
                .w_full()
                .items_stretch()
                .pt(px(12.0))
                .form_primary(can_save, move |_, cx| Self::save(&submit, submit_store.clone(), cx))
                .child(
                    div().px(px(16.0)).child(
                        MainText::new(tr(if is_channel {
                            "chat_settings_type_channel_section"
                        } else {
                            "chat_settings_type_section"
                        }))
                        .px(13.0)
                        .color(theme.text.secondary),
                    ),
                )
                .child(visibility_rows)
                .child(
                    div().px(px(16.0)).child(
                        MainText::new(tr(if self.is_public {
                            "chat_settings_type_public_hint"
                        } else {
                            "chat_settings_type_private_hint"
                        }))
                        .px(13.0)
                        .color(theme.text.secondary)
                        .number_of_lines(0),
                    ),
                )
                .when(self.is_public, |column| column.child(self.public_link_editor(&theme, cx)))
                .child(div().w_full().pt(px(8.0)).child(CreateChatForwardGroupedBtns::new(is_channel, forward_blocked)))
                .child(spacer_min_h(80.0)),
        );

        let back_store = navigation_store.clone();
        let appearing = this.clone();
        let disappearing = this;
        ListShellUi::new()
            .title(SharedString::from(tr(if is_channel {
                "chat_settings_type_channel_title"
            } else {
                "chat_settings_type_title"
            })))
            .on_back(move |_: &mut Window, cx: &mut App| {
                if let Some(navigation_store) = &back_store {
                    navigation_store.update(cx, |store, cx| store.pop(cx));
                }
            })
            .right_jsx(self.save_button(navigation_store, &theme, cx))
            .content(content)
            .on_appear("chat-settings-type-appear", move |_, cx| {
                appearing.update(cx, |state, cx| state.hydrate(cx));
            })
            .and_on_disappear(move |_, cx| {
                disappearing.update(cx, |state, _| state.tag_check_task = None);
            })
    }
}
