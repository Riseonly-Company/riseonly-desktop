use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, AsyncApp, Bounds, Context, Entity, Pixels, Window};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::photo_library::LibraryAsset;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::media_viewer::media_viewer_types::MediaViewerHeroSkin;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::media_picker::editor::editor_mode::EditorMode;
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorWindowLog;
use crate::modules::media_picker::library::media_asset::MediaAsset;

pub type MediaEditorPresentationDismissPersistHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// what the editor layer shows and how it got there, every mutation notifies
pub struct MediaEditorPresentationState {
    pub media_asset: Option<MediaAsset>,
    pub ph_asset: Option<LibraryAsset>,
    pub source_frame: Option<Bounds<Pixels>>,
    pub hero_lift_id: Option<String>,
    pub hero_cell_hidden: bool,
    pub hero_skin: Option<MediaViewerHeroSkin>,
    pub hero_warm_ui_image: Option<CleverImage>,
    pub is_presented: bool,
    pub page_index: Option<i64>,
    pub arrived_by_page_turn: bool,
    pub dismiss_persist_handler: Option<MediaEditorPresentationDismissPersistHandler>,
    pub editor_mode: EditorMode,
    pub focused_text_entity_id: Option<Uuid>,
    pub user_deselected_asset_ids: HashSet<String>,
    pub last_stage_bounds: Bounds<f32>,
    pub covers_picker: bool,
}

impl MediaEditorPresentationState {
    pub fn new() -> Self {
        Self {
            media_asset: None,
            ph_asset: None,
            source_frame: None,
            hero_lift_id: None,
            hero_cell_hidden: false,
            hero_skin: None,
            hero_warm_ui_image: None,
            is_presented: false,
            page_index: None,
            arrived_by_page_turn: false,
            dismiss_persist_handler: None,
            editor_mode: EditorMode::Initial,
            focused_text_entity_id: None,
            user_deselected_asset_ids: HashSet::new(),
            last_stage_bounds: Bounds::default(),
            covers_picker: false,
        }
    }

    pub fn set_dismiss_persist_handler(
        &mut self,
        handler: Option<MediaEditorPresentationDismissPersistHandler>,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_persist_handler = handler;
        cx.notify();
    }

    pub fn set_editor_mode(&mut self, editor_mode: EditorMode, cx: &mut Context<Self>) {
        self.editor_mode = editor_mode;
        cx.notify();
    }

    pub fn set_focused_text_entity_id(&mut self, focused_text_entity_id: Option<Uuid>, cx: &mut Context<Self>) {
        self.focused_text_entity_id = focused_text_entity_id;
        cx.notify();
    }

    pub fn set_user_deselected(&mut self, deselected: bool, asset_id: &str, cx: &mut Context<Self>) {
        if deselected {
            self.user_deselected_asset_ids.insert(asset_id.to_owned());
        } else {
            self.user_deselected_asset_ids.remove(asset_id);
        }
        cx.notify();
    }

    /// a selected text or card doesnt hold the editor open
    /// drags starting on it never reach the dismiss pan, and back closes with or without a selection
    pub fn allows_dismiss_swipe(&self) -> bool {
        self.editor_mode == EditorMode::Initial
    }

    pub fn blocks_navigation_pop(&self) -> bool {
        self.is_presented && !self.allows_dismiss_swipe()
    }

