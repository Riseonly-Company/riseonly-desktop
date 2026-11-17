use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyWindowHandle, App, Context, ElementId, Entity, FontWeight, SharedString, Subscription, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::layer_keys::LayerKeysExt;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::input_ui::InputUi;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::ChatWallpaperPermissions;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_profile_more_sheets::TopicNotificationSheet;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::chat::pages::chats::chat::chat_topics::channel_kind_editor::{
    VoiceChannelConfigDraft, VoiceChannelConfigEditor,
};
use crate::modules::chat::pages::chats::chat::forum_home::channel_visibility_editor::{
    ChannelVisibilityEditor, VisibilityTarget, VisibilityTargetKind,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatChannelManagementPolicy, ChatPerm, ChatPermExt, ChatTopic,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTOExt;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const NAME_FIELD: &str = "name";
const WALLPAPER_PICKER_PUSH_DELAY: Duration = Duration::from_millis(320);

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// editing uses the same endpoint and pointer as topic creation
fn title_rule() -> RiseFieldRule {
    RiseFieldRule::new("chat_update_topic", "/title")
}

struct TopicSettingsSheetState {
    name: HashMap<String, String>,
    voice_draft: VoiceChannelConfigDraft,
    show_visibility: bool,
    show_notifications: bool,
    _services_subscription: Subscription,
}

impl TopicSettingsSheetState {
    fn new(chat_id: &str, topic_id: &str, cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let subscription = cx.observe(&services, |_, _, cx| cx.notify());
        let (name, voice_draft) = Self::seed(chat_id, topic_id, cx);
        Self {
            name,
            voice_draft,
            show_visibility: false,
            show_notifications: false,
            _services_subscription: subscription,
        }
    }

    fn seed(chat_id: &str, topic_id: &str, cx: &mut App) -> (HashMap<String, String>, VoiceChannelConfigDraft) {
        let topic = ChatServicesStore::shared(cx).update(cx, |store, cx| store.topic_from_any_cache(chat_id, topic_id, cx));
        let title = topic.as_ref().map(|topic| topic.title.clone()).unwrap_or_default();
        (
            HashMap::from([(NAME_FIELD.to_owned(), title)]),
            VoiceChannelConfigDraft::new(topic.and_then(|topic| topic.voice_config)),
        )
    }

    fn raw_name(&self) -> &str {
        self.name.get(NAME_FIELD).map_or("", String::as_str)
    }
}

#[derive(Clone)]
struct TopicSettingsSheetParts {
    id: ElementId,
    is_presented: Binding<bool>,
    chat_id: String,
    topic_id: String,
    on_deleted: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    state: Entity<TopicSettingsSheetState>,
    nav_store: Option<Entity<NavigationStore>>,
    window: AnyWindowHandle,
}

impl TopicSettingsSheetParts {
    fn topic(&self, cx: &mut App) -> Option<ChatTopic> {
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.topic_from_any_cache(&self.chat_id, &self.topic_id, cx))
    }

    fn title(&self, cx: &mut App) -> String {
        self.topic(cx).map(|topic| topic.title).unwrap_or_default()
    }

    /// channel kind isnt editable so it is read from the topic and not mirrored in state
    fn channel_kind(&self, cx: &mut App) -> ChatChannelKind {
        self.topic(cx).map_or(ChatChannelKind::Text, |topic| topic.channel_kind)
    }

    fn permission_bits(&self, cx: &mut App) -> i64 {
        ChatServicesStore::shared(cx).update(cx, |store, cx| {
            store
                .effective_permissions(&self.chat_id, Some(self.topic_id.as_str()), cx)
                .or_else(|| store.effective_permissions(&self.chat_id, None, cx))
                .map_or(0, |permissions| permissions.mask())
        })
    }

    fn allowed_channel_kinds(&self, cx: &mut App) -> Vec<ChatChannelKind> {
        let source = self.topic(cx).map(|topic| topic.channel_kind);
        ChatChannelManagementPolicy::allowed_destination_kinds(source, self.permission_bits(cx))
    }

    fn can_edit_visibility(&self, cx: &mut App) -> bool {
        ChatServicesStore::shared(cx).update(cx, |store, cx| {
            let role = store.chat_info(&self.chat_id, cx).and_then(|chat| chat.role);
            let permissions =
                store.effective_permissions(&self.chat_id, None, cx).map_or(0, |permissions| permissions.mask());
            ChatPerm::is_owner_role(role.as_deref()) || ChatPerm::has(permissions, ChatPerm::MANAGE_ROLES)
        })
    }

    fn can_edit_voice_config(&self, cx: &mut App) -> bool {
        self.topic(cx).is_some_and(|topic| !topic.is_general)
            && self.channel_kind(cx).is_rtc()
            && !self.allowed_channel_kinds(cx).is_empty()
    }

    /// server rejects a voice channel without a valid config and region is free text
    /// disable the button instead of a silent refusal, same for a name the field marks invalid
    fn can_save(&self, cx: &mut App) -> bool {
        let kind = self.channel_kind(cx);
        let state = self.state.read(cx);
        let raw = state.raw_name();
        if trimming_whitespaces_and_newlines(raw).is_empty() || !title_rule().is_satisfied(raw) {
            return false;
        }
        !kind.is_rtc() || state.voice_draft.normalized(kind).is_some()
    }

    fn save(&self, window: &mut Window, cx: &mut App) {
        let (trimmed, draft) = {
            let state = self.state.read(cx);
            (trimming_whitespaces_and_newlines(state.raw_name()).to_owned(), state.voice_draft.clone())
        };
        let current = self.topic(cx);
        let draft_config = draft.normalized(self.channel_kind(cx));
        let config_changed = self.can_edit_voice_config(cx)
            && current.as_ref().and_then(|topic| topic.voice_config.as_ref()) != draft_config.as_ref();
        let title_changed = !trimmed.is_empty() && trimmed != current.map(|topic| topic.title).unwrap_or_default();
        if !title_changed && !config_changed {
            self.is_presented.set(false, window, cx);
            return;
        }
        if !self.can_save(cx) {
            return;
        }

        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_topic(
                &self.chat_id,
                &self.topic_id,
                title_changed.then_some(trimmed.as_str()),
                None,
                None,
                draft_config,
                None,
                cx,
            );
        });
        self.is_presented.set(false, window, cx);
    }

    fn confirm_delete(&self, cx: &mut App) {
        let parts = self.clone();
        NativeConfirmAlert::present_delete(
            &tr("topic_settings_delete_confirm"),
            "",
            Some(tr("chat_topics_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.delete_topic(&parts.chat_id, &parts.topic_id, None, cx);
                });
                let _ = parts.window.update(cx, |_, window, cx| {
                    parts.is_presented.set(false, window, cx);
                    if let Some(on_deleted) = &parts.on_deleted {
                        on_deleted(window, cx);
                    }
                });
            },
            cx,
        );
    }

    fn open_wallpaper_picker(&self, window: &mut Window, cx: &mut App) {
        self.is_presented.set(false, window, cx);
        let Some(nav_store) = self.nav_store.clone() else {
            return;
        };
        let route = RootRoute::ChatWallpaperPicker {
            chat_id: Some(self.chat_id.clone()),
            topic_id: Some(self.topic_id.clone()),
        };
        cx.spawn(async move |cx| {
            cx.background_executor().timer(WALLPAPER_PICKER_PUSH_DELAY).await;
            cx.update(|cx| {
                nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
            });
        })
        .detach();
    }

    fn navigation_bar(&self, cx: &mut App) -> SheetHeaderBar {
        let can_save = self.can_save(cx);
        let dismissing = self.is_presented.clone();
        let saving = self.clone();
        SheetHeaderBar::new(named_child(&self.id, "header"), tr("topic_settings_title"), move |window, cx| {
            dismissing.set(false, window, cx);
        })
        .right_text(tr("common_save"))
        .right_enabled(can_save)
        .on_right(move |window, cx| saving.save(window, cx))
    }

    fn rename_card(&self, theme: &AppTheme, cx: &mut App) -> Div {
        let (values, raw) = {
            let state = self.state.read(cx);
            (state.name.clone(), state.raw_name().to_owned())
        };
        let state = self.state.clone();
        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                div().px(px(16.0)).child(
                    MainText::new(tr("topic_settings_name").to_uppercase()).px(12.0).color(theme.text.secondary),
                ),
            )
            .child(
                div().w_full().px(px(12.0)).child(
                    InputUi::new(named_child(&self.id, "name"))
                        .values(values)
                        .error_message(title_rule().error_text(&raw))
                        .name(NAME_FIELD)
                        .placeholder(tr("chat_topics_create_placeholder"))
                        .rule(title_rule())
                        .set_value(move |_, value, _, cx| {
                            state.update(cx, |state, cx| {
                                state.name.insert(NAME_FIELD.to_owned(), value.to_owned());
                                cx.notify();
                            });
                        }),
                ),
            )
    }

    fn visibility_card(&self, cx: &mut App) -> GroupedBtns {
        let notifications = self.state.clone();
        let mut items = vec![
            GroupedBtn::new("notifications", tr("chat_mute_advanced_title"))
                .icon("bell")
                .icon_background(ChatTileColor::blue())
                .callback(move |_, cx| {
                    notifications.update(cx, |state, cx| {
                        state.show_notifications = true;
                        cx.notify();
                    });
                }),
        ];
        if self.can_edit_visibility(cx) {
            let visibility = self.state.clone();
            items.push(
                GroupedBtn::new("visibility", tr("forum_visibility_action"))
                    .icon("lock.shield")
                    .icon_background(ChatTileColor::indigo())
                    .callback(move |_, cx| {
                        visibility.update(cx, |state, cx| {
                            state.show_visibility = true;
                            cx.notify();
                        });
                    }),
            );
        }
        let chat = ChatServicesStore::shared(cx).read(cx).chat_info(&self.chat_id, cx);
        if chat.is_some_and(|chat| ChatWallpaperPermissions::can_set(&chat, cx)) {
            let parts = self.clone();
            items.push(
                GroupedBtn::new("wallpaper", tr("wallpaper_topic_title"))
                    .icon("photo.on.rectangle.angled")
                    .icon_background(ChatTileColor::orange())
                    .callback(move |window, cx| parts.open_wallpaper_picker(window, cx)),
            );
        }
        GroupedBtns::new(named_child(&self.id, "actions"), items)
    }

    fn delete_card(&self, theme: &AppTheme) -> Div {
        let deleting = self.clone();
        div().w_full().px(px(12.0)).child(
            plain(named_child(&self.id, "delete"))
                .w_full()
                .min_h(px(50.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.bg._200)
                .capsule()
                .on_press(move |_, cx| deleting.confirm_delete(cx))
                .child(
                    MainText::new(tr("chat_topics_delete"))
                        .px(15.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.semantic.error_100),
                ),
        )
    }

    fn visibility_sheet(&self) -> NativeBottomSheetPresentationHost {
        let parts = self.clone();
        NativeBottomSheetPresentationHost::new(
            named_child(&self.id, "visibility-sheet"),
            Binding::entity(
                self.state.clone(),
                |state: &TopicSettingsSheetState| state.show_visibility,
                |state, value, cx| {
                    state.show_visibility = value;
                    cx.notify();
                },
            ),
            move |_, cx| {
                ChannelVisibilityEditor::new(
                    parts.chat_id.clone(),
                    VisibilityTarget {
                        kind: VisibilityTargetKind::Topic,
                        id: parts.topic_id.clone(),
                        title: parts.title(cx),
                    },
                )
            },
        )
        .presentation_detents([PresentationDetent::Large])
    }

    fn notifications_sheet(&self) -> NativeBottomSheetPresentationHost {
        let (chat_id, topic_id) = (self.chat_id.clone(), self.topic_id.clone());
        NativeBottomSheetPresentationHost::new(
            named_child(&self.id, "notifications-sheet"),
            Binding::entity(
                self.state.clone(),
                |state: &TopicSettingsSheetState| state.show_notifications,
                |state, value, cx| {
                    state.show_notifications = value;
                    cx.notify();
                },
            ),
            move |_, _| TopicNotificationSheet::new(chat_id.clone(), topic_id.clone()),
        )
        .presentation_drag_indicator(true)
    }

    fn body(&self, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let channel_kind = self.channel_kind(cx);
        let voice_editor = self.can_edit_voice_config(cx).then(|| {
            VoiceChannelConfigEditor::new(
                Binding::entity(
                    self.state.clone(),
                    |state: &TopicSettingsSheetState| state.voice_draft.clone(),
                    |state, draft, cx| {
                        state.voice_draft = draft;
                        cx.notify();
                    },
                ),
                channel_kind,
            )
        });
        let can_save = self.can_save(cx);
        let submit = self.clone();
        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .form_primary(can_save, move |window, cx| submit.save(window, cx))
            .child(self.navigation_bar(cx))
            .child(
                div().w_full().flex_1().min_h_0().child(
                    scroll_view(named_child(&self.id, "scroll")).size_full().child(
                        vstack(18.0)
                            .w_full()
                            .pt(px(14.0))
                            .child(self.rename_card(&theme, cx))
                            .children(voice_editor)
                            .child(self.visibility_card(cx))
                            .child(self.delete_card(&theme))
                            .child(spacer_min_h(20.0)),
                    ),
                ),
            )
            .child(self.visibility_sheet())
            .child(self.notifications_sheet())
            .into_any_element()
    }
}

