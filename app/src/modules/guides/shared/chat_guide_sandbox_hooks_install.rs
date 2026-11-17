use gpui::App;

use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::chat::stores::chat_guide_sandbox_hooks::{
    ChatGuideSandboxHooks, ChatGuideSandboxRolePatch, set_chat_guide_sandbox_hooks,
};
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatMember, ChatPermOverwrite, ChatRole, ChatSection, ChatTopic, ChatWallpaperCatalogEntry,
    ChatWallpaperCatalogState, LinkedChatInfo, OverwriteScope,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTO;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::theme::engine::rise_theme_wallpaper_upload::RiseChatWallpaperDraft;

pub fn install_chat_guide_sandbox_hooks() {
    let _ = set_chat_guide_sandbox_hooks(ChatGuideSandboxHooks {
        owns_chat_id,
        owns_message_id,
        owns_forum_chat_id,
        owns_roles,
        owns_folders,
        owns_folder,
        owns_wallpaper,
        owns_discussion,
        invents,
        overlay_folders,
        invented_folder,
        wallpaper_catalog,
        linked_discussion,
        forum_members,
        overwrites,
        member_override,
        forum_permissions,
        forum_sections,
        forum_topics,
        forum_roles,
        create_role,
        update_role,
        delete_role,
        reorder_roles,
        assign_role,
        set_overwrite,
        remove_overwrite,
        set_member_override,
        edit_member,
        create_folder,
        update_folder,
        delete_folder,
        reorder_folders,
        pin_wallpaper,
        unpin_wallpaper,
        adopt_wallpaper,
        link_discussion,
        unlink_discussion,
        replace_photo,
    });
}

fn owns_chat_id(chat_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_chat_id(Some(chat_id)))
}

fn owns_message_id(message_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_message_id(message_id))
}

fn owns_forum_chat_id(chat_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_forum_chat_id(Some(chat_id)))
}

fn owns_roles(chat_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_roles(Some(chat_id)))
}

fn owns_folders(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_folders())
}

fn owns_folder(folder_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_folder(folder_id))
}

fn owns_wallpaper(chat_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_wallpaper(Some(chat_id)))
}

fn owns_discussion(chat_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_discussion(Some(chat_id)))
}

fn invents(chat_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).invents(Some(chat_id)))
}

fn overlay_folders(real: Vec<ChatFolder>, cx: &App) -> Vec<ChatFolder> {
    match GuideSandbox::existing(cx) {
        Some(sandbox) => sandbox.read(cx).overlay_folders(real),
        None => real,
    }
}

fn invented_folder(folder_id: &str, cx: &App) -> Option<ChatFolder> {
    GuideSandbox::existing(cx).and_then(|sandbox| {
        let scene = sandbox.read(cx).folder_scene.as_ref()?;
        scene.invented.iter().find(|folder| folder.id == folder_id).cloned()
    })
}

fn wallpaper_catalog(chat_id: &str, cx: &App) -> Option<ChatWallpaperCatalogState> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).wallpaper_catalog(chat_id))
}

fn linked_discussion(chat_id: &str, cx: &App) -> Option<Option<LinkedChatInfo>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).linked_discussion(chat_id))
}

fn forum_members(scope: &str, cx: &App) -> Option<Vec<ChatMember>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).forum_members(scope, cx))
}

fn overwrites(scope: OverwriteScope, scope_id: &str, cx: &App) -> Option<Vec<ChatPermOverwrite>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).overwrites(scope, scope_id))
}

fn member_override(participant_id: &str, cx: &App) -> Option<(i64, i64)> {
    GuideSandbox::existing(cx)
        .and_then(|sandbox| sandbox.read(cx).member_override(participant_id))
        .map(|member_override| (member_override.allow, member_override.deny))
}

fn forum_permissions(cx: &mut App) -> EffectivePermissionsDTO {
    GuideSandbox::shared(cx).read(cx).forum_permissions()
}

fn forum_sections(cx: &mut App) -> Vec<ChatSection> {
    GuideSandbox::shared(cx).read(cx).forum_sections()
}

fn forum_topics(cx: &mut App) -> Vec<ChatTopic> {
    GuideSandbox::shared(cx).read(cx).forum_topics()
}

fn forum_roles(cx: &mut App) -> Vec<ChatRole> {
    GuideSandbox::shared(cx).read(cx).forum_roles()
}

fn create_role(
    name: String,
    color: i64,
    permissions: i64,
    mentionable: bool,
    hoist: bool,
    cx: &mut App,
) -> Option<ChatRole> {
    GuideSandbox::shared(cx)
        .update(cx, |sandbox, cx| sandbox.create_role(name, color, permissions, mentionable, hoist, cx))
}

fn update_role(role_id: &str, patch: ChatGuideSandboxRolePatch, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| {
        sandbox.update_role(
            role_id,
            patch.name,
            patch.color,
            patch.permissions,
            patch.mentionable,
            patch.hoist,
            cx,
        );
    });
}

fn delete_role(role_id: &str, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_role(role_id, cx));
}

fn reorder_roles(role_ids: &[String], cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.reorder_roles(role_ids, cx));
}

fn assign_role(participant_id: &str, role_id: &str, assign: bool, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.assign_role(participant_id, role_id, assign, cx));
}

fn set_overwrite(scope: OverwriteScope, scope_id: &str, overwrite: ChatPermOverwrite, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_overwrite(scope, scope_id, overwrite, cx));
}

fn remove_overwrite(scope: OverwriteScope, scope_id: &str, target_type: &str, target_id: &str, cx: &mut App) {
    GuideSandbox::shared(cx)
        .update(cx, |sandbox, cx| sandbox.remove_overwrite(scope, scope_id, target_type, target_id, cx));
}

fn set_member_override(participant_id: &str, allow: i64, deny: i64, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_member_override(participant_id, allow, deny, cx));
}

fn edit_member(
    participant_id: &str,
    role: Option<&str>,
    rank: Option<String>,
    rank_color: Option<String>,
    cx: &mut App,
) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.edit_member(participant_id, role, rank, rank_color, cx));
}

fn create_folder(folder: ChatFolder, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.create_folder(folder, cx));
}

fn update_folder(folder: ChatFolder, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.update_folder(folder, cx));
}

fn delete_folder(folder_id: &str, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_folder(folder_id, cx));
}

fn reorder_folders(folder_ids: Vec<String>, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.reorder_folders(folder_ids, cx));
}

fn pin_wallpaper(entry: &ChatWallpaperCatalogEntry, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.pin_wallpaper(entry, cx));
}

fn unpin_wallpaper(cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.unpin_wallpaper(cx));
}

fn adopt_wallpaper(draft: RiseChatWallpaperDraft, look: ChatWallpaperLook, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.adopt_wallpaper(draft, look, cx));
}

fn link_discussion(group_id: &str, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.link_discussion(group_id, cx));
}

fn unlink_discussion(cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.unlink_discussion(cx));
}

fn replace_photo(item: MediaPickerItem, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.replace_photo(item, cx));
}
