use std::collections::HashMap;

use gpui::Context;
use rise_platform::photo_library::{LibraryAsset, LibraryAssetKind};

use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerTrackSelection, MediaPickerViewMode};
use crate::modules::media_picker::editor::editor_edit_preview_cache::{
    EditorEditPreviewCache, MEDIA_PICKER_EDITS_DID_CHANGE,
};
use crate::modules::media_picker::editor::editor_state::{EditorImageState, EditorVideoState};

/// picker selection in tap order with the edits made to each selected asset
pub struct MediaSelectionState {
    ordered_identifiers: Vec<String>,
    assets_by_identifier: HashMap<String, LibraryAsset>,
    edits_by_identifier: HashMap<String, MediaSelectionEdit>,

    pub shared_caption: String,

    pub send_as_file: bool,

    pub hides_under_spoiler: bool,

    pub view_mode: Option<MediaPickerViewMode>,

    pub attached_track: Option<MediaPickerTrackSelection>,

    pub max_allowed: i64,

    pub enforces_single_kind: bool,

    pub replaces_single_pick: bool,

    pub max_video_count: Option<i64>,
    pub max_image_count: Option<i64>,
}

impl MediaSelectionState {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            ordered_identifiers: Vec::new(),
            assets_by_identifier: HashMap::new(),
            edits_by_identifier: HashMap::new(),
            shared_caption: String::new(),
            send_as_file: false,
            hides_under_spoiler: false,
            view_mode: None,
            attached_track: None,
            max_allowed: 10,
            enforces_single_kind: false,
            replaces_single_pick: false,
            max_video_count: None,
            max_image_count: None,
        }
    }

    pub fn ordered_identifiers(&self) -> &[String] {
        &self.ordered_identifiers
    }

    pub fn assets_by_identifier(&self) -> &HashMap<String, LibraryAsset> {
        &self.assets_by_identifier
    }

    pub fn edits_by_identifier(&self) -> &HashMap<String, MediaSelectionEdit> {
        &self.edits_by_identifier
    }

    /// cells draw the spoiler for both
    /// a view once item stays closed until the recipient opens it
    pub fn draws_spoiler(&self) -> bool {
        self.hides_under_spoiler || self.view_mode.is_some()
    }

    pub fn contains_video(&self) -> bool {
        self.ordered_identifiers.iter().any(|id| {
            self.assets_by_identifier.get(id).is_some_and(|asset| asset.kind == LibraryAssetKind::Video)
        })
    }

    /// what the ceilings count, a radio pick is about to be replaced so it counts toward none
    fn ceiling_identifiers(&self) -> &[String] {
        if self.replaces_single_pick { &[] } else { &self.ordered_identifiers }
    }

    pub fn count(&self) -> i64 {
        self.ordered_identifiers.len() as i64
    }

    pub fn is_empty(&self) -> bool {
        self.ordered_identifiers.is_empty()
    }

    pub fn max_reached(&self) -> bool {
        self.count() >= self.effective_max_allowed()
    }

    /// derived never stored
    /// clear doesnt reset max_allowed so a video clamp there would wreck the preset number
    pub fn locked_kind(&self) -> Option<LibraryAssetKind> {
        if !self.enforces_single_kind {
            return None;
        }
        let first = self.ceiling_identifiers().first()?;
        self.assets_by_identifier.get(first).map(|asset| asset.kind)
    }

    /// one video is the whole selection, photos keep the preset ceiling
    pub fn effective_max_allowed(&self) -> i64 {
        if self.locked_kind() == Some(LibraryAssetKind::Video) {
            1
        } else {
            self.max_allowed
        }
    }

    /// single gate for both entry points and for greying grid cells
    /// an already selected asset always passes so a full selection can still be undone
    pub fn can_select(&self, asset: &LibraryAsset) -> bool {
        if self.contains(&asset.id) {
            return true;
        }
        if self.is_kind_locked(asset) {
            return false;
        }
        if self.is_video_ceiling_reached(asset) {
            return false;
        }
        if self.is_image_ceiling_reached(asset) {
            return false;
        }
        (self.ceiling_identifiers().len() as i64) < self.effective_max_allowed()
    }

    pub fn is_image_ceiling_reached(&self, asset: &LibraryAsset) -> bool {
        let Some(max_image_count) = self.max_image_count else {
            return false;
        };
        if asset.kind != LibraryAssetKind::Image || self.contains(&asset.id) {
            return false;
        }
        self.selected_count(LibraryAssetKind::Image) >= max_image_count
    }

    /// video ceiling alone
    /// recounted on every call so a removal or clear cant leave a stale count
    pub fn is_video_ceiling_reached(&self, asset: &LibraryAsset) -> bool {
        let Some(max_video_count) = self.max_video_count else {
            return false;
        };
        if asset.kind != LibraryAssetKind::Video || self.contains(&asset.id) {
            return false;
        }
        self.selected_count(LibraryAssetKind::Video) >= max_video_count
    }

    /// the kind lock alone, without the numeric ceiling
    pub fn is_kind_locked(&self, asset: &LibraryAsset) -> bool {
        if !self.enforces_single_kind || self.contains(&asset.id) {
            return false;
        }
        match self.locked_kind() {
            Some(locked_kind) => locked_kind != asset.kind,
            None => false,
        }
    }

    pub fn index(&self, of: &str) -> Option<usize> {
        self.ordered_identifiers.iter().position(|id| id == of)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.assets_by_identifier.contains_key(id) && self.ordered_identifiers.iter().any(|item| item == id)
    }

    pub fn toggle(&mut self, asset: &LibraryAsset, cx: &mut Context<Self>) -> bool {
        if self.contains(&asset.id) {
            self.remove(&asset.id, cx);
            return false;
        }
        if !self.can_select(asset) {
            return false;
        }
        if self.replaces_single_pick {
            return self.ensure_selected(asset, cx);
        }
        self.ordered_identifiers.push(asset.id.clone());
        self.assets_by_identifier.insert(asset.id.clone(), asset.clone());
        cx.notify();
        true
    }

    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ordered_identifiers.retain(|item| item != id);
        self.assets_by_identifier.remove(id);
        self.edits_by_identifier.remove(id);
        cx.notify();
        EditorEditPreviewCache::remove(id, cx);
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let edited_ids: Vec<String> = self.edits_by_identifier.keys().cloned().collect();
        self.ordered_identifiers.clear();
        self.assets_by_identifier.clear();
        self.edits_by_identifier.clear();
        self.attached_track = None;
        self.hides_under_spoiler = false;
        self.view_mode = None;
        cx.notify();
        for id in &edited_ids {
            EditorEditPreviewCache::remove(id, cx);
        }
    }

    pub fn ensure_selected(&mut self, asset: &LibraryAsset, cx: &mut Context<Self>) -> bool {
        if self.contains(&asset.id) {
            return true;
        }
        if self.max_allowed == 1 && !self.ordered_identifiers.is_empty() {
            let previous_ids = std::mem::take(&mut self.ordered_identifiers);
            let previous_assets = std::mem::take(&mut self.assets_by_identifier);
            if !self.can_select(asset) {
                self.ordered_identifiers = previous_ids;
                self.assets_by_identifier = previous_assets;
                return false;
            }
            cx.notify();
        }
        if !self.can_select(asset) {
            return false;
        }
        self.ordered_identifiers.push(asset.id.clone());
        self.assets_by_identifier.insert(asset.id.clone(), asset.clone());
        cx.notify();
        true
    }

    pub fn set_edit_for_asset(&mut self, edit: MediaSelectionEdit, asset: &LibraryAsset, cx: &mut Context<Self>) {
        let was_edited = edit.payload.was_edited();
        self.edits_by_identifier.insert(asset.id.clone(), edit);
        cx.notify();
        if was_edited {
            self.ensure_selected(asset, cx);
        }
        RiseAppEvents::default_center().post_with(MEDIA_PICKER_EDITS_DID_CHANGE, asset.id.clone());
    }

    pub fn set_edit(&mut self, edit: MediaSelectionEdit, id: &str, cx: &mut Context<Self>) {
        self.edits_by_identifier.insert(id.to_string(), edit);
        cx.notify();
        RiseAppEvents::default_center().post_with(MEDIA_PICKER_EDITS_DID_CHANGE, id.to_string());
    }

    pub fn remove_edit(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.edits_by_identifier.remove(id).is_none() {
            return;
        }
        cx.notify();
        EditorEditPreviewCache::remove(id, cx);
        RiseAppEvents::default_center().post_with(MEDIA_PICKER_EDITS_DID_CHANGE, id.to_string());
    }

    pub fn edit(&self, id: &str) -> Option<&MediaSelectionEdit> {
        self.edits_by_identifier.get(id)
    }

    pub fn ordered_assets(&self) -> Vec<LibraryAsset> {
        self.ordered_identifiers
            .iter()
            .filter_map(|id| self.assets_by_identifier.get(id).cloned())
            .collect()
    }

    fn selected_count(&self, kind: LibraryAssetKind) -> i64 {
        self.ordered_identifiers
            .iter()
            .filter(|id| self.assets_by_identifier.get(*id).is_some_and(|asset| asset.kind == kind))
            .count() as i64
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaSelectionEditPayload {
    Image(EditorImageState),
    Video(EditorVideoState),
}

impl MediaSelectionEditPayload {
    pub fn was_edited(&self) -> bool {
        match self {
            Self::Image(state) => state.was_edited && state.has_user_edits(),
            Self::Video(state) => state.has_meaningful_edits(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaSelectionEdit {
    pub payload: MediaSelectionEditPayload,
}