/// topic settings sheet, holds its own presentation so the caller renders it whenever the topic exists
#[derive(IntoElement)]
pub struct TopicSettingsSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    chat_id: String,
    topic_id: String,
    on_deleted: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl TopicSettingsSheet {
    pub fn new(id: impl Into<ElementId>, is_presented: Binding<bool>, chat_id: String, topic_id: String) -> Self {
        Self { id: id.into(), is_presented, chat_id, topic_id, on_deleted: None }
    }

    pub fn on_deleted(mut self, on_deleted: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_deleted = Some(Rc::new(on_deleted));
        self
    }
}

impl RenderOnce for TopicSettingsSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (seed_chat_id, seed_topic_id) = (self.chat_id.clone(), self.topic_id.clone());
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, move |_, cx| {
            TopicSettingsSheetState::new(&seed_chat_id, &seed_topic_id, cx)
        });

        let presented = self.is_presented.get(cx);
        let (reset_state, reset_chat_id, reset_topic_id) = (state.clone(), self.chat_id.clone(), self.topic_id.clone());
        on_change(named_child(&self.id, "presented"), presented, true, window, cx, move |_, presented: &bool, _, cx| {
            if !*presented {
                return;
            }
            let (name, voice_draft) = TopicSettingsSheetState::seed(&reset_chat_id, &reset_topic_id, cx);
            reset_state.update(cx, |state, cx| {
                state.name = name;
                state.voice_draft = voice_draft;
                state.show_visibility = false;
                state.show_notifications = false;
                cx.notify();
            });
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.load_effective_permissions(&reset_chat_id, Some(reset_topic_id.as_str()), false, cx);
            });
        });

        let parts = TopicSettingsSheetParts {
            id: self.id.clone(),
            is_presented: self.is_presented.clone(),
            chat_id: self.chat_id,
            topic_id: self.topic_id,
            on_deleted: self.on_deleted,
            state,
            nav_store: env::<NavigationStoreKey>(cx),
            window: window.window_handle(),
        };
        NativeBottomSheetPresentationHost::new(named_child(&self.id, "sheet"), self.is_presented, move |_, cx| parts.body(cx))
            .presentation_detents([PresentationDetent::Large])
    }
}
