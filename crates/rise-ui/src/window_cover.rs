use gpui::{App, Decorations, Edges, Pixels, Point, Size, Window, point, px, size};
use rise_platform::{HostOs, window_chrome};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WindowCoverFrame {
    pub origin: Point<Pixels>,
    pub size: Size<Pixels>,
    pub safe_top: Pixels,
}

impl WindowCoverFrame {
    pub fn resolve(window: &Window, cx: &App) -> Self {
        let strip = f32::from(crate::theme(cx).shell.window_drag_height);
        let top_insets = window_chrome::cover_top_insets(
            HostOs::current(),
            window_chrome::granted_decorations(window),
            strip,
            window.is_fullscreen(),
        );
        Self::laid_out(window.viewport_size(), Self::client_edges(window), top_insets.frame_top, top_insets.safe_top)
    }

    /// height a sheet detent resolves against, large stops under the window top band
    pub fn sheet_container_height(&self) -> f32 {
        f32::from(self.size.height - self.safe_top)
    }

    fn client_edges(window: &Window) -> Edges<f32> {
        let Decorations::Client { tiling } = window.window_decorations() else {
            return Edges::default();
        };
        let border = window.client_inset().map_or(0.0, f32::from);
        let edge = |tiled: bool| if tiled { 0.0 } else { border };
        Edges { top: edge(tiling.top), right: edge(tiling.right), bottom: edge(tiling.bottom), left: edge(tiling.left) }
    }

    pub(crate) fn laid_out(viewport: Size<Pixels>, client: Edges<f32>, frame_top: f32, safe_top: f32) -> Self {
        let top = client.top + frame_top;
        let width = f32::from(viewport.width) - client.left - client.right;
        let height = f32::from(viewport.height) - top - client.bottom;
        Self {
            origin: point(px(client.left), px(top)),
            size: size(px(width.max(0.0)), px(height.max(0.0))),
            safe_top: px(safe_top),
        }
    }
}
