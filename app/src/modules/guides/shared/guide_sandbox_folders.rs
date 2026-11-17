use std::collections::HashMap;

use gpui::Context;
use rise_core::foundation::foundation_date::Date;

use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

/// the folder a folder lesson creates, kept on the computer
/// real folders stay untouched and the new one joins them in every folder read
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuideSandboxFoldersScene {
    pub invented: Vec<ChatFolder>,
    pub order: Option<Vec<String>>,
}

impl GuideSandbox {
    pub fn owns_folders(&self) -> bool {
        self.is_armed && self.folder_scene.is_some()
    }

    pub fn overlay_folders(&self, real: Vec<ChatFolder>) -> Vec<ChatFolder> {
        let Some(scene) = self.folder_scene.as_ref().filter(|_| self.owns_folders()) else {
            return real;
        };
        let mut all = real;
        all.extend(scene.invented.iter().cloned());
        if let Some(order) = &scene.order {
            let rank: HashMap<&str, usize> =
                order.iter().enumerate().map(|(index, id)| (id.as_str(), index)).collect();
            all.sort_by_key(|folder| rank.get(folder.id.as_str()).copied().unwrap_or(usize::MAX));
        }
        for (index, folder) in all.iter_mut().enumerate() {
            folder.position = index as i64;
        }
        all
    }

    pub fn owns_folder(&self, id: &str) -> bool {
        self.folder_scene
            .as_ref()
            .is_some_and(|scene| scene.invented.iter().any(|folder| folder.id == id))
    }

    pub fn create_folder(&mut self, folder: ChatFolder, cx: &mut Context<Self>) {
        let Some(scene) = self.folder_scene.as_mut() else {
            return;
        };
        let mut created = folder;
        let count = scene.invented.len();
        created.id = if count == 0 {
            Self::folder_id()
        } else {
            format!("{}_{}", Self::folder_id(), count + 1)
        };
        created.created_at = Date::now().time_interval_since_1970();
        created.updated_at = created.created_at;
        scene.invented.push(created);
        cx.notify();
    }

    pub fn update_folder(&mut self, folder: ChatFolder, cx: &mut Context<Self>) {
        let Some(scene) = self.folder_scene.as_mut() else {
            return;
        };
        let Some(index) = scene.invented.iter().position(|existing| existing.id == folder.id) else {
            return;
        };
        scene.invented[index] = folder;
        cx.notify();
    }

    pub fn delete_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(scene) = self.folder_scene.as_mut() else {
            return;
        };
        scene.invented.retain(|folder| folder.id != id);
        if let Some(order) = scene.order.as_mut() {
            order.retain(|existing| existing != id);
        }
        cx.notify();
    }

    pub fn reorder_folders(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(scene) = self.folder_scene.as_mut() else {
            return;
        };
        scene.order = Some(ids);
        cx.notify();
    }
}
