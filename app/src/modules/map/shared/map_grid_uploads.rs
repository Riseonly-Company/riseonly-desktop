use std::collections::HashMap;

use gpui::{App, AppContext, Context, Entity};
use rise_core::foundation::foundation_url::Url;

/// what a cell shows before the server picture exists, the source gallery asset or the picked file
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MapGridPreview {
    Asset(String),
    File(Url),
}

/// hand picked file on its way into an open grid
/// drafts need no such row since they already own a cell and only change state
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapGridUpload {
    pub id: String,
    pub grid_id: String,
    pub preview: MapGridPreview,
    pub is_video: bool,
    pub duration_ms: i64,
}

/// progress of one cell
/// only that cell's ring reads it
/// so an arriving byte re-evaluates one small view and not the card or grid
#[derive(Default)]
pub struct MapUploadProgressBox {
    pub fraction: f64,
    pub total_bytes: i64,
    pub is_active: bool,
}

/// ties a draft and its upload and the finished photo to one cell
/// draft cell is keyed by gallery id and the photo by server id
/// alias lets the photo answer to the draft key so the grid sees one row changing content
pub struct MapGridUploadCenter {
    boxes: HashMap<String, Entity<MapUploadProgressBox>>,
    slot_of_photo: HashMap<String, String>,
    previews: HashMap<String, MapGridPreview>,
    order: Vec<String>,
    pinned: Vec<String>,
}

impl MapGridUploadCenter {
    const CAPACITY: usize = 256;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        Self {
            boxes: HashMap::new(),
            slot_of_photo: HashMap::new(),
            previews: HashMap::new(),
            order: Vec::new(),
            pinned: Vec::new(),
        }
    }

    pub fn box_for(&mut self, slot_id: &str, cx: &mut Context<Self>) -> Entity<MapUploadProgressBox> {
        if let Some(progress) = self.boxes.get(slot_id) {
            return progress.clone();
        }
        let progress = cx.new(|_| MapUploadProgressBox::default());
        self.boxes.insert(slot_id.to_owned(), progress.clone());
        progress
    }

    pub fn begin(&mut self, slot_id: &str, photo_id: &str, preview: MapGridPreview, cx: &mut Context<Self>) {
        self.slot_of_photo.insert(photo_id.to_owned(), slot_id.to_owned());
        self.previews.insert(slot_id.to_owned(), preview);
        self.order.retain(|id| id != photo_id);
        self.order.push(photo_id.to_owned());
        while self.order.len() > Self::CAPACITY {
            let evicted = self.order.remove(0);
            if let Some(slot) = self.slot_of_photo.remove(&evicted) {
                self.previews.remove(&slot);
            }
        }
        let progress = self.box_for(slot_id, cx);
        progress.update(cx, |progress, cx| {
            progress.fraction = 0.0;
            progress.total_bytes = 0;
            progress.is_active = false;
            cx.notify();
        });
    }

    pub fn pin_front(&mut self, slot_ids: &[String]) {
        self.pinned.retain(|id| !slot_ids.contains(id));
        let mut pinned = slot_ids.to_vec();
        pinned.append(&mut self.pinned);
        pinned.truncate(Self::CAPACITY);
        self.pinned = pinned;
    }

    pub fn pinned(&self) -> &[String] {
        &self.pinned
    }

    /// upload finished, success or failure
    /// alias and preview stay
    /// so the finished photo keeps the cell and has something to show while its picture loads
    pub fn end(&mut self, slot_id: &str) {
        self.boxes.remove(slot_id);
    }

    /// upload cancelled, the cell goes back to its previous content
    pub fn forget(&mut self, slot_id: &str, photo_id: &str) {
        self.boxes.remove(slot_id);
        self.slot_of_photo.remove(photo_id);
        self.previews.remove(slot_id);
        self.order.retain(|id| id != photo_id);
        self.pinned.retain(|id| id != slot_id);
    }

    pub fn slot_id(&self, for_photo: &str) -> String {
        self.slot_of_photo
            .get(for_photo)
            .cloned()
            .unwrap_or_else(|| for_photo.to_owned())
    }

    pub fn preview(&self, for_slot: &str) -> Option<MapGridPreview> {
        self.previews.get(for_slot).cloned()
    }

    pub fn reset(&mut self) {
        self.boxes = HashMap::new();
        self.slot_of_photo = HashMap::new();
        self.previews = HashMap::new();
        self.order = Vec::new();
        self.pinned = Vec::new();
    }
}
