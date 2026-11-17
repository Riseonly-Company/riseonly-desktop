use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyView, AnyWindowHandle, App, AppContext as _, Bounds, Context, IntoElement, ParentElement, Pixels, Render,
    SharedString, Styled, Window, div,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::animated_media_playback_scope::AnimatedMediaPlaybackActiveKey;
use rise_ui::swiftui::environment::env_scope;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::media_viewer::media_viewer_types::MediaViewerFrameKeys;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementContentPolicy, SharedElementContentPolicyVideoSource, SharedElementDirection,
    SharedElementGeometry, SharedElementKey, SharedElementPart, SharedElementSessionPolicyPhase,
    SharedElementVisibilityPolicy,
};
use crate::core::shared_element_transition::renderer::shared_element_bitmap_resolver::{
    SharedElementBitmapResolver, SharedElementBitmapResolverBitmap,
};
use crate::core::shared_element_transition::renderer::shared_element_video_frame_grabber::{
    SharedElementPlayerLayerView, SharedElementVideoFrameEpochGuard, SharedElementVideoFrameGrabber,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementFlightContent, SharedElementStopFrameRequest,
};
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::engine::rise_post_engine_models::RisePostViewSurface;
use crate::modules::post::pages::posts::post_detail::post_list_anchor_keeper::PostListAnchorKeeper;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// what the destination reads off the list scroll surface, gpui has no view object for that
/// the list owner creates it and attaches it to the destination
pub struct PostListSharedElementScrollView {
    window: AnyWindowHandle,
    scale: Cell<f32>,
    viewport: Box<dyn Fn() -> Bounds<Pixels>>,
}

impl PostListSharedElementScrollView {
    /// viewport gives the current bounds of the surface in window space
    pub fn new(window: AnyWindowHandle, scale: f32, viewport: impl Fn() -> Bounds<Pixels> + 'static) -> Self {
        Self { window, scale: Cell::new(scale), viewport: Box::new(viewport) }
    }

    pub fn update_scale(&self, scale: f32) {
        self.scale.set(scale);
    }
}

/// host view and scroll view as a capture reads them once
#[derive(Clone)]
pub struct PostListSharedElementHostView {
    pub session: u64,
    pub window: AnyWindowHandle,
    pub viewport: Bounds<Pixels>,
    pub scale: f32,
    pub player_layers: Vec<SharedElementPlayerLayerView>,
}

impl PostListSharedElementHostView {
    pub fn new(session: u64, scroll_view: &PostListSharedElementScrollView) -> Self {
        Self {
            session,
            window: scroll_view.window,
            viewport: (scroll_view.viewport)(),
            scale: scroll_view.scale.get(),
            player_layers: PostListSharedElementCapture::player_layers(session),
        }
    }
}

thread_local! {
    static PLAYER_LAYERS: RefCell<HashMap<u64, HashMap<String, SharedElementPlayerLayerView>>> =
        RefCell::new(HashMap::new());
}

struct PostListRowReplicant {
    post: Rc<GetPostFeedResponse>,
    row_frame: Bounds<Pixels>,
}

impl Render for PostListRowReplicant {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        div().relative().size_full().overflow_hidden().bg(theme.bg._100).child(
            div()
                .absolute()
                .left(self.row_frame.origin.x)
                .top(self.row_frame.origin.y)
                .w(self.row_frame.size.width)
                .child(
                    env_scope::<AnimatedMediaPlaybackActiveKey>(false).child(
                        Post::new(
                            SharedString::from(format!("post-list-replicant-{}", self.post.presentation_id())),
                            self.post.clone(),
                        )
                        .feed_row(true)
                        .view_surface(RisePostViewSurface::ProfileList),
                    ),
                ),
        )
    }
}

/// what a profile list post row flies back to the grid with
/// reads frame registry and image caches and the row player, memory only
/// only async step is stop frame grab of a playing video
pub enum PostListSharedElementCapture {}

impl PostListSharedElementCapture {
    pub const REPLICANT_IDENTITY: &'static str = "replicant";

    const BOX_ASPECT_TOLERANCE: f32 = 0.01;
    const MINIMUM_SHARPNESS: f32 = 0.5;