    pub fn aspect_ratio(&self) -> f32 {
        match &self.media_asset {
            Some(asset) if asset.pixel_width > 0 && asset.pixel_height > 0 => {
                asset.pixel_width as f32 / asset.pixel_height as f32
            }
            _ => 1.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn present(
        &mut self,
        media_asset: MediaAsset,
        ph_asset: LibraryAsset,
        source_frame: Option<Bounds<Pixels>>,
        hero_lift_id: String,
        hero_skin: MediaViewerHeroSkin,
        page_index: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let detail = format!("asset={} hero={hero_lift_id} frame={}", media_asset.id, source_frame.is_some());
        self.media_asset = Some(media_asset);
        self.ph_asset = Some(ph_asset);
        self.page_index = page_index;
        self.arrived_by_page_turn = false;
        self.source_frame = source_frame;
        self.hero_lift_id = Some(hero_lift_id);
        self.hero_cell_hidden = false;
        self.hero_skin = Some(hero_skin);
        self.editor_mode = EditorMode::Initial;
        self.focused_text_entity_id = None;
        self.user_deselected_asset_ids = HashSet::new();
        self.covers_picker = false;
        self.is_presented = true;
        MediaEditorWindowLog::state_event("state-open", || detail, cx);
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn turn_page(
        &mut self,
        media_asset: MediaAsset,
        ph_asset: LibraryAsset,
        page_index: i64,
        hero_lift_id: String,
        hero_skin: MediaViewerHeroSkin,
        warm_image: Option<CleverImage>,
        cx: &mut Context<Self>,
    ) {
        let detail = format!("asset={} index={page_index}", media_asset.id);
        self.media_asset = Some(media_asset);
        self.ph_asset = Some(ph_asset);
        self.page_index = Some(page_index);
        self.arrived_by_page_turn = true;
        self.hero_warm_ui_image = warm_image.or_else(|| {
            MediaViewerImageMemoryCache::memory_only(
                &hero_skin.display_source,
                SaiAccountCacheScope::active_user_id().as_deref(),
            )
        });
        self.source_frame = MediaCellScreenFrameRegistry::frame(&hero_lift_id);
        self.hero_lift_id = Some(hero_lift_id);
        self.hero_skin = Some(hero_skin);
        self.hero_cell_hidden = true;
        self.editor_mode = EditorMode::Initial;
        self.focused_text_entity_id = None;
        MediaEditorWindowLog::state_event("state-page", || detail, cx);
        cx.notify();
    }

    pub fn update_source_frame(&mut self, frame: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.source_frame = Some(frame);
        cx.notify();
    }

    pub fn set_covers_picker(&mut self, covers: bool, cx: &mut Context<Self>) {
        if self.covers_picker == covers {
            return;
        }
        self.covers_picker = covers;
        cx.notify();
    }

    pub fn set_hero_cell_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        self.hero_cell_hidden = hidden;
        cx.notify();
    }

    pub fn set_hero_warm_ui_image(&mut self, image: Option<CleverImage>, cx: &mut Context<Self>) {
        self.hero_warm_ui_image = image;
        cx.notify();
    }

    pub fn adopt_warm_image(&mut self, url_string: &str, cx: &mut Context<Self>) {
        if let Some(image) = Self::memory_image(url_string) {
            self.set_hero_warm_ui_image(Some(image), cx);
        }
    }

    pub fn resolved_source_frame(&self, registry_id: &str) -> Option<Bounds<Pixels>> {
        MediaCellScreenFrameRegistry::frame(registry_id).or(self.source_frame)
    }

    pub fn complete_flyback_handoff(&mut self, cx: &mut Context<Self>) {
        let was_presented = self.is_presented;
        MediaEditorWindowLog::state_event("state-close", || format!("wasPresented={was_presented}"), cx);
        self.hero_cell_hidden = false;
        self.source_frame = None;
        self.hero_lift_id = None;
        self.hero_skin = None;
        self.hero_warm_ui_image = None;
        self.dismiss_persist_handler = None;
        self.editor_mode = EditorMode::Initial;
        self.focused_text_entity_id = None;
        self.user_deselected_asset_ids = HashSet::new();
        self.covers_picker = false;
        self.is_presented = false;
        self.media_asset = None;
        self.ph_asset = None;
        self.page_index = None;
        self.arrived_by_page_turn = false;
        self.last_stage_bounds = Bounds::default();
        cx.notify();
    }

    pub async fn prepare_hero_warm_image(state: Entity<Self>, url_string: String, cx: &mut AsyncApp) {
        let image = match Self::memory_image(&url_string) {
            Some(hit) => Some(hit),
            None => {
                MediaViewerBitmapLoader::load_into_memory(
                    &url_string,
                    false,
                    false,
                    CleverImageCachePolicy::Standard,
                    &*cx,
                )
                .await
            }
        };
        let Some(image) = image else {
            return;
        };
        let _ = state.update(cx, |state, cx| {
            state.hero_warm_ui_image = Some(image);
            cx.notify();
        });
    }

    fn memory_image(url_string: &str) -> Option<CleverImage> {
        MediaViewerImageMemoryCache::image(
            url_string,
            SaiAccountCacheScope::active_user_id().as_deref(),
            CleverImageCachePolicy::Standard,
        )
    }
}

impl Default for MediaEditorPresentationState {
    fn default() -> Self {
        Self::new()
    }
}
