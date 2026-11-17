use gpui::{AppContext, AsyncApp, Bounds, Entity, Pixels};
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers, MediaViewerPresentationState,
};
use crate::core::media_viewer::media_viewer_types::{MediaViewerChrome, MediaViewerGalleryItem, MediaViewerHeroSkin};

pub enum MediaViewerOpen {}

impl MediaViewerOpen {
    #[allow(clippy::too_many_arguments)]
    pub async fn present(
        state: Entity<MediaViewerPresentationState>,
        items: Vec<MediaViewerGalleryItem>,
        at_index: usize,
        source_frame: Bounds<Pixels>,
        frame_registry_id: String,
        hero_skin: MediaViewerHeroSkin,
        chrome: MediaViewerChrome,
        gallery_scope: MediaViewerGalleryScope,
        handlers: MediaViewerPresentationHandlers,
        cx: &mut AsyncApp,
    ) {
        let Some(selected_item) = items.get(at_index) else {
            return;
        };
        let persist_to_disk = selected_item.persist_media_to_disk;
        let cache_policy = selected_item.image_cache_policy;
        let screen_rect = MediaCellScreenFrameRegistry::frame(&frame_registry_id).unwrap_or(source_frame);
        MediaViewerPresentationState::prepare_hero_warm_image(
            &state,
            &hero_skin.display_source,
            persist_to_disk,
            cache_policy,
            cx,
        )
        .await;
        let mut resolved_hero_skin = hero_skin;
        let live_media_tail = MediaCellScreenFrameRegistry::media_tail_presentation(&frame_registry_id);
        if resolved_hero_skin.media_tail.is_some() || live_media_tail.is_some() {
            resolved_hero_skin.media_tail = live_media_tail;
        }
        let live_source_media_radii = MediaCellScreenFrameRegistry::source_media_radii(&frame_registry_id);
        if resolved_hero_skin.source_media_radii.is_some() || live_source_media_radii.is_some() {
            resolved_hero_skin.source_media_radii = live_source_media_radii;
        }
        state.update(cx, |state, cx| {
            state.present(
                items,
                at_index,
                Some(screen_rect),
                Some(frame_registry_id),
                Some(resolved_hero_skin),
                chrome,
                gallery_scope,
                handlers,
                cx,
            );
        });
    }
}