    pub fn capture(
        post: &GetPostFeedResponse,
        part: SharedElementPart,
        clip: Bounds<Pixels>,
        source_key: &SharedElementKey,
        host_view: &PostListSharedElementHostView,
        cx: &mut App,
    ) -> Option<SharedElementCapture> {
        if f32::from(clip.size.width) <= 1.0 || f32::from(clip.size.height) <= 1.0 {
            return None;
        }
        match part {
            SharedElementPart::Text => Self::replicant_capture(
                post,
                clip,
                source_key,
                SharedElementContentMode::AnchoredWidthFit,
                host_view,
                None,
                cx,
            ),
            SharedElementPart::Media => {
                if Self::uses_inline_video_card(post) {
                    return Self::reel_capture(post, clip, source_key, host_view, cx);
                }
                let slides = Self::gallery_slides(post);
                if !slides.is_empty() {
                    return Self::gallery_capture(post, &slides, clip, source_key, host_view, cx);
                }
                Self::replicant_capture(
                    post,
                    clip,
                    source_key,
                    SharedElementContentMode::AnchoredWidthFit,
                    host_view,
                    None,
                    cx,
                )
            }
        }
    }

    /// window frame of a registry record whose view lives in the hosts window
    pub fn hosted_frame(key: &str, host_view: &PostListSharedElementHostView) -> Option<Bounds<Pixels>> {
        let view = MediaCellScreenFrameRegistry::attached_view(key)?;
        if view.window()? != host_view.window {
            return None;
        }
        MediaCellScreenFrameRegistry::attached_frame(key)
    }

    /// true when media of this aspect fills a feed box shown in clip, same rule the row draws with
    pub fn media_fills(content_aspect: f32, data_box_natural: Option<f32>, clip: Bounds<Pixels>) -> bool {
        let clip_width = f32::from(clip.size.width);
        let clip_height = f32::from(clip.size.height);
        if !content_aspect.is_finite() || content_aspect <= 0.0 || clip_width <= 0.0 || clip_height <= 0.0 {
            return true;
        }
        let rect_box = clip_width / clip_height;
        if data_box_natural.is_some() {
            let data_box = PostMediaBox::display_aspect(data_box_natural);
            if (data_box - rect_box).abs() <= rect_box * Self::BOX_ASPECT_TOLERANCE {
                return PostMediaBox::fills(Some(content_aspect), data_box);
            }
        }
        (content_aspect - rect_box).abs() <= rect_box * Self::BOX_ASPECT_TOLERANCE
    }

    /// the rows page reports its player layer here, none when the player left the row
    pub fn update_player_layer(session: u64, post_id: &str, view: Option<SharedElementPlayerLayerView>) {
        PLAYER_LAYERS.with(|layers| {
            let mut layers = layers.borrow_mut();
            match view {
                Some(view) => {
                    layers.entry(session).or_default().insert(post_id.to_owned(), view);
                }
                None => {
                    if let Some(session_layers) = layers.get_mut(&session) {
                        session_layers.remove(post_id);
                    }
                }
            }
        });
    }

    fn player_layers(session: u64) -> Vec<SharedElementPlayerLayerView> {
        PLAYER_LAYERS.with(|layers| {
            layers.borrow().get(&session).map(|layers| layers.values().cloned().collect()).unwrap_or_default()
        })
    }

    pub(crate) fn remove_player_layers(session: u64) {
        PLAYER_LAYERS.with(|layers| {
            layers.borrow_mut().remove(&session);
        });
    }

    fn gallery_capture(
        post: &GetPostFeedResponse,
        slides: &[PostMediaSlide],
        clip: Bounds<Pixels>,
        source_key: &SharedElementKey,
        host_view: &PostListSharedElementHostView,
        cx: &mut App,
    ) -> Option<SharedElementCapture> {
        let page_frames: Vec<(usize, Bounds<Pixels>)> = (0..slides.len())
            .filter_map(|index| {
                let key = MediaViewerFrameKeys::post_image(&post.id, index);
                Self::hosted_frame(&key, host_view).map(|frame| (index, frame))
            })
            .collect();
        let picked = SharedElementVisibilityPolicy::pick_page(&page_frames, clip).and_then(|index| {
            page_frames.iter().find(|(page, _)| *page == index).map(|(_, frame)| (index, *frame))
        });
        let Some((index, page_frame)) = picked else {
            return Self::replicant_capture(
                post,
                clip,
                source_key,
                SharedElementContentMode::Fill,
                host_view,
                None,
                cx,
            );
        };
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        let box_natural = Self::gallery_box_natural(slides, is_own);

        match &slides[index] {
            PostMediaSlide::Image { url, .. } => {
                let bitmap = SharedElementBitmapResolver::full_size(url, is_own)
                    .and_then(|bitmap| Self::sharp(bitmap, page_frame, host_view.scale));
                let Some(bitmap) = bitmap else {
                    return Self::replicant_capture(
                        post,
                        clip,
                        source_key,
                        SharedElementContentMode::Fill,
                        host_view,
                        None,
                        cx,
                    );
                };
                let fills = Self::media_fills(bitmap.aspect, box_natural, clip);
                Some(Self::bitmap_capture(
                    &bitmap,
                    url,
                    if fills { None } else { Some(url.as_str()) },
                    is_own,
                    fills,
                    clip,
                    page_frame,
                    source_key,
                    None,
                    cx,
                ))
            }
            PostMediaSlide::Video { thumbnail, .. } => {
                let player_view = page_frame.intersects(&clip).then(|| page_frame.intersect(&clip)).and_then(
                    |visible_page| {
                        SharedElementVideoFrameGrabber::player_layer_view(&host_view.player_layers, visible_page)
                    },
                );
                let poster = thumbnail
                    .as_deref()
                    .and_then(|thumbnail| SharedElementBitmapResolver::full_size(thumbnail, is_own))
                    .or_else(|| SharedElementBitmapResolver::poster(post, cx));
                let poster_source = thumbnail.clone().or_else(|| Self::trimmed(post.video_thumbnail.as_deref()));
                Self::video_capture(
                    post,
                    clip,
                    page_frame,
                    source_key,
                    player_view,
                    poster,
                    poster_source,
                    |aspect| Self::media_fills(aspect, box_natural, clip),
                    host_view,
                    cx,
                )
            }
        }
    }

