use std::collections::HashMap;

use gpui::{AnyWindowHandle, App, Axis, Context, Entity, ExternalPaths, FontWeight, SharedString, Subscription, Task, Window};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::keyboard_dismiss::KeyboardDismiss;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::environment_values::IsEnabledKey;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerBitmapLoader;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::select_media_ui::{SelectMediaUi, SelectMediaUploadSwap};
use crate::core::ui::settings_field_row::SettingsFieldRow;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::shared::bot_ui_validation::BotChatManagement;
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotTagRules;
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::ChatWallpaperPermissions;
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::{ChatCountFormatter, ChatTileColor};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatInfo, ChatPerm, ChatPermExt, ChatReactionsType, ChatType,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::pages::profile::settings::profile_settings::profile_avatar_picker_block::ProfileAvatarPickerBlock;

const TITLE_KEY: &str = "title";
const DESCRIPTION_KEY: &str = "description";
const UPDATE_CHAT_ENDPOINT: &str = "chat_update_chat";
const TITLE_POINTER: &str = "/title";
const DESCRIPTION_POINTER: &str = "/description";
const MISSING_CHAT_TITLE: &str = "\u{2014}";
const LOGO_SIZE: f32 = 96.0;
const LOGO_ICON_SIZE: f32 = 36.0;

type NavStore = Option<Entity<NavigationStore>>;

fn page_id(chat_id: Option<&str>, part: &str) -> SharedString {
    SharedString::from(format!("chat-settings-{part}-{}", chat_id.unwrap_or_default()))
}

fn push_action(nav_store: &NavStore, route: RootRoute) -> impl Fn(&mut Window, &mut App) + 'static + use<> {
    let nav_store = nav_store.clone();
    move |_, cx| {
        if let Some(nav_store) = &nav_store {
            let route = route.clone();
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }
}

fn is_community_type(chat: &ChatInfo) -> bool {
    chat.r#type == ChatType::Group || chat.r#type == ChatType::Channel
}

pub struct ChatSettings {
    chat_id: Option<String>,
    values: HashMap<String, String>,
    loaded_for_id: String,
    logo_file: Option<SaiWsOptimisticLocalFile>,
    logo_preview: Option<CleverImage>,
    logo_uploading: bool,
    logo_progress: f64,
    logo_upload_swap: SelectMediaUploadSwap,
    window_handle: Option<AnyWindowHandle>,
    meta_saved: bool,
    title_rule: RiseFieldRule,
    description_rule: RiseFieldRule,
    chat_services: Entity<ChatServicesStore>,
    load_task: Option<Task<()>>,
    dropped_logo_task: Option<Task<()>>,
    _subscriptions: [Subscription; 3],
}

