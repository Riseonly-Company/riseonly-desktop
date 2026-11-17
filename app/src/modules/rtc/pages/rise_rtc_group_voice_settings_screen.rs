use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div, px,
};
use rise_i18n::{FormatArg, tr, tr_args};
use rise_theme::AppTheme;
use rise_ui::swiftui::button_style::DISABLED_OPACITY;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::{BorderedStyle, plain, system_button};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::stacks::{hstack, spacer_min_h, spacer_min_w, vstack};
use rise_ui::swiftui::stepper::Stepper;
use rise_ui::switch_ui::SwitchUi;
use rise_ui::{IconUi, MainText};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatChannelKind, ChatPerm, VoiceChannelConfig};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::rtc::stores::group_voice::rise_rtc_group_voice_settings_stores::{
    RTCGroupVoiceSettingsInteractionsStore, RTCGroupVoiceSettingsServicesStore, RiseRTCGroupVoiceSettingsSnapshot,
};
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;

const ESCAPE_KEY: &str = "escape";
const AUDIO_BITRATE_OPTIONS: [i64; 7] = [16, 24, 32, 48, 64, 96, 128];

pub struct RiseRTCGroupVoiceSettingsScreen {
    chat_id: String,
    services: Entity<RTCGroupVoiceSettingsServicesStore>,
    chat_services: Entity<ChatServicesStore>,
    draft: VoiceChannelConfig,
    initialized: bool,
    focus_handle: FocusHandle,
    _load_task: Task<bool>,
    _subscriptions: Vec<Subscription>,
}

