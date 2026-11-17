use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, Global, IntoElement, ParentElement, Render,
    RenderOnce, SharedString, Size, Styled, WeakEntity, Window, div,
};

use crate::core::media_viewer::media_zoomable_image_view::{
    MediaZoomContainerView, MediaZoomSingleTap, MediaZoomViewBuilder,
};
use crate::modules::rtc::components::rise_rtc_floating_video_window::{
    RiseRTCVideoSurfaceView, VideoViewLayoutMode, VideoViewMirrorMode,
};
use crate::modules::rtc::engine::rise_rtc_call_trace::RiseRTCCallTrace;
use crate::modules::rtc::media::rise_rtc_media_presentation::VideoTrack;

/// stage sized call surface with zoom
/// trackpad pinch or ctrl or cmd with wheel zooms around the pointer
/// double click toggles magnification, drag moves a magnified picture
#[derive(IntoElement)]
pub struct RiseRTCZoomableVideoSurface {
    id: ElementId,
    role: SharedString,
    track: Rc<VideoTrack>,
    layout_mode: VideoViewLayoutMode,
    mirror_mode: VideoViewMirrorMode,
    on_single_tap: MediaZoomSingleTap,
}

impl RiseRTCZoomableVideoSurface {
    pub fn new(
        id: impl Into<ElementId>,
        role: impl Into<SharedString>,
        track: Rc<VideoTrack>,
        layout_mode: VideoViewLayoutMode,
        mirror_mode: VideoViewMirrorMode,
        on_single_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            role: role.into(),
            track,
            layout_mode,
            mirror_mode,
            on_single_tap: Rc::new(on_single_tap),
        }
    }
}

impl RenderOnce for RiseRTCZoomableVideoSurface {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        RiseRTCCallTrace::note_video_pass(&self.role);
        let view = window.use_keyed_state(self.id, cx, |_, cx| RiseRTCVideoZoomContainerView::new(cx));
        view.update(cx, |view, cx| {
            view.set_on_single_tap(self.on_single_tap, cx);
            view.configure(&self.track, self.layout_mode, self.mirror_mode, cx);
        });
        div().size_full().child(view)
    }
}

pub struct RiseRTCVideoZoomContainerView {
    scroll_view: Entity<MediaZoomContainerView>,
    video_view: Entity<RiseRTCVideoSurfaceView>,
}

/// every live call zoom container, so a guide step that
/// taught zoom can return the picture to rest when it ends
#[derive(Default)]
struct RiseRTCVideoZoomContainers(Vec<WeakEntity<RiseRTCVideoZoomContainerView>>);

impl Global for RiseRTCVideoZoomContainers {}

impl RiseRTCVideoZoomContainerView {
    pub const MAXIMUM_ZOOM_SCALE: f32 = 5.0;

    fn new(cx: &mut Context<Self>) -> Self {
        let video_view = cx.new(|_| RiseRTCVideoSurfaceView::new());
        let zoomed_view: MediaZoomViewBuilder = {
            let video_view = video_view.clone();
            Rc::new(move |_: &mut Window, _: &mut App| -> AnyElement { video_view.clone().into_any_element() })
        };
        let scroll_view = cx.new(|cx| {
            let mut scroll_view = MediaZoomContainerView::new(zoomed_view);
            scroll_view.max_zoom = Self::MAXIMUM_ZOOM_SCALE;
            scroll_view.set_content_geometry(Size::default(), true, cx);
            scroll_view
        });
        let this = cx.entity().downgrade();
        let containers = cx.default_global::<RiseRTCVideoZoomContainers>();
        containers.0.retain(|container| container.upgrade().is_some());
        containers.0.push(this);
        Self { scroll_view, video_view }
    }

    pub fn reset_all_zoom(cx: &mut App) {
        let Some(containers) = cx.try_global::<RiseRTCVideoZoomContainers>() else {
            return;
        };
        let live: Vec<Entity<Self>> = containers.0.iter().filter_map(WeakEntity::upgrade).collect();
        for container in live {
            let scroll_view = container.read(cx).scroll_view.clone();
            scroll_view.update(cx, |scroll_view, cx| scroll_view.reset_zoom(true, cx));
        }
    }

    fn set_on_single_tap(&mut self, on_single_tap: MediaZoomSingleTap, cx: &mut Context<Self>) {
        self.scroll_view.update(cx, |scroll_view, _| scroll_view.on_single_tap = Some(on_single_tap));
    }

    fn configure(
        &mut self,
        track: &Rc<VideoTrack>,
        layout_mode: VideoViewLayoutMode,
        mirror_mode: VideoViewMirrorMode,
        cx: &mut Context<Self>,
    ) {
        let is_new_track = !self.video_view.read(cx).is_attached(track);
        self.video_view.update(cx, |view, cx| view.configure(track, layout_mode, mirror_mode, cx));
        if is_new_track {
            self.scroll_view.update(cx, |scroll_view, cx| scroll_view.reset_zoom(false, cx));
        }
    }
}

impl Render for RiseRTCVideoZoomContainerView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.scroll_view.clone())
    }
}
