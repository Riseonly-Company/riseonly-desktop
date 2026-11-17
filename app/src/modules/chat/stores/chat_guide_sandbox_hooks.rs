use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatMember, ChatPermOverwrite, ChatRole, ChatSection, ChatTopic, ChatWallpaperCatalogEntry,
    ChatWallpaperCatalogState, LinkedChatInfo, OverwriteScope,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTO;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::RiseChatWallpaperDraft;

/// the fields a role edit may change, none keeps the old value
pub struct ChatGuideSandboxRolePatch {
    pub name: Option<String>,
    pub color: Option<i64>,
    pub permissions: Option<i64>,
    pub mentionable: Option<bool>,
    pub hoist: Option<bool>,
}

/// what the chat and message stores ask the guide sandbox
pub struct ChatGuideSandboxHooks {
    pub owns_chat_id: fn(chat_id: &str, cx: &App) -> bool,
    pub owns_message_id: fn(message_id: &str, cx: &App) -> bool,
    pub owns_forum_chat_id: fn(chat_id: &str, cx: &App) -> bool,
    pub owns_roles: fn(chat_id: &str, cx: &App) -> bool,
    pub owns_folders: fn(cx: &App) -> bool,
    pub owns_folder: fn(folder_id: &str, cx: &App) -> bool,
    pub owns_wallpaper: fn(chat_id: &str, cx: &App) -> bool,
    pub owns_discussion: fn(chat_id: &str, cx: &App) -> bool,
    pub invents: fn(chat_id: &str, cx: &App) -> bool,
    pub overlay_folders: fn(real: Vec<ChatFolder>, cx: &App) -> Vec<ChatFolder>,
    pub invented_folder: fn(folder_id: &str, cx: &App) -> Option<ChatFolder>,
    pub wallpaper_catalog: fn(chat_id: &str, cx: &App) -> Option<ChatWallpaperCatalogState>,
    pub linked_discussion: fn(chat_id: &str, cx: &App) -> Option<Option<LinkedChatInfo>>,
    pub forum_members: fn(scope: &str, cx: &App) -> Option<Vec<ChatMember>>,
    pub overwrites: fn(scope: OverwriteScope, scope_id: &str, cx: &App) -> Option<Vec<ChatPermOverwrite>>,
    pub member_override: fn(participant_id: &str, cx: &App) -> Option<(i64, i64)>,
    pub forum_permissions: fn(cx: &mut App) -> EffectivePermissionsDTO,
    pub forum_sections: fn(cx: &mut App) -> Vec<ChatSection>,
    pub forum_topics: fn(cx: &mut App) -> Vec<ChatTopic>,
    pub forum_roles: fn(cx: &mut App) -> Vec<ChatRole>,
    pub create_role: fn(
        name: String,
        color: i64,
        permissions: i64,
        mentionable: bool,
        hoist: bool,
        cx: &mut App,
    ) -> Option<ChatRole>,
    pub update_role: fn(role_id: &str, patch: ChatGuideSandboxRolePatch, cx: &mut App),
    pub delete_role: fn(role_id: &str, cx: &mut App),
    pub reorder_roles: fn(role_ids: &[String], cx: &mut App),
    pub assign_role: fn(participant_id: &str, role_id: &str, assign: bool, cx: &mut App),
    pub set_overwrite: fn(scope: OverwriteScope, scope_id: &str, overwrite: ChatPermOverwrite, cx: &mut App),
    pub remove_overwrite:
        fn(scope: OverwriteScope, scope_id: &str, target_type: &str, target_id: &str, cx: &mut App),
    pub set_member_override: fn(participant_id: &str, allow: i64, deny: i64, cx: &mut App),
    pub edit_member:
        fn(participant_id: &str, role: Option<&str>, rank: Option<String>, rank_color: Option<String>, cx: &mut App),
    pub create_folder: fn(folder: ChatFolder, cx: &mut App),
    pub update_folder: fn(folder: ChatFolder, cx: &mut App),
    pub delete_folder: fn(folder_id: &str, cx: &mut App),
    pub reorder_folders: fn(folder_ids: Vec<String>, cx: &mut App),
    pub pin_wallpaper: fn(entry: &ChatWallpaperCatalogEntry, cx: &mut App),
    pub unpin_wallpaper: fn(cx: &mut App),
    pub adopt_wallpaper: fn(draft: RiseChatWallpaperDraft, look: ChatWallpaperLook, cx: &mut App),
    pub link_discussion: fn(group_id: &str, cx: &mut App),
    pub unlink_discussion: fn(cx: &mut App),
    pub replace_photo: fn(item: MediaPickerItem, cx: &mut App),
}

pub(crate) static CHAT_GUIDE_SANDBOX_HOOKS: FeatureHookSlot<ChatGuideSandboxHooks> =
    FeatureHookSlot::new("CHAT_GUIDE_SANDBOX_HOOKS");

static UNINSTALLED: ChatGuideSandboxHooks = ChatGuideSandboxHooks {
    owns_chat_id: |_, _| false,
    owns_message_id: |_, _| false,
    owns_forum_chat_id: |_, _| false,
    owns_roles: |_, _| false,
    owns_folders: |_| false,
    owns_folder: |_, _| false,
    owns_wallpaper: |_, _| false,
    owns_discussion: |_, _| false,
    invents: |_, _| false,
    overlay_folders: |real, _| real,
    invented_folder: |_, _| None,
    wallpaper_catalog: |_, _| None,
    linked_discussion: |_, _| None,
    forum_members: |_, _| None,
    overwrites: |_, _, _| None,
    member_override: |_, _| None,
    forum_permissions: |_| EffectivePermissionsDTO::default(),
    forum_sections: |_| Vec::new(),
    forum_topics: |_| Vec::new(),
    forum_roles: |_| Vec::new(),
    create_role: |_, _, _, _, _, _| None,
    update_role: |_, _, _| {},
    delete_role: |_, _| {},
    reorder_roles: |_, _| {},
    assign_role: |_, _, _, _| {},
    set_overwrite: |_, _, _, _| {},
    remove_overwrite: |_, _, _, _, _| {},
    set_member_override: |_, _, _, _| {},
    edit_member: |_, _, _, _, _| {},
    create_folder: |_, _| {},
    update_folder: |_, _| {},
    delete_folder: |_, _| {},
    reorder_folders: |_, _| {},
    pin_wallpaper: |_, _| {},
    unpin_wallpaper: |_| {},
    adopt_wallpaper: |_, _, _| {},
    link_discussion: |_, _| {},
    unlink_discussion: |_| {},
    replace_photo: |_, _| {},
};

pub fn set_chat_guide_sandbox_hooks(hooks: ChatGuideSandboxHooks) -> Result<(), ChatGuideSandboxHooks> {
    CHAT_GUIDE_SANDBOX_HOOKS.set(hooks)
}

pub fn chat_guide_sandbox_hooks() -> &'static ChatGuideSandboxHooks {
    CHAT_GUIDE_SANDBOX_HOOKS.get().unwrap_or(&UNINSTALLED)
}
