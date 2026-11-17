use gpui::App;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntry;
use crate::modules::chat::components::chat::async_render::rise_exact_fallback_geometry::RiseExactFallbackGeometry;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;

/// debug audit of natively drawn rows
/// height a native path answered against the height the regular hosted bubble lays out
pub enum NativeBubbleRowAudit {}

impl NativeBubbleRowAudit {
    pub const TOLERANCE: f32 = 1.0;
    pub const LOG_TARGET: &'static str = "riseonly::chat_rows";

    /// signed drift of the native height from the regular row when it passes the tolerance
    /// none when they agree or the regular row has no exact height
    pub fn mismatch(native_height: f32, regular_height: Option<f32>) -> Option<f32> {
        let delta = native_height - regular_height?;
        (delta.abs() > Self::TOLERANCE).then_some(delta)
    }

    /// warns on the chat_rows target when a native row would jump against the regular one
    pub fn check(
        kind: &str,
        entry: &ChatMessageRenderEntry,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        native_height: f32,
        cx: &mut App,
    ) {
        let regular_height = RiseExactFallbackGeometry::row_height(entry, configuration, viewport_width, cx);
        if let Some(delta) = Self::mismatch(native_height, regular_height) {
            tracing::warn!(
                target: "riseonly::chat_rows",
                "native row {kind} message {} width {viewport_width}: native {native_height} regular {} delta {delta}",
                entry.id,
                native_height - delta,
            );
        }
    }
}