impl RiseRTCGroupVoiceSettingsScreen {
    pub fn new(chat_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let services = RTCGroupVoiceSettingsServicesStore::shared(cx);
        let chat_services = ChatServicesStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);

        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_effective_permissions(&chat_id, None, false, cx);
        });
        let load_task = RTCGroupVoiceSettingsInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.load(&chat_id, cx));

        let subscriptions = vec![
            cx.observe(&services, |this, _, cx| {
                let snapshot = this.snapshot(cx);
                this.install_snapshot_if_appropriate(&snapshot, cx);
                cx.notify();
            }),
            cx.observe(&chat_services, |_, _, cx| cx.notify()),
            cx.observe(&auth, |_, _, cx| cx.notify()),
        ];

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let mut screen = Self {
            chat_id,
            services,
            chat_services,
            draft: VoiceChannelConfig::defaults(ChatChannelKind::Voice).unwrap_or(VoiceChannelConfig {
                user_limit: 0,
                speaker_limit: 0,
                audio_bitrate_kbps: 32,
                video_enabled: true,
                screen_share_enabled: true,
                default_muted: false,
                region: String::new(),
            }),
            initialized: false,
            focus_handle,
            _load_task: load_task,
            _subscriptions: subscriptions,
        };
        let snapshot = screen.snapshot(cx);
        screen.install_snapshot_if_appropriate(&snapshot, cx);
        screen
    }

    fn snapshot(&self, cx: &mut App) -> RiseRTCGroupVoiceSettingsSnapshot {
        self.services.update(cx, |services, cx| services.snapshot(&self.chat_id, cx))
    }

    fn is_dirty(&self, snapshot: &RiseRTCGroupVoiceSettingsSnapshot) -> bool {
        self.initialized && self.draft != snapshot.config
    }

    fn can_manage(&self, cx: &mut App) -> bool {
        self.chat_services.update(cx, |services, cx| services.can(&self.chat_id, ChatPerm::MANAGE_VOICE_CHANNEL, cx))
    }

    fn pop(navigation_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        if let Some(navigation_store) = navigation_store {
            navigation_store.update(cx, |store, cx| store.pop(cx));
        }
    }

    fn settings_section(&self, can_manage: bool, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity();
        let user_limit = {
            let reader = this.clone();
            let writer = this.clone();
            Binding::new(
                move |cx: &App| reader.read(cx).draft.user_limit,
                move |value: i64, _: &mut Window, cx: &mut App| {
                    writer.update(cx, |screen, cx| {
                        if value == 1 {
                            screen.draft.user_limit = if screen.draft.user_limit == 0 { 2 } else { 0 };
                        } else {
                            screen.draft.user_limit = value;
                        }
                        if screen.draft.user_limit > 0 && screen.draft.speaker_limit > screen.draft.user_limit {
                            screen.draft.speaker_limit = screen.draft.user_limit;
                        }
                        cx.notify();
                    });
                },
            )
        };
        let speaker_limit = {
            let reader = this.clone();
            let writer = this.clone();
            Binding::new(
                move |cx: &App| reader.read(cx).draft.speaker_limit,
                move |value: i64, _: &mut Window, cx: &mut App| {
                    writer.update(cx, |screen, cx| {
                        screen.draft.speaker_limit = value;
                        cx.notify();
                    });
                },
            )
        };
        let maximum_speaker_limit = VoiceChannelConfig::maximum_speaker_limit();
        let speaker_ceiling = maximum_speaker_limit.min(if self.draft.user_limit == 0 {
            maximum_speaker_limit
        } else {
            self.draft.user_limit
        });

        vstack(0.0)
            .relative()
            .w_full()
            .rounded(theme.radius._300)
            .bg(theme.bg._200)
            .child(self.settings_row(
                tr("rtc_user_limit"),
                Some(self.limit_label(self.draft.user_limit)),
                theme,
                Stepper::new("rtc-voice-user-limit", user_limit, 0..=VoiceChannelConfig::maximum_user_limit())
                    .disabled(!can_manage),
            ))
            .child(Self::divider(theme))
            .child(self.settings_row(
                tr("rtc_speaker_limit"),
                Some(self.limit_label(self.draft.speaker_limit)),
                theme,
                Stepper::new("rtc-voice-speaker-limit", speaker_limit, 0..=speaker_ceiling).disabled(!can_manage),
            ))
            .child(Self::divider(theme))
            .child(self.settings_row(tr("rtc_audio_bitrate"), None, theme, Self::dimmed_control(can_manage, self.bitrate_picker(theme, cx))))
            .child(Self::divider(theme))
            .child(self.toggle_row(
                "rtc-voice-video",
                tr("rtc_video_enabled"),
                can_manage,
                self.draft_toggle(|draft| draft.video_enabled, |draft, value| draft.video_enabled = value, cx),
            ))
            .child(Self::divider(theme))
            .child(self.toggle_row(
                "rtc-voice-screen-share",
                tr("rtc_screen_share_enabled"),
                can_manage,
                self.draft_toggle(
                    |draft| draft.screen_share_enabled,
                    |draft, value| draft.screen_share_enabled = value,
                    cx,
                ),
            ))
            .child(Self::divider(theme))
            .child(self.toggle_row(
                "rtc-voice-default-muted",
                tr("rtc_default_muted"),
                can_manage,
                self.draft_toggle(|draft| draft.default_muted, |draft, value| draft.default_muted = value, cx),
            ))
            .when(!can_manage, |section| section.child(div().absolute().inset_0().occlude()))
            .into_any_element()
    }

    fn draft_toggle(
        &self,
        get: fn(&VoiceChannelConfig) -> bool,
        set: fn(&mut VoiceChannelConfig, bool),
        cx: &mut Context<Self>,
    ) -> Binding<bool> {
        let reader = cx.entity();
        let writer = reader.clone();
        Binding::new(
            move |cx: &App| get(&reader.read(cx).draft),
            move |value: bool, _: &mut Window, cx: &mut App| {
                writer.update(cx, |screen, cx| {
                    set(&mut screen.draft, value);
                    cx.notify();
                });
            },
        )
    }

    fn bitrate_picker(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let is_premium = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .is_some_and(|profile| profile.more.is_premium);
        let bitrate_ceiling = VoiceChannelConfig::audio_bitrate_ceiling_kbps(is_premium);
        let selected = self.draft.audio_bitrate_kbps.min(bitrate_ceiling);
        let this = cx.entity();

        let items = AUDIO_BITRATE_OPTIONS
            .into_iter()
            .filter(|value| *value <= VoiceChannelConfig::maximum_audio_bitrate_kbps())
            .map(|value| {
                let screen = this.clone();
                ContextMenuItem::new(
                    value,
                    Self::bitrate_title(value),
                    (value > bitrate_ceiling).then(|| "lock.fill".to_owned()),
                    value.to_string(),
                    move |_: &mut Window, cx: &mut App| {
                        let selected = RTCInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.select_audio_bitrate(value, cx));
                        if !selected {
                            return;
                        }
                        screen.update(cx, |screen, cx| {
                            screen.draft.audio_bitrate_kbps = value;
                            cx.notify();
                        });
                    },
                )
            })
            .collect();

        ContextMenuUi::new(
            "rtc-voice-audio-bitrate",
            items,
            hstack(4.0)
                .child(MainText::new(Self::bitrate_title(selected)).px(14.0).color(theme.primary._100))
                .children(IconUi::sized("chevron.up.chevron.down", px(11.0), theme.primary._100, false)),
        )
        .selected(selected.to_string())
        .shows_item_icons()
    }

    fn bitrate_title(value: i64) -> String {
        tr_args("rtc_audio_bitrate_kbps", &[FormatArg::Int(value)])
    }

    fn settings_row(
        &self,
        title: String,
        value: Option<String>,
        theme: &AppTheme,
        accessory: impl IntoElement,
    ) -> impl IntoElement {
        hstack(12.0)
            .w_full()
            .px(px(14.0))
            .min_h(px(58.0))
            .child(
                vstack(2.0)
                    .min_w_0()
                    .flex_shrink_1()
                    .items_start()
                    .child(MainText::new(title).px(14.0).font_weight(FontWeight::MEDIUM))
                    .when_some(value, |column, value| {
                        column.child(MainText::new(value).px(12.0).color(theme.text.secondary))
                    }),
            )
            .child(spacer_min_w(8.0))
            .child(div().flex_none().child(accessory))
    }

    /// dims the control and leaves its label alone when the user cant manage
    fn dimmed_control(can_manage: bool, control: impl IntoElement) -> impl IntoElement {
        div().when(!can_manage, |wrapper| wrapper.opacity(DISABLED_OPACITY)).child(control)
    }

    fn toggle_row(&self, id: &'static str, title: String, can_manage: bool, value: Binding<bool>) -> impl IntoElement {
        hstack(12.0)
            .w_full()
            .px(px(14.0))
            .min_h(px(54.0))
            .child(div().min_w_0().flex_shrink_1().child(MainText::new(title).px(14.0).font_weight(FontWeight::MEDIUM)))
            .child(spacer_min_w(8.0))
            .child(div().flex_none().child(Self::dimmed_control(can_manage, SwitchUi::new(id, value))))
    }

    fn divider(theme: &AppTheme) -> impl IntoElement {
        div().w_full().pl(px(14.0)).child(div().w_full().h(px(0.5)).bg(theme.border._100.opacity(0.55)))
    }

    fn save_button(
        &self,
        snapshot: &RiseRTCGroupVoiceSettingsSnapshot,
        navigation_store: Option<Entity<NavigationStore>>,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let is_dirty = self.is_dirty(snapshot);
        let is_saving = snapshot.is_saving;
        let tint = if is_dirty { theme.primary._100 } else { theme.text.secondary };
        let this = cx.entity();

        plain("rtc-voice-save")
            .disabled(!is_dirty || is_saving)
            .on_press(move |_, cx| {
                if !this.update(cx, |screen, cx| screen.can_manage(cx)) {
                    return;
                }
                let (chat_id, draft) = {
                    let screen = this.read(cx);
                    (screen.chat_id.clone(), screen.draft.clone())
                };
                let save = RTCGroupVoiceSettingsInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.save(&chat_id, draft, cx));
                let navigation_store = navigation_store.clone();
                cx.spawn(async move |cx| match save.await {
                    Ok(()) => {
                        cx.update(|cx| Self::pop(&navigation_store, cx));
                    }
                    Err(_) => {
                        cx.update(|cx| {
                            Notifier::send(
                                tr("rtc_group_voice_save_failed"),
                                NotifierSendOptions {
                                    r#type: NotifierType::Error,
                                    position: NotifierPosition::Bottom,
                                    ..NotifierSendOptions::default()
                                },
                                cx,
                            );
                        });
                    }
                })
                .detach();
            })
            .size(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .map(|button| {
                if is_saving {
                    button.child(progress_view())
                } else {
                    button.children(IconUi::sized("checkmark", px(16.0), tint, false))
                }
            })
    }

    fn install_snapshot_if_appropriate(&mut self, value: &RiseRTCGroupVoiceSettingsSnapshot, cx: &mut Context<Self>) {
        if !value.is_loaded {
            return;
        }
        let is_dirty = self.is_dirty(&self.snapshot(cx));
        if self.initialized && is_dirty && !value.is_saving {
            return;
        }
        self.draft = value.config.clone();
        self.initialized = true;
    }

    fn limit_label(&self, value: i64) -> String {
        if value == 0 { tr("rtc_unlimited") } else { value.to_string() }
    }
}

