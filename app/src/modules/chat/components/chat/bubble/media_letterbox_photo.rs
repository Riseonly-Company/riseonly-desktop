use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, Corners, ElementId, Entity, ImageSource, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Styled,
    StyledImage, Task, Window, div, hsla, img,
};
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_ui::swiftui::blur::{BlurModifiers, blur_aware_image_source};
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::environment::layout_direction;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
pub use crate::core::media_viewer::media_letterbox_photo_hooks::{MediaAspectFit, MediaLetterboxPhoto};
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::chat::components::chat::bubble::bubble_corners::BubbleCorners;
use crate::modules::chat::components::chat::bubble::rise_native_message_image_grid::{
    RISE_NATIVE_MEDIA_BACKDROP_APPLIED, RiseNativeMediaThumbnailPipeline,
};

/// clips one element with per corner radii
fn rounded<E: Styled>(element: E, radii: Corners<Pixels>) -> E {
    element
        .rounded_tl(radii.top_left)
        .rounded_tr(radii.top_right)
        .rounded_bl(radii.bottom_left)
        .rounded_br(radii.bottom_right)
}

/// state of the media side fill plus the observer of the tile backdrop event
struct MessageMediaSideFillState {
    source: String,
    video_source: String,
    image: Option<CleverImage>,
    image_source: Option<String>,
    _observer: Option<RiseSubscription>,
    _listener: Option<Task<()>>,
}

impl MessageMediaSideFillState {
    /// one cache read, a miss keeps the previous fill
    /// so the bubble doesnt flash flat when the upload replaces the local preview
    fn resolve(&mut self) -> bool {
        let Some(cached) = MessageMediaSideFill::lookup(&self.source, &self.video_source) else {
            return false;
        };
        self.image = Some(cached);
        self.image_source = Some(self.source.clone());
        true
    }
}

/// blurred copy filling a bubble beside media too narrow to reach its edges
/// reuses the tile fill, loads nothing
/// until the tile has it the bubble keeps its own color
#[derive(IntoElement)]
pub struct MessageMediaSideFill {
    id: ElementId,
    source: String,
    video_source: String,
    corners: BubbleCorners,
}

impl MessageMediaSideFill {
    pub fn new(id: impl Into<ElementId>, source: impl Into<String>, corners: BubbleCorners) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            video_source: String::new(),
            corners,
        }
    }

    pub fn video_source(mut self, video_source: impl Into<String>) -> Self {
        self.video_source = video_source.into();
        self
    }

    fn lookup(source: &str, video_source: &str) -> Option<CleverImage> {
        RiseNativeMediaThumbnailPipeline::cached_backdrop(
            source,
            video_source,
            SaiAccountCacheScope::active_user_id().as_deref(),
            SaiAccountCacheScope::active_epoch().as_deref(),
        )
    }
}

impl RenderOnce for MessageMediaSideFill {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = self.source.clone();
        let video_source = self.video_source.clone();
        let state: Entity<MessageMediaSideFillState> = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("side-fill-state")),
            cx,
            move |_, cx| {
                let mut state = MessageMediaSideFillState {
                    source,
                    video_source,
                    image: None,
                    image_source: None,
                    _observer: None,
                    _listener: None,
                };
                state.resolve();
                let (sender, mut receiver) = mpsc::unbounded::<()>();
                state._observer = Some(RiseAppEvents::default_center().observe(
                    RISE_NATIVE_MEDIA_BACKDROP_APPLIED,
                    move |_| {
                        let _ = sender.unbounded_send(());
                    },
                ));
                state._listener = Some(cx.spawn(async move |this, cx| {
                    while receiver.next().await.is_some() {
                        let updated = this.update(cx, |state: &mut MessageMediaSideFillState, cx| {
                            if state.image_source.as_deref() == Some(state.source.as_str()) {
                                return;
                            }
                            if state.resolve() {
                                cx.notify();
                            }
                        });
                        if updated.is_err() {
                            break;
                        }
                    }
                }));
                state
            },
        );

        state.update(cx, |state, _| {
            state.video_source = self.video_source.clone();
            if state.source != self.source {
                state.source = self.source.clone();
                state.resolve();
            }
        });

        let radii = self.corners.uneven_shape(layout_direction(cx));
        let mut fill = rounded(div().relative().size_full().overflow_hidden(), radii);
        if let Some(image) = state.read(cx).image.clone() {
            let picture = rounded(
                img(blur_aware_image_source(ImageSource::Render(image)))
                    .size_full()
                    .object_fit(ContentMode::Fill.object_fit()),
                radii,
            );
            fill = fill
                .child(div().absolute().inset_0().child(picture.blur_opaque(22.0, true)))
                .child(
                    rounded(div().absolute().inset_0(), radii).bg(hsla(0.0, 0.0, 0.0, 0.24)),
                );
        }
        fill
    }
}