    fn gallery_slides(post: &GetPostFeedResponse) -> Vec<PostMediaSlide> {
        post.media_slides()
            .into_iter()
            .filter_map(|slide| match slide {
                PostMediaSlide::Image { url, size, image_index } => {
                    let value = trimming_whitespaces_and_newlines(&url).to_owned();
                    value.to_lowercase().starts_with("http").then_some(PostMediaSlide::Image {
                        url: value,
                        size,
                        image_index,
                    })
                }
                PostMediaSlide::Video { ref url, .. } => url.to_lowercase().starts_with("http").then_some(slide),
            })
            .collect()
    }

    fn gallery_box_natural(slides: &[PostMediaSlide], is_own: bool) -> Option<f32> {
        let first = slides.first()?;
        if let PostMediaSlide::Image { size, .. } = first
            && let Some(server) = PostMediaBox::aspect(size.as_ref())
        {
            return Some(server);
        }
        let source = first.poster_url()?;
        SharedElementBitmapResolver::full_size(source, is_own).map(|bitmap| bitmap.aspect)
    }

    fn uses_inline_video_card(post: &GetPostFeedResponse) -> bool {
        post.is_riff && post.images.is_empty() && post.has_video_media()
    }

    fn reel_capture(
        post: &GetPostFeedResponse,
        clip: Bounds<Pixels>,
        source_key: &SharedElementKey,
        host_view: &PostListSharedElementHostView,
        cx: &mut App,
    ) -> Option<SharedElementCapture> {
        let player_view = SharedElementVideoFrameGrabber::player_layer_view(&host_view.player_layers, clip);
        let poster = SharedElementBitmapResolver::poster(post, cx);
        let poster_source = Self::trimmed(post.video_thumbnail.as_deref());
        Self::video_capture(
            post,
            clip,
            clip,
            source_key,
            player_view,
            poster,
            poster_source,
            |aspect| Self::media_fills(aspect, Some(aspect), clip),
            host_view,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn video_capture(
        post: &GetPostFeedResponse,
        clip: Bounds<Pixels>,
        content_frame: Bounds<Pixels>,
        source_key: &SharedElementKey,
        player_view: Option<SharedElementPlayerLayerView>,
        poster: Option<SharedElementBitmapResolverBitmap>,
        poster_source: Option<String>,
        fills: impl Fn(f32) -> bool,
        host_view: &PostListSharedElementHostView,
        cx: &mut App,
    ) -> Option<SharedElementCapture> {
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        let is_playing =
            player_view.as_ref().is_some_and(|view| SharedElementVideoFrameGrabber::is_playing(view, cx));
        let sharp_poster = poster.and_then(|poster| Self::sharp(poster, content_frame, host_view.scale));
        let cover_source =
            if sharp_poster.is_none() { SharedElementBitmapResolver::grid_cover_source(post) } else { None };
        let grid_cover = cover_source.as_ref().and_then(|source| {
            SharedElementBitmapResolver::bitmap(std::slice::from_ref(source), is_own, &[None])
                .and_then(|bitmap| Self::sharp(bitmap, content_frame, host_view.scale))
        });
        let choice = SharedElementContentPolicy::video_choice(
            is_playing,
            false,
            false,
            sharp_poster.is_some(),
            grid_cover.is_some(),
            true,
        );
        if choice == SharedElementContentPolicyVideoSource::FadeOnly {
            return None;
        }

        let stop_frame = if is_playing {
            player_view.as_ref().and_then(|view| {
                let guard = Self::exit_session_guard(source_key, cx);
                SharedElementVideoFrameGrabber::make_stop_frame_request(view, guard, cx)
            })
        } else {
            None
        };
        let gravity_fills = player_view.as_ref().and_then(|view| view.player()).and_then(|player| {
            let presentation = player.read(cx).presentation_size();
            (presentation.width > 0.0 && presentation.height > 0.0)
                .then(|| fills(presentation.width / presentation.height))
        });

        let capture = match choice {
            SharedElementContentPolicyVideoSource::Poster
            | SharedElementContentPolicyVideoSource::GridCover
            | SharedElementContentPolicyVideoSource::StopFrame => {
                let is_poster = sharp_poster.is_some();
                if let Some(bitmap) = sharp_poster.or(grid_cover) {
                    let fills_box = gravity_fills.unwrap_or_else(|| fills(bitmap.aspect));
                    let identity = if is_poster { poster_source.clone() } else { cover_source };
                    let identity = identity.unwrap_or_else(|| bitmap.source.clone());
                    let backdrop_source = if fills_box { None } else { poster_source.as_deref() };
                    Some(Self::bitmap_capture(
                        &bitmap,
                        &identity,
                        backdrop_source,
                        is_own,
                        fills_box,
                        clip,
                        content_frame,
                        source_key,
                        stop_frame.clone(),
                        cx,
                    ))
                } else {
                    Self::replicant_capture(
                        post,
                        clip,
                        source_key,
                        SharedElementContentMode::Fill,
                        host_view,
                        stop_frame.clone(),
                        cx,
                    )
                }
            }
            SharedElementContentPolicyVideoSource::Replicant => Self::replicant_capture(
                post,
                clip,
                source_key,
                SharedElementContentMode::Fill,
                host_view,
                stop_frame.clone(),
                cx,
            ),
            SharedElementContentPolicyVideoSource::FadeOnly => None,
        };
        if let Some(stop_frame) = stop_frame {
            if capture.is_none() {
                stop_frame.cancel();
            } else {
                Self::hold_playback_on_settle(&stop_frame, source_key, cx);
            }
        }
        capture
    }

    /// none for a decode far below the pixels the frame covers on screen
    fn sharp(
        bitmap: SharedElementBitmapResolverBitmap,
        frame: Bounds<Pixels>,
        scale: f32,
    ) -> Option<SharedElementBitmapResolverBitmap> {
        let needed =
            f32::from(frame.size.width).max(f32::from(frame.size.height)) * scale.max(1.0) * Self::MINIMUM_SHARPNESS;
        let pixels = bitmap.image.size(0);
        let longest = pixels.width.0.max(pixels.height.0) as f32;
        (longest >= needed).then_some(bitmap)
    }

    #[allow(clippy::too_many_arguments)]
    fn bitmap_capture(
        bitmap: &SharedElementBitmapResolverBitmap,
        identity: &str,
        backdrop_source: Option<&str>,
        is_own: bool,
        fills: bool,
        clip: Bounds<Pixels>,
        content_frame: Bounds<Pixels>,
        source_key: &SharedElementKey,
        stop_frame: Option<Rc<SharedElementStopFrameRequest>>,
        cx: &mut App,
    ) -> SharedElementCapture {
        let content_rect = if fills {
            SharedElementGeometry::fill_rect(bitmap.aspect, content_frame)
        } else {
            SharedElementGeometry::fit_rect(bitmap.aspect, content_frame)
        };
        let backdrop = backdrop_source.and_then(|source| {
            let guard = Self::exit_session_guard(source_key, cx);
            let adopt_key = source_key.clone();
            SharedElementBitmapResolver::backdrop(
                source,
                is_own,
                content_frame.size,
                move |cx: &App| guard(cx),
                move |image: CleverImage, cx: &mut App| Self::adopt_backdrop(image, &adopt_key, cx),
                cx,
            )
        });
        SharedElementCapture {
            item_id: source_key.item_id.clone(),
            part: source_key.part,
            source_key: source_key.clone(),
            clip,
            content: bitmap.flight_content(),
            content_rect,
            mode: if fills {
                SharedElementContentMode::Fill
            } else {
                SharedElementContentMode::Fit { backdrop: backdrop_source.is_some() }
            },
            backdrop,
            flying_identity: Some(identity.to_owned()),
            stop_frame,
            live_readiness: None,
            placeholder: None,
        }
    }

    /// row drawn again for the visible part of clip so the floating header stays out of it
    fn replicant_capture(
        post: &GetPostFeedResponse,
        clip: Bounds<Pixels>,
        source_key: &SharedElementKey,
        mode: SharedElementContentMode,
        host_view: &PostListSharedElementHostView,
        stop_frame: Option<Rc<SharedElementStopFrameRequest>>,
        cx: &mut App,
    ) -> Option<SharedElementCapture> {
        if !clip.intersects(&host_view.viewport) {
            return None;
        }
        let rect = clip.intersect(&host_view.viewport);
        if f32::from(rect.size.width) < 1.0 || f32::from(rect.size.height) < 1.0 {
            return None;
        }
        let row_key = PostListAnchorKeeper::row_frame_key(host_view.session, &source_key.item_id);
        let row = MediaCellScreenFrameRegistry::attached_frame(&row_key).unwrap_or(rect);
        let row_frame = Bounds::new(row.origin - rect.origin, row.size);
        let post = Rc::new(post.clone());
        let view: AnyView = cx.new(|_| PostListRowReplicant { post, row_frame }).into();
        Some(SharedElementCapture {
            item_id: source_key.item_id.clone(),
            part: source_key.part,
            source_key: source_key.clone(),
            clip: rect,
            content: SharedElementFlightContent::Replicant(view, rect.size),
            content_rect: rect,
            mode,
            backdrop: None,
            flying_identity: Some(Self::REPLICANT_IDENTITY.to_owned()),
            stop_frame,
            live_readiness: None,
            placeholder: None,
        })
    }

    /// true while the exit that samples this capture is the live session
    fn exit_session_guard(key: &SharedElementKey, cx: &mut App) -> SharedElementVideoFrameEpochGuard {
        let capture_epoch = SharedElementTransitionCoordinator::shared(cx).read(cx).epoch();
        let key = key.clone();
        Rc::new(move |cx: &App| {
            let Some(coordinator) = rise_ui::shared::try_read::<SharedElementTransitionCoordinator>(cx) else {
                return false;
            };
            if coordinator.epoch() == capture_epoch {
                return coordinator.phase() == SharedElementSessionPolicyPhase::Idle;
            }
            if coordinator.epoch() != capture_epoch.wrapping_add(1) {
                return false;
            }
            let Some(intent) = coordinator.intent() else {
                return false;
            };
            intent.direction == SharedElementDirection::Exit
                && intent.capture().is_some_and(|capture| capture.source_key == key)
        })
    }

    fn hold_playback_on_settle(request: &Rc<SharedElementStopFrameRequest>, key: &SharedElementKey, cx: &mut App) {
        let coordinator = SharedElementTransitionCoordinator::shared(cx);
        let capture_epoch = coordinator.read(cx).epoch();
        let key = key.clone();
        let app = cx.to_async();
        let executor = cx.foreground_executor().clone();
        request.on_settle(move |_| {
            executor
                .spawn(async move {
                    app.update(|cx| {
                        let epoch = {
                            let coordinator = coordinator.read(cx);
                            let Some(intent) = coordinator.intent() else {
                                return;
                            };
                            if coordinator.epoch() != capture_epoch.wrapping_add(1)
                                || coordinator.phase() == SharedElementSessionPolicyPhase::Idle
                                || intent.epoch != coordinator.epoch()
                                || intent.direction != SharedElementDirection::Exit
                                || !intent.capture().is_some_and(|capture| capture.source_key == key)
                            {
                                return;
                            }
                            intent.epoch
                        };
                        coordinator.update(cx, |coordinator, cx| coordinator.hold_inline_playback(epoch, cx));
                    });
                })
                .detach();
        });
    }

    fn adopt_backdrop(image: CleverImage, key: &SharedElementKey, cx: &mut App) {
        let Some(intent) = SharedElementTransitionCoordinator::shared(cx).read(cx).intent() else {
            return;
        };
        if intent.direction != SharedElementDirection::Exit {
            return;
        }
        let Some(mut current) = intent.capture().filter(|capture| capture.source_key == *key) else {
            return;
        };
        current.backdrop = Some(image);
        intent.set_capture(Some(current));
    }

    fn trimmed(value: Option<&str>) -> Option<String> {
        let result = trimming_whitespaces_and_newlines(value.unwrap_or_default());
        (!result.is_empty()).then(|| result.to_owned())
    }
}