impl ChatSettings {
    pub fn new(chat_id: Option<String>, cx: &mut Context<Self>) -> Self {
        let chat_services = ChatServicesStore::shared(cx);
        let chat_store = ChatActionsStore::shared(cx);
        let bot_services = BotServicesStore::shared(cx);
        let subscriptions = [
            cx.observe(&chat_services, |_, _, cx| cx.notify()),
            cx.observe(&chat_store, |_, _, cx| cx.notify()),
            cx.observe(&bot_services, |_, _, cx| cx.notify()),
        ];
        Self {
            chat_id,
            values: HashMap::new(),
            loaded_for_id: String::new(),
            logo_file: None,
            logo_preview: None,
            logo_uploading: false,
            logo_progress: 0.0,
            logo_upload_swap: SelectMediaUploadSwap::new(cx.entity_id().as_u64() ^ (0x53 << 56)),
            window_handle: None,
            meta_saved: false,
            title_rule: RiseFieldRule::new(UPDATE_CHAT_ENDPOINT, TITLE_POINTER),
            description_rule: RiseFieldRule::new(UPDATE_CHAT_ENDPOINT, DESCRIPTION_POINTER),
            chat_services,
            load_task: None,
            dropped_logo_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn cached_chat(&self, cx: &App) -> Option<ChatInfo> {
        let id = self.chat_id.as_deref().filter(|id| !id.is_empty())?;
        self.chat_services.read(cx).chat_info(id, cx)
    }

    fn resolved_chat(&self, cx: &App) -> ChatInfo {
        if let Some(cached_chat) = self.cached_chat(cx) {
            return cached_chat;
        }
        ChatInfo {
            id: self.chat_id.clone().unwrap_or_default(),
            title: self.chat_id.clone().unwrap_or_else(|| MISSING_CHAT_TITLE.to_owned()),
            ..ChatInfo::default()
        }
    }

    fn chat_load_identity(chat: &ChatInfo, is_cached: bool) -> String {
        format!("{}|{}|{}", chat.id, is_cached, chat.r#type.wire())
    }

    fn value(&self, key: &str) -> &str {
        self.values.get(key).map(String::as_str).unwrap_or_default()
    }

    fn has_changes(&self, chat: &ChatInfo) -> bool {
        let title = trimming_whitespaces_and_newlines(self.value(TITLE_KEY));
        let desc = self.value(DESCRIPTION_KEY);
        (!title.is_empty() && title != chat.title) || desc != chat.description.as_deref().unwrap_or_default()
    }

    fn title_error_message(&self) -> Option<String> {
        let title = self.value(TITLE_KEY);
        if title.is_empty() {
            return None;
        }
        self.title_rule.error_text(title)
    }

    fn description_error_message(&self) -> Option<String> {
        let description = self.value(DESCRIPTION_KEY);
        if description.is_empty() {
            return None;
        }
        self.description_rule.error_text(description)
    }

    fn can_save_meta(&self, chat: &ChatInfo) -> bool {
        self.has_changes(chat) && self.title_error_message().is_none()
    }

    fn owned_bot_id(chat: &ChatInfo, cx: &mut App) -> Option<String> {
        if !chat.is_bot_chat() {
            return None;
        }
        let bot_tag = BotChatManagement::normalized_tag(chat);
        if !BotTagRules::is_valid(&bot_tag) {
            return None;
        }
        BotServicesStore::bot_by_tag(&bot_tag, cx).map(|bot| bot.id)
    }

    fn redirect_owned_bot_if_needed(this: &Entity<Self>, nav_store: &NavStore, cx: &mut App) {
        let chat = this.read(cx).resolved_chat(cx);
        if !chat.is_bot_chat() {
            return;
        }
        let bot_tag = BotChatManagement::normalized_tag(&chat);
        if !BotTagRules::is_valid(&bot_tag) {
            return;
        }
        let owned_bot = BotServicesStore::bot_by_tag(&bot_tag, cx);
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        if BotChatManagement::can_manage(&chat, &current_user_id, owned_bot.as_ref()) {
            if let Some(bot_id) = owned_bot.as_ref().map(|bot| bot.id.clone()).filter(|id| !id.is_empty()) {
                if let Some(nav_store) = nav_store {
                    nav_store.update(cx, |nav_store, cx| nav_store.replace_top(RootRoute::BotSettings { bot_id }, cx));
                }
                return;
            }
        }
        if owned_bot.is_none() {
            BotInteractionsStore::shared(cx).update(cx, |store, cx| store.load_bot_by_tag(&bot_tag, cx));
        }
    }

    fn run_load_task(this: &Entity<Self>, chat_load_identity: String, cx: &mut App) {
        let (cached_chat, chat, already_loaded) = {
            let state = this.read(cx);
            (state.cached_chat(cx), state.resolved_chat(cx), state.loaded_for_id == chat_load_identity)
        };
        this.update(cx, |state, _| state.load_task = None);
        let Some(loaded_chat) = cached_chat else {
            if !chat.id.is_empty() {
                ChatActionsStore::shared(cx).update(cx, |store, cx| store.get_chat_by_id_action(&chat.id, false, cx));
            }
            return;
        };
        if already_loaded {
            return;
        }
        this.update(cx, |state, cx| {
            state.loaded_for_id = chat_load_identity;
            state.values.insert(TITLE_KEY.to_owned(), loaded_chat.title.clone());
            state.values.insert(DESCRIPTION_KEY.to_owned(), loaded_chat.description.clone().unwrap_or_default());
            cx.notify();
        });
        if is_community_type(&loaded_chat) {
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.load_chat_settings(&loaded_chat.id, cx);
                store.load_linked_chat(&loaded_chat.id, cx);
                store.load_effective_permissions(&loaded_chat.id, None, false, cx);
            });
        }
        if !is_community_type(&chat) || this.read(cx).logo_preview.is_some() {
            return;
        }
        let Some(url_str) = AvatarPalette::resolved_logo_url(chat.logo_url.as_deref()) else {
            return;
        };
        this.update(cx, |state, cx| {
            state.load_task = Some(cx.spawn(async move |state, cx| {
                let img = MediaViewerBitmapLoader::load_into_memory(
                    &url_str,
                    true,
                    true,
                    CleverImageCachePolicy::Standard,
                    &*cx,
                )
                .await;
                if let Some(img) = img {
                    let _ = state.update(cx, |state, cx| {
                        state.logo_preview = Some(img);
                        cx.notify();
                    });
                }
            }));
        });
    }

    fn logo_file_did_change(this: &Entity<Self>, chat_id: &str, cx: &mut App) {
        let Some(file) = this.read(cx).logo_file.clone() else {
            return;
        };
        this.update(cx, |state, cx| {
            state.logo_uploading = true;
            state.logo_progress = 0.0;
            cx.notify();
        });
        let on_progress = this.downgrade();
        let on_error = this.downgrade();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_chat_logo(
                chat_id,
                file,
                Some(std::rc::Rc::new(move |progress: f64, cx: &mut App| {
                    let _ = on_progress.update(cx, |state, cx| {
                        state.logo_progress = progress;
                        if progress >= 100.0 {
                            state.logo_uploading = false;
                            cx.notify();
                            return;
                        }
                        let swap = state.logo_upload_swap.clone();
                        let swapped = state.window_handle.is_some_and(|handle| {
                            handle.update(cx, |_, window, _| swap.swap(progress, window)).unwrap_or(false)
                        });
                        if !swapped {
                            cx.notify();
                        }
                    });
                })),
                Some(Box::new(move |cx: &mut App| {
                    let _ = on_error.update(cx, |state, cx| {
                        state.logo_uploading = false;
                        state.logo_preview = None;
                        state.logo_file = None;
                        cx.notify();
                    });
                })),
                cx,
            );
        });
    }

    fn import_dropped_logo(this: &Entity<Self>, paths: &ExternalPaths, cx: &mut App) {
        if this.read(cx).logo_uploading {
            return;
        }
        let Some((path, (url, filename, content_type))) = paths
            .paths()
            .iter()
            .find_map(|path| ProfileAvatarPickerBlock::local_image_file(path).map(|file| (path.clone(), file)))
        else {
            return;
        };
        this.update(cx, |state, cx| {
            state.logo_file = Some(SaiWsOptimisticLocalFile::new(url, filename, content_type));
            state.dropped_logo_task = Some(cx.spawn(async move |state, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move {
                        std::fs::read(path).ok().and_then(|data| CleverImageBitmapDecoder::decode(&data, None))
                    })
                    .await;
                if let Some(image) = decoded {
                    let _ = state.update(cx, |state, cx| {
                        state.logo_preview = Some(image);
                        cx.notify();
                    });
                }
            }));
            cx.notify();
        });
    }

    fn save_button(this: &Entity<Self>, nav_store: &NavStore, can_save_meta: bool, cx: &App) -> LiquidGlassButton {
        let theme = rise_ui::theme(cx);
        let this = this.clone();
        let nav_store = nav_store.clone();
        LiquidGlassButton::new("chat-settings-save", LiquidGlassButtonShape::Capsule)
            .horizontal_padding(16.0)
            .vertical_padding(10.0)
            .disabled(!can_save_meta)
            .on_press(move |_: &mut Window, cx: &mut App| Self::save_meta(&this, &nav_store, cx))
            .child(
                MainText::new(tr("common_done"))
                    .px(15.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(if can_save_meta { theme.primary._100 } else { theme.text.secondary }),
            )
    }

    fn metadata_items(&self, this: &Entity<Self>, chat: &ChatInfo, editable: bool) -> Vec<GroupedBtn> {
        let is_channel = chat.r#type == ChatType::Channel;
        let mut items = vec![
            GroupedBtn::new(TITLE_KEY, "").content_slot(
                SettingsFieldRow::new(
                    page_id(self.chat_id.as_deref(), "title-field"),
                    "textformat",
                    ChatTileColor::blue(),
                    tr(if is_channel { "create_channel_chat_name_placeholder" } else { "chat_group_title_placeholder" }),
                    self.title_rule.binding(Self::value_binding(this, TITLE_KEY, editable)),
                )
                .error_message(self.title_error_message())
                .rule(self.title_rule),
            ),
        ];
        if editable || !self.value(DESCRIPTION_KEY).is_empty() {
            items.push(
                GroupedBtn::new(DESCRIPTION_KEY, "").content_slot(
                    SettingsFieldRow::new(
                        page_id(self.chat_id.as_deref(), "description-field"),
                        "text.alignleft",
                        ChatTileColor::teal(),
                        tr(if is_channel {
                            "create_channel_chat_description_placeholder"
                        } else {
                            "chat_group_description_placeholder"
                        }),
                        self.description_rule.binding(Self::value_binding(this, DESCRIPTION_KEY, editable)),
                    )
                    .error_message(self.description_error_message())
                    .rule(self.description_rule)
                    .axis(Axis::Vertical),
                ),
            );
        }
        items
    }

    /// clamping to the ceiling stays with the canon rule, binding wraps this access
    /// the shaped field ignores isEnabled so read only is a setter that drops the edit
    fn value_binding(this: &Entity<Self>, key: &'static str, editable: bool) -> Binding<String> {
        let reader = this.clone();
        let writer = this.clone();
        Binding::new(
            move |cx: &App| reader.read(cx).value(key).to_owned(),
            move |value: String, _: &mut Window, cx: &mut App| {
                if !editable {
                    return;
                }
                writer.update(cx, |state, cx| {
                    state.values.insert(key.to_owned(), value);
                    cx.notify();
                });
            },
        )
    }

    fn save_meta(this: &Entity<Self>, nav_store: &NavStore, cx: &mut App) {
        let (chat_id, title, desc) = {
            let state = this.read(cx);
            let chat = state.resolved_chat(cx);
            if state.meta_saved || !state.can_save_meta(&chat) {
                return;
            }
            (
                chat.id,
                trimming_whitespaces_and_newlines(state.value(TITLE_KEY)).to_owned(),
                state.value(DESCRIPTION_KEY).to_owned(),
            )
        };
        KeyboardDismiss::perform(cx);
        let accepted = ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.update_chat(&chat_id, (!title.is_empty()).then_some(title.as_str()), Some(desc.as_str()), None, cx)
        });
        if !accepted {
            return;
        }
        this.update(cx, |state, _| state.meta_saved = true);
        let nav_store = nav_store.clone();
        cx.defer(move |cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        });
    }

    fn type_items(chat: &ChatInfo, nav_store: &NavStore) -> Vec<GroupedBtn> {
        let is_channel = chat.r#type == ChatType::Channel;
        vec![
            GroupedBtn::new("type", tr(if is_channel { "chat_settings_type_channel_title" } else { "chat_settings_type_title" }))
                .icon(if is_channel { "megaphone.fill" } else { "person.2.fill" })
                .icon_background(ChatTileColor::blue())
                .trailing(tr(if chat.is_public { "chat_settings_type_public" } else { "chat_settings_type_private" }))
                .callback(push_action(nav_store, RootRoute::ChatSettingsType { chat_id: Some(chat.id.clone()) })),
        ]
    }

    fn content_items(&self, chat: &ChatInfo, nav_store: &NavStore, cx: &mut App) -> Vec<GroupedBtn> {
        let linked_card = self.chat_services.read(cx).resolved_linked_chat(&chat.id, cx);
        let is_linked = linked_card.is_some();
        let linked_title = linked_card.map(|card| card.title).filter(|title| !title.is_empty());
        let chat_id = Some(chat.id.clone());

        let mut items = Vec::new();
        if ChatProfilePermissions::lists_invite_links(chat, cx) {
            items.push(
                GroupedBtn::new("links", tr("chat_settings_invation_links"))
                    .icon("link")
                    .icon_background(ChatTileColor::orange())
                    .callback(push_action(nav_store, RootRoute::ChatSettingsLinks { chat_id: chat_id.clone() })),
            );
        }
        if chat.r#type == ChatType::Channel {
            items.push(
                GroupedBtn::new("discussion", tr("chat_settings_discussion"))
                    .icon("bubble.left.and.bubble.right.fill")
                    .icon_background(ChatTileColor::green())
                    .trailing(linked_title.clone().unwrap_or_else(|| tr("chat_settings_discussion_off")))
                    .guide_anchor(RiseGuideAnchor::ChatSettingsDiscussion)
                    .callback(push_action(nav_store, RootRoute::ChatSettingsLinkedChat { chat_id: chat_id.clone() })),
            );
        } else if chat.r#type == ChatType::Group && is_linked {
            let mut linked = GroupedBtn::new("linked", tr("chat_settings_linked_channel"))
                .icon("megaphone.fill")
                .icon_background(ChatTileColor::orange())
                .callback(push_action(nav_store, RootRoute::ChatSettingsLinkedChat { chat_id: chat_id.clone() }));
            if let Some(linked_title) = linked_title.clone() {
                linked = linked.trailing(linked_title);
            }
            items.push(linked);
        }
        if ChatProfilePermissions::manages_reactions(chat, cx) {
            items.push(self.reactions_item(chat, nav_store, cx));
        }
        if ChatWallpaperPermissions::can_set(chat, cx) {
            items.push(
                GroupedBtn::new("wallpaper", tr("wallpaper_chat_sheet_title"))
                    .icon("photo.on.rectangle.angled")
                    .icon_background(ChatTileColor::blue())
                    .callback(push_action(
                        nav_store,
                        RootRoute::ChatWallpaperPicker { chat_id: chat_id.clone(), topic_id: None },
                    )),
            );
        }
        if chat.r#type == ChatType::Group {
            if !chat.topics_enabled
                && self.chat_services.update(cx, |services, cx| services.can(&chat.id, ChatPerm::MANAGE_VOICE_CHANNEL, cx))
            {
                items.push(
                    GroupedBtn::new("group_voice", tr("rtc_group_voice_settings"))
                        .icon("waveform.circle.fill")
                        .icon_background(ChatTileColor::green())
                        .callback(push_action(nav_store, RootRoute::GroupVoiceSettings { chat_id: chat.id.clone() })),
                );
            }
            if self.chat_services.update(cx, |services, cx| services.topics_enabled(&chat.id, cx))
                || ChatProfilePermissions::has(chat, ChatPerm::MANAGE_GROUP, cx)
            {
                items.push(Self::topics_item(chat, nav_store));
            }
        }
        if is_community_type(chat) {
            items.push(Self::roles_item(chat, nav_store));
        }
        items
    }

    fn partial_management_items(&self, chat: &ChatInfo, nav_store: &NavStore, cx: &mut App) -> Vec<GroupedBtn> {
        let mut items = Vec::new();
        if ChatProfilePermissions::manages_reactions(chat, cx) {
            items.push(self.reactions_item(chat, nav_store, cx));
        }
        if ChatProfilePermissions::manages_forum_structure(chat, cx) {
            items.push(Self::topics_item(chat, nav_store));
        }
        if ChatProfilePermissions::manages_roles(chat, cx) {
            items.push(Self::roles_item(chat, nav_store));
        }
        items.extend(Self::moderation_list_items(chat, nav_store, cx));
        items
    }

    fn topics_item(chat: &ChatInfo, nav_store: &NavStore) -> GroupedBtn {
        GroupedBtn::new("topics", tr("chat_topics_title"))
            .icon("list.bullet.rectangle.portrait.fill")
            .icon_background(ChatTileColor::indigo())
            .guide_anchor(RiseGuideAnchor::ChatSettingsTopics)
            .callback(push_action(nav_store, RootRoute::ChatTopics { chat_id: Some(chat.id.clone()) }))
    }

    fn roles_item(chat: &ChatInfo, nav_store: &NavStore) -> GroupedBtn {
        GroupedBtn::new("roles", tr("roles_title"))
            .icon("person.3.sequence.fill")
            .icon_background(ChatTileColor::indigo())
            .guide_anchor(RiseGuideAnchor::ChatSettingsRoles)
            .callback(push_action(nav_store, RootRoute::ChatRoles { chat_id: Some(chat.id.clone()) }))
    }

    fn reactions_item(&self, chat: &ChatInfo, nav_store: &NavStore, cx: &App) -> GroupedBtn {
        let item = GroupedBtn::new("reactions", tr("chat_settings_allowed_reactions"))
            .icon("heart.fill")
            .icon_background(ChatTileColor::pink())
            .callback(push_action(nav_store, RootRoute::ChatSettingsAllowedReactions { chat_id: Some(chat.id.clone()) }));
        match self.reaction_summary(chat, cx) {
            Some(summary) => item.trailing(summary),
            None => item,
        }
    }

    /// the chat row keeps the last saved list in every mode so only the settings know the mode
    fn reaction_summary(&self, chat: &ChatInfo, cx: &App) -> Option<String> {
        let settings = self.chat_services.read(cx).chat_settings(&chat.id, cx)?;
        Some(match settings.reactions_type {
            ChatReactionsType::All => tr("chat_reactions_all"),
            ChatReactionsType::None => tr("chat_reactions_none"),
            ChatReactionsType::Custom => {
                let count = settings.allowed_reactions.len();
                if count == 0 { tr("chat_reactions_none") } else { count.to_string() }
            }
        })
    }

    fn management_items(chat: &ChatInfo, nav_store: &NavStore, cx: &mut App) -> Vec<GroupedBtn> {
        let chat_id = Some(chat.id.clone());
        let is_channel = chat.r#type == ChatType::Channel;
        let mut items = vec![
            GroupedBtn::new("members", tr(if is_channel { "chat_profile_subscribers" } else { "chat_settings_members" }))
                .icon("person.2.fill")
                .icon_background(ChatTileColor::blue())
                .trailing(ChatCountFormatter::compact(chat.member_count))
                .guide_anchor(RiseGuideAnchor::ChatSettingsMembers)
                .callback(push_action(nav_store, RootRoute::ChatSettingsMembers { chat_id: chat_id.clone() })),
        ];
        if chat.r#type == ChatType::Group && ChatProfilePermissions::has(chat, ChatPerm::MANAGE_GROUP, cx) {
            items.push(
                GroupedBtn::new("permissions", tr("chat_settings_permissions"))
                    .icon("key.fill")
                    .icon_background(ChatTileColor::gray())
                    .callback(push_action(nav_store, RootRoute::ChatSettingsPermissions { chat_id: chat_id.clone() })),
            );
        }
        items.push(
            GroupedBtn::new("admins", tr("chat_settings_admins"))
                .icon("checkmark.shield.fill")
                .icon_background(ChatTileColor::green())
                .callback(push_action(nav_store, RootRoute::ChatSettingsAdmins { chat_id: chat_id.clone() })),
        );
        if is_channel && ChatProfilePermissions::has(chat, ChatPerm::VIEW_ANALYTICS, cx) {
            items.push(
                GroupedBtn::new("statistics", tr("chat_settings_statistics"))
                    .icon("chart.bar.fill")
                    .icon_background(ChatTileColor::indigo())
                    .callback(push_action(nav_store, RootRoute::ChatStatistics { chat_id })),
            );
        }
        items.extend(Self::moderation_list_items(chat, nav_store, cx));
        items
    }

    fn moderation_list_items(chat: &ChatInfo, nav_store: &NavStore, cx: &mut App) -> Vec<GroupedBtn> {
        let mut items = Vec::new();
        if ChatProfilePermissions::has(chat, ChatPerm::BAN_MEMBERS, cx) {
            items.push(
                GroupedBtn::new("blacklist", tr("chat_settings_blacklist"))
                    .icon("minus.circle.fill")
                    .icon_background(ChatTileColor::red())
                    .callback(push_action(nav_store, RootRoute::ChatSettingsBlacklist { chat_id: Some(chat.id.clone()) })),
            );
        }
        if ChatProfilePermissions::has(chat, ChatPerm::VIEW_AUDIT_LOG, cx) {
            items.push(
                GroupedBtn::new("recent", tr("chat_settings_recent_actions"))
                    .icon("eye.fill")
                    .icon_background(ChatTileColor::orange())
                    .callback(push_action(
                        nav_store,
                        RootRoute::ChatSettingsRecentActions { chat_id: Some(chat.id.clone()) },
                    )),
            );
        }
        items
    }

    fn danger_items(chat: &ChatInfo, nav_store: &NavStore) -> Vec<GroupedBtn> {
        let is_channel = chat.r#type == ChatType::Channel;
        let target = chat.clone();
        let completion_store = nav_store.clone();
        if ChatProfilePermissions::is_owner(chat) {
            return vec![
                GroupedBtn::new("delete", tr(if is_channel { "chat_delete_channel" } else { "chat_delete_group" }))
                    .icon("trash.fill")
                    .icon_background(ChatTileColor::red())
                    .destructive(true)
                    .show_chevron(false)
                    .callback(move |_: &mut Window, cx: &mut App| {
                        let completion_store = completion_store.clone();
                        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.confirm_delete_chat(
                                &target,
                                None,
                                Some(Box::new(move |cx: &mut App| Self::back_to_chats_list(&completion_store, cx))),
                                cx,
                            );
                        });
                    }),
            ];
        }
        vec![
            GroupedBtn::new("leave", tr(if is_channel { "chat_leave_channel" } else { "chat_leave_group" }))
                .icon("rectangle.portrait.and.arrow.right.fill")
                .icon_background(ChatTileColor::red())
                .destructive(true)
                .show_chevron(false)
                .callback(move |_: &mut Window, cx: &mut App| {
                    let completion_store = completion_store.clone();
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.confirm_leave_chat(
                            &target,
                            None,
                            Some(Box::new(move |cx: &mut App| Self::back_to_chats_list(&completion_store, cx))),
                            cx,
                        );
                    });
                }),
        ]
    }

    fn back_to_chats_list(nav_store: &NavStore, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |nav_store, cx| {
                nav_store.switch_tab(MainTab::Chats, cx);
                nav_store.pop_to_root(cx);
            });
        }
    }

    fn type_settings_key(chat_type: ChatType) -> &'static str {
        match chat_type {
            ChatType::Direct => "chat_private_settings_title",
            ChatType::Group => "chat_group_settings_title",
            ChatType::Channel => "chat_channel_settings_title",
            ChatType::Favourites => "chat_private_settings_title",
        }
    }

    fn logo_block(&self, this: &Entity<Self>, chat: &ChatInfo, can_edit_meta: bool) -> Div {
        let chat_id = self.chat_id.as_deref();
        let column = vstack(10.0).w_full().items_center().pt(px(14.0));
        if !can_edit_meta {
            return column.child(
                ChatLogo::new(page_id(chat_id, "logo"), chat.clone())
                    .size(LOGO_SIZE)
                    .icon_size(LOGO_ICON_SIZE)
                    .show_online_indicator(false),
            );
        }
        let file_reader = this.clone();
        let file_writer = this.clone();
        let preview_reader = this.clone();
        let preview_writer = this.clone();
        let drop_target = this.clone();
        column
            .child(
                div()
                    .on_drop(move |paths: &ExternalPaths, _: &mut Window, cx: &mut App| {
                        Self::import_dropped_logo(&drop_target, paths, cx);
                    })
                    .child(
                        SelectMediaUi::new(
                            page_id(chat_id, "select-logo"),
                            Binding::new(
                                move |cx: &App| file_reader.read(cx).logo_file.clone(),
                                move |value: Option<SaiWsOptimisticLocalFile>, _: &mut Window, cx: &mut App| {
                                    file_writer.update(cx, |state, cx| {
                                        state.logo_file = value;
                                        cx.notify();
                                    });
                                },
                            ),
                            Binding::new(
                                move |cx: &App| preview_reader.read(cx).logo_preview.clone(),
                                move |value: Option<CleverImage>, _: &mut Window, cx: &mut App| {
                                    preview_writer.update(cx, |state, cx| {
                                        state.logo_preview = value;
                                        cx.notify();
                                    });
                                },
                            ),
                        )
                        .size(LOGO_SIZE)
                        .cover_upload_progress(self.logo_progress)
                        .show_upload_overlay(self.logo_uploading)
                        .upload_swap(self.logo_upload_swap.clone())
                        .remote_source(chat.logo_url.clone())
                        .remote_name(chat.title.clone())
                        .uses_app_sheet_presentation(true)
                        .caption(tr("select_avatar_text")),
                    ),
            )
    }
}

