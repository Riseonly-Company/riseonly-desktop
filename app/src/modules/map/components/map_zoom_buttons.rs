use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div};
use rise_media::map_view::{InputModifiers, MapInput, MapKey};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;

use crate::modules::map::components::globe::map_globe_camera_link::MapGlobeCameraLink;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

/// desktop zoom controls at the map's trailing edge
/// why: a mouse without a trackpad has no pinch so the flat map couldnt zoom back out to the globe
#[derive(IntoElement, Default)]
pub struct MapZoomButtons;

impl MapZoomButtons {
    pub fn new() -> Self {
        Self
    }

    /// one zoom level over the globe or the flat map, whichever is shown
    /// zooming out past the handover opens the globe like a pinch
    /// cmd plus and minus on the map page end here too
    pub fn zoom(levels: f64, window: &mut Window, cx: &mut App) {
        let is_globe_shown = MapInteractionsStore::shared(cx).read(cx).globe_entry.is_some();
        let link = MapGlobeCameraLink::shared(cx);
        if is_globe_shown {
            let globe = link.read(cx).globe();
            if let Some(globe) = globe {
                globe.update(cx, |globe, cx| globe.zoom_by(levels, window, cx));
            }
            return;
        }
        let map = link.read(cx).map_view();
        if let Some(map) = map {
            let key = if levels > 0.0 { MapKey::ZoomIn } else { MapKey::ZoomOut };
            map.handle_input(MapInput::Key { key, modifiers: InputModifiers::default() }, true, cx);
        }
    }
}

impl RenderOnce for MapZoomButtons {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(PageHeaderIconButton::new("map-zoom-in", "plus", |window, cx| Self::zoom(1.0, window, cx)).font_size(16.0))
            .child(PageHeaderIconButton::new("map-zoom-out", "minus", |window, cx| Self::zoom(-1.0, window, cx)).font_size(16.0))
    }
}