impl Render for RiseRTCGroupVoiceSettingsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let navigation_store = env::<NavigationStoreKey>(cx);
        let snapshot = self.snapshot(cx);
        let can_manage = self.can_manage(cx);
        let chat_id = self.chat_id.clone();
        let back_store = navigation_store.clone();
        let escape_store = navigation_store.clone();

        let mut wrapper = SettingsWrapperUi::new(tr("rtc_group_voice_settings"), move |_: &mut Window, cx: &mut App| {
            Self::pop(&back_store, cx);
        });
        if can_manage {
            wrapper = wrapper.right_jsx(self.save_button(&snapshot, navigation_store, &theme, cx));
        }

        let section = snapshot.is_loaded.then(|| self.settings_section(can_manage, &theme, cx));
        let content = scroll_view(SharedString::from(format!("rtc-voice-settings-{chat_id}")))
            .list_shell_scroll_under_chrome(cx)
            .bg(theme.bg._100)
            .child(
                vstack(18.0)
                    .w_full()
                    .px(px(12.0))
                    .pt(px(16.0))
                    .children(section)
                    .map(|column| {
                        if snapshot.is_loading {
                            column.child(div().pt(px(12.0)).child(progress_view()))
                        } else if snapshot.error_code.is_some() {
                            column.child(
                                system_button("rtc-voice-retry", BorderedStyle::Bordered)
                                    .tint(theme.primary._100)
                                    .on_press(move |_, cx| {
                                        RTCGroupVoiceSettingsInteractionsStore::shared(cx)
                                            .update(cx, |interactions, cx| interactions.load(&chat_id, cx))
                                            .detach();
                                    })
                                    .child(MainText::new(tr("rtc_retry")).px(14.0).color(theme.primary._100)),
                            )
                        } else {
                            column
                        }
                    })
                    .child(spacer_min_h(100.0)),
            );

        div()
            .id("rtc-group-voice-settings")
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == ESCAPE_KEY {
                    Self::pop(&escape_store, cx);
                    cx.stop_propagation();
                }
            })
            .child(wrapper.content(content))
    }
}