impl Render for ChatSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        self.window_handle = Some(window.window_handle());
        let this = cx.entity();
        let nav_store: NavStore = env::<NavigationStoreKey>(cx);
        let chat_id = self.chat_id.clone();
        let cached_chat = self.cached_chat(cx);
        let chat = self.resolved_chat(cx);
        let is_community = is_community_type(&chat);
        let can_edit_meta = ChatProfilePermissions::can_edit_profile(&chat, cx);
        let shows_management = !is_community || ChatProfilePermissions::manages_chat(&chat, cx);
        let partial_items =
            if shows_management { Vec::new() } else { self.partial_management_items(&chat, &nav_store, cx) };

        let load_page = this.clone();
        on_change(
            page_id(chat_id.as_deref(), "load"),
            Self::chat_load_identity(&chat, cached_chat.is_some()),
            true,
            window,
            cx,
            move |_, identity: &String, _, cx| Self::run_load_task(&load_page, identity.clone(), cx),
        );

        let logo_page = this.clone();
        let logo_chat_id = chat.id.clone();
        on_change(
            page_id(chat_id.as_deref(), "logo-file"),
            self.logo_file.as_ref().map(|file| file.url.clone()),
            false,
            window,
            cx,
            move |_, _, _, cx| Self::logo_file_did_change(&logo_page, &logo_chat_id, cx),
        );

        let redirect_page = this.clone();
        let redirect_store = nav_store.clone();
        let owned_bot_id = Self::owned_bot_id(&chat, cx);
        on_change(
            page_id(chat_id.as_deref(), "owned-bot"),
            (chat.clone(), owned_bot_id),
            false,
            window,
            cx,
            move |_, _, _, cx| Self::redirect_owned_bot_if_needed(&redirect_page, &redirect_store, cx),
        );

        let mut sections = vstack(22.0)
            .w_full()
            .items_stretch()
            .pt(px(4.0))
            .child(self.logo_block(&this, &chat, can_edit_meta));

        if is_community {
            let mut metadata = GroupedBtns::new(
                page_id(chat_id.as_deref(), "metadata"),
                self.metadata_items(&this, &chat, can_edit_meta),
            );
            if !can_edit_meta {
                metadata = metadata.end_group_title(tr("error_chat_permission_denied"));
            }
            sections = sections.child(env_scope::<IsEnabledKey>(can_edit_meta).child(metadata));
        }

        if is_community && ChatProfilePermissions::has(&chat, ChatPerm::MANAGE_GROUP, cx) {
            sections = sections
                .child(GroupedBtns::new(page_id(chat_id.as_deref(), "type"), Self::type_items(&chat, &nav_store)));
        }

        if shows_management {
            sections = sections
                .child(GroupedBtns::new(
                    page_id(chat_id.as_deref(), "content"),
                    self.content_items(&chat, &nav_store, cx),
                ))
                .child(GroupedBtns::new(
                    page_id(chat_id.as_deref(), "management"),
                    Self::management_items(&chat, &nav_store, cx),
                ));
        } else if !partial_items.is_empty() {
            sections = sections.child(GroupedBtns::new(page_id(chat_id.as_deref(), "partial"), partial_items));
        }

        sections = sections
            .child(GroupedBtns::new(page_id(chat_id.as_deref(), "danger"), Self::danger_items(&chat, &nav_store)))
            .child(spacer_min_h(100.0));

        let content = scroll_view(page_id(chat_id.as_deref(), "scroll"))
            .list_shell_scroll_under_chrome(cx)
            .size_full()
            .bg(theme.bg._100)
            .child(sections);

        let back_store = nav_store.clone();
        let mut shell = SettingsWrapperUi::new(tr(Self::type_settings_key(chat.r#type)), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(content);
        if can_edit_meta {
            shell = shell.right_jsx(Self::save_button(&this, &nav_store, self.can_save_meta(&chat), cx));
        }

        shell.on_appear(page_id(chat_id.as_deref(), "appear"), move |_, cx| {
            Self::redirect_owned_bot_if_needed(&this, &nav_store, cx);
        })
    }
}
