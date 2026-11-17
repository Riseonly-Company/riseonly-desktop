use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, Context, Div, Entity, EntityId, Font as GpuiFont, FontWeight, Hsla,
    IntoElement, ParentElement, Pixels, Render, ShapedLine, SharedString, StyleRefinement, Styled, Svg, TextAlign,
    TextRun, TruncateFrom, Window, canvas, div, point, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::font::Font;
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec, spring};
use rise_ui::swiftui::rich_text::RichFont;
use rise_ui::window_screen_frame_reader::{
    MediaCellScreenFrameRegistry, WindowScreenFrameReader, WindowScreenFrameReaderFrameView,
};

use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementPart;
use crate::core::ui::list_engine::rise_manual_list_integration::{
    RiseManualListConformances, RiseManualListViewportVisibilityReceiving,
};
use crate::core::ui::rich_text::post_rich_text_render::{PostRichTextCalibration, PostRichTextRender};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListReusableView;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_leaf_mount_queue::PostListReusableRowConformances;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_playback_focus::PostListFocusFrameView;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListPlaybackToken, PostListPlaybackTokenKind, PostListReusableRow, PostRowActionRouting,
    PostRowAnchorRegistering, PostRowBodyKind, PostRowCaptionLayout, PostRowLayout, PostRowLayoutSpec,
    PostRowMainContext, PostRowMediaBoxLayout, PostRowPalette, PreparedPostRow,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::{
    PostRowMetrics, PostRowTextMetrics,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_actions_view::PostRowActionsView;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_caption_view::PostRowCaptionView;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_header_view::PostRowHeaderView;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_heart_overlay::{
    PostRowHeartOverlay, PostRowHeartOverlaySurface,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_hosted_leaf::PostRowAmbientMusicDriver;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_likes_summary_view::PostRowLikesSummaryView;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_moderation_banner_view::PostRowModerationBannerView;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_music_strip_view::PostRowMusicStripView;

/// places a part at its row layout frame, in the coords of its parent
pub(crate) fn post_row_frame(frame: Bounds<Pixels>) -> Div {
    div()
        .absolute()
        .left(frame.origin.x)
        .top(frame.origin.y)
        .w(frame.size.width)
        .h(frame.size.height)
}

/// part view of the row (header
/// banner, music strip
/// actions, likes) as a cached view that replays when moved
fn post_row_part(view: impl Into<AnyView>) -> AnyElement {
    view.into().cached(StyleRefinement::default().size_full()).replay_when_moved().into_any_element()
}

pub type PostRowMediaDoubleTap = Rc<dyn Fn(&mut App)>;

/// media box a native row hosts, implemented by the media package
pub trait PostRowMediaHosting {
    /// the box view, the row draws it at the media box layout frame
    fn host_view(&self) -> AnyView;
    /// the media anchor the row registers, pages or card
    /// without the overlays that stay visible during a lift
    /// same frame as host_view, none until the box was drawn once
    fn anchor_view(&self, cx: &App) -> Option<WindowScreenFrameReaderFrameView>;
    fn set_anchor_lifted(&self, is_lifted: bool, cx: &mut App);
    fn set_row_frame_view(&self, row_frame_view: Rc<PostListFocusFrameView>, cx: &mut App);
    fn set_on_double_tap(&self, on_double_tap: Option<PostRowMediaDoubleTap>, cx: &mut App);
    fn configure(
        &self,
        prepared: &Arc<PreparedPostRow>,
        layout: &PostRowMediaBoxLayout,
        context: &PostRowMainContext,
        anchors: Rc<dyn PostRowAnchorRegistering>,
        cx: &mut App,
    );
    fn prepare_for_row_reuse(&self, cx: &mut App);
    fn set_hosted_leaves_active(&self, is_active: bool, cx: &mut App);
    fn set_animated_media_visible_in_viewport(&self, is_visible: bool, cx: &mut App);
}

/// builds the media box of a gallery or riff row, once per row view
pub type PostRowMediaHostFactory = Rc<dyn Fn(EntityId, &mut App) -> Rc<dyn PostRowMediaHosting>>;

/// native post row with header, moderation banner and music strip or media box and caption
/// actions and likes summary and heart and separator frames only come from the row layout
pub struct PostNativeRowView {
    media_host_factory: Option<PostRowMediaHostFactory>,
    header_view: Entity<PostRowHeaderView>,
    has_pending_anchor_registration: bool,
    text_reader_id: String,
    is_media_lifted: bool,
    is_text_lifted: bool,
    on_caption_double_tap: Rc<dyn Fn(&mut Window, &mut App)>,
    actions_view: Entity<PostRowActionsView>,
    banner_view: Option<Entity<PostRowModerationBannerView>>,
    music_strip_view: Option<Entity<PostRowMusicStripView>>,
    likes_summary_view: Option<Entity<PostRowLikesSummaryView>>,
    media_host: Option<Rc<dyn PostRowMediaHosting>>,
    heart_overlay: Option<Entity<PostRowHeartOverlay>>,
    is_caption_shown: bool,
    is_banner_shown: bool,
    is_music_strip_shown: bool,
    is_likes_summary_shown: bool,
    is_media_host_shown: bool,

    configured_presentation_id: Option<String>,
    prepared: Option<Arc<PreparedPostRow>>,
    row_layout: Option<Arc<PostRowLayout>>,
    palette: Option<PostRowPalette>,
    context: Option<PostRowMainContext>,
    router: Option<Weak<dyn PostRowActionRouting>>,
    anchors: Option<Rc<dyn PostRowAnchorRegistering>>,
    ambient_driver: Option<Entity<PostRowAmbientMusicDriver>>,
    leaves_active: bool,
    media_double_tap_enabled: bool,
    frame_view: Rc<PostListFocusFrameView>,
}

impl PostNativeRowView {
    /// media_host_factory is required for rows with a media box, plain rows can pass none
    pub fn new(media_host_factory: Option<PostRowMediaHostFactory>, cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_moved_replay_row::<Self>(cx);
        RiseManualListConformances::register_viewport_visibility_receiving::<Self>(cx);
        PostListReusableRowConformances::register::<Self>(cx);

        let row_view = cx.entity_id();
        let header_view = cx.new(PostRowHeaderView::new);
        let actions_view = cx.new(PostRowActionsView::new);
        let row = cx.weak_entity();
        let on_caption_double_tap: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = row.update(cx, |row, cx| row.handle_double_tap(PostRowHeartOverlaySurface::Copy, cx));
        });
        Self {
            media_host_factory,
            header_view,
            has_pending_anchor_registration: false,
            text_reader_id: format!("post-row-text-{}", row_view.as_u64()),
            is_media_lifted: false,
            is_text_lifted: false,
            on_caption_double_tap,
            actions_view,
            banner_view: None,
            music_strip_view: None,
            likes_summary_view: None,
            media_host: None,
            heart_overlay: None,
            is_caption_shown: false,
            is_banner_shown: false,
            is_music_strip_shown: false,
            is_likes_summary_shown: false,
            is_media_host_shown: false,
            configured_presentation_id: None,
            prepared: None,
            row_layout: None,
            palette: None,
            context: None,
            router: None,
            anchors: None,
            ambient_driver: None,
            leaves_active: false,
            media_double_tap_enabled: false,
            frame_view: Rc::new(PostListFocusFrameView::new(row_view)),
        }
    }

    /// padded caption surface, full width
    /// top 8 or 5, bottom 2
    /// the double tap target
    fn caption_surface_frame(caption: &PostRowCaptionLayout, layout: &PostRowLayout) -> Bounds<Pixels> {
        let top = if layout.inputs.body == PostRowBodyKind::Reel || layout.inputs.has_visual_media {
            PostRowMetrics::CAPTION_TOP_WITH_MEDIA
        } else {
            PostRowMetrics::CAPTION_TOP_WITHOUT_MEDIA
        };
        Bounds::new(
            point(px(0.0), caption.frame.origin.y - px(top)),
            size(
                layout.separator.size.width,
                px(top) + caption.frame.size.height + px(PostRowMetrics::CAPTION_BOTTOM),
            ),
        )
    }

    /// standard body overlays the padded copy and the reel body the unpadded one
    fn heart_rect(surface: PostRowHeartOverlaySurface, layout: &PostRowLayout) -> Option<Bounds<Pixels>> {
        match surface {
            PostRowHeartOverlaySurface::Media => layout.media_box.as_ref().map(|media_box| media_box.frame),
            PostRowHeartOverlaySurface::Copy => {
                let caption = layout.caption.as_ref()?;
                Some(if layout.inputs.body == PostRowBodyKind::Reel {
                    caption.frame
                } else {
                    Self::caption_surface_frame(caption, layout)
                })
            }
        }
    }

    fn handle_double_tap(&mut self, surface: PostRowHeartOverlaySurface, cx: &mut Context<Self>) {
        let (Some(prepared), Some(router), Some(layout), Some(palette)) = (
            self.prepared.clone(),
            self.router.as_ref().and_then(Weak::upgrade),
            self.row_layout.clone(),
            self.palette.clone(),
        ) else {
            return;
        };
        let was_liked =
            router.post(&prepared.id, cx).map(|post| post.is_liked).unwrap_or(prepared.seed.counts.is_liked);
        router.like_from_double_tap(&prepared.id, cx);
        if !was_liked {
            self.actions_view.update(cx, |actions, cx| actions.bump_like(cx));
        }
        let Some(rect) = Self::heart_rect(surface, &layout) else {
            return;
        };
        let overlay = self.ensure_heart_overlay(cx);
        overlay.update(cx, |overlay, cx| {
            overlay.place(rect, cx);
            overlay.show(surface, palette.primary100, cx);
        });
        cx.notify();
    }

    fn reset_post_state(&mut self, cx: &mut Context<Self>) {
        if let Some(anchors) = self.anchors.as_ref() {
            anchors.unregister_all(cx);
        }
        if let Some(heart_overlay) = self.heart_overlay.as_ref() {
            heart_overlay.update(cx, |overlay, cx| overlay.cancel(cx));
        }
        self.actions_view.update(cx, |actions, cx| actions.cancel_bumps(cx));
    }

    fn anchor_registration(
        &mut self,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) -> Rc<dyn PostRowAnchorRegistering> {
        if let Some(anchors) = self.anchors.as_ref() {
            if anchors.surface() == context.shared_element_surface {
                return anchors.clone();
            }
            anchors.unregister_all(cx);
        }
        let registration = (context.make_anchor_registration)(cx);
        let row = cx.weak_entity();
        registration.set_on_lift_change(Some(Rc::new(move |part: SharedElementPart, is_hidden: bool, cx: &mut App| {
            let row = row.clone();
            cx.defer(move |cx| {
                let _ = row.update(cx, |row, cx| row.apply_lift(part, is_hidden, cx));
            });
        })));
        self.anchors = Some(registration.clone());
        registration
    }

    fn apply_lift(&mut self, part: SharedElementPart, is_hidden: bool, cx: &mut Context<Self>) {
        match part {
            SharedElementPart::Media => {
                self.is_media_lifted = is_hidden;
                if let Some(view) = self.music_strip_view.as_ref() {
                    view.update(cx, |view, cx| view.set_anchor_lifted(is_hidden, cx));
                }
                if let Some(media_host) = self.media_host.as_ref() {
                    media_host.set_anchor_lifted(is_hidden, cx);
                }
            }
            SharedElementPart::Text => self.is_text_lifted = is_hidden,
        }
        cx.notify();
    }

    /// registers anchors of the shown parts once their reader has drawn, safe to repeat
    fn register_drawn_anchors(&mut self, cx: &mut Context<Self>) {
        let (Some(prepared), Some(registration)) = (self.prepared.clone(), self.anchors.clone()) else {
            self.has_pending_anchor_registration = false;
            return;
        };
        let post_id = prepared.seed.post_id.as_str();
        let mut is_pending = false;
        let media_anchor_view = if self.is_media_host_shown {
            self.media_host.as_ref().map(|host| host.anchor_view(cx))
        } else if self.is_music_strip_shown {
            self.music_strip_view.as_ref().map(|view| view.read(cx).anchor_view())
        } else {
            None
        };
        match media_anchor_view {
            Some(Some(view)) => registration.register_media(&view, post_id, cx),
            Some(None) => is_pending = true,
            None => {}
        }
        if self.is_caption_shown {
            match MediaCellScreenFrameRegistry::attached_view(&self.text_reader_id) {
                Some(view) => registration.register_text(&view, post_id, cx),
                None => is_pending = true,
            }
        }
        self.has_pending_anchor_registration = is_pending;
    }

    fn ensure_banner_view(&mut self, cx: &mut Context<Self>) -> Entity<PostRowModerationBannerView> {
        if let Some(banner_view) = self.banner_view.as_ref() {
            return banner_view.clone();
        }
        let view = cx.new(PostRowModerationBannerView::new);
        self.banner_view = Some(view.clone());
        view
    }

    fn ensure_music_strip_view(&mut self, cx: &mut Context<Self>) -> Entity<PostRowMusicStripView> {
        if let Some(music_strip_view) = self.music_strip_view.as_ref() {
            return music_strip_view.clone();
        }
        let row_view = cx.entity_id();
        let view = cx.new(|cx| PostRowMusicStripView::new(row_view, cx));
        self.music_strip_view = Some(view.clone());
        view
    }

    fn ensure_media_host(&mut self, cx: &mut Context<Self>) -> Option<Rc<dyn PostRowMediaHosting>> {
        if let Some(media_host) = self.media_host.as_ref() {
            return Some(media_host.clone());
        }
        let media_host_factory = self.media_host_factory.clone()?;
        let host = media_host_factory(cx.entity_id(), cx);
        host.set_row_frame_view(self.frame_view.clone(), cx);
        self.media_host = Some(host.clone());
        Some(host)
    }

    fn ensure_likes_summary_view(&mut self, cx: &mut Context<Self>) -> Entity<PostRowLikesSummaryView> {
        if let Some(likes_summary_view) = self.likes_summary_view.as_ref() {
            return likes_summary_view.clone();
        }
        let view = cx.new(PostRowLikesSummaryView::new);
        self.likes_summary_view = Some(view.clone());
        view
    }

    fn ensure_ambient_driver(&mut self, cx: &mut Context<Self>) -> Entity<PostRowAmbientMusicDriver> {
        if let Some(ambient_driver) = self.ambient_driver.as_ref() {
            return ambient_driver.clone();
        }
        let driver = cx.new(PostRowAmbientMusicDriver::new);
        self.ambient_driver = Some(driver.clone());
        driver
    }

    fn ensure_heart_overlay(&mut self, cx: &mut Context<Self>) -> Entity<PostRowHeartOverlay> {
        if let Some(heart_overlay) = self.heart_overlay.as_ref() {
            return heart_overlay.clone();
        }
        let view = cx.new(PostRowHeartOverlay::new);
        self.heart_overlay = Some(view.clone());
        view
    }
}

impl PostListReusableRow for PostNativeRowView {
    fn configured_presentation_id(&self) -> Option<&str> {
        self.configured_presentation_id.as_deref()
    }

    fn configure(&mut self, prepared: &Arc<PreparedPostRow>, context: &PostRowMainContext, cx: &mut Context<Self>) {
        let Some(layout) = prepared.layout.clone() else {
            self.prepare_for_manual_list_reuse(cx);
            return;
        };
        if let (Some(previous_row), Some(previous_context)) = (self.prepared.as_ref(), self.context.as_ref())
            && Arc::ptr_eq(previous_row, prepared)
            && Rc::ptr_eq(&previous_context.make_anchor_registration, &context.make_anchor_registration)
        {
            return;
        }
        if self.configured_presentation_id.as_deref() != Some(prepared.presentation_id.as_str()) {
            self.reset_post_state(cx);
        }
        tracing::warn!(
            target: "riseonly::X1856",
            "row configure id={} spec_width={} previous_width={:?}",
            prepared.presentation_id,
            context.spec.width,
            self.context.as_ref().map(|previous| previous.spec.width)
        );
        self.prepared = Some(prepared.clone());
        self.row_layout = Some(layout.clone());
        self.palette = Some(context.palette.clone());
        self.context = Some(context.clone());
        self.router = Some(Rc::downgrade(&context.router));
        self.configured_presentation_id = Some(prepared.presentation_id.clone());

        let seed = &prepared.seed;
        let registration = self.anchor_registration(context, cx);

        self.header_view.update(cx, |header, cx| header.configure(seed, &layout.header, context, cx));

        if let Some(banner) = layout.moderation_banner.as_ref() {
            let view = self.ensure_banner_view(cx);
            self.is_banner_shown = true;
            view.update(cx, |view, cx| view.configure(banner, context, cx));
        } else {
            self.is_banner_shown = false;
        }

        if let (Some(strip), Some(track)) = (layout.music_strip.as_ref(), seed.track.as_ref()) {
            let view = self.ensure_music_strip_view(cx);
            self.is_music_strip_shown = true;
            view.update(cx, |view, cx| view.configure(&seed.post_id, track, strip, context, cx));
        } else if self.is_music_strip_shown
            && let Some(view) = self.music_strip_view.clone()
        {
            if let Some(anchor_view) = view.read(cx).anchor_view() {
                registration.unregister(&anchor_view, cx);
            }
            view.update(cx, |view, cx| view.prepare_for_reuse(cx));
            self.is_music_strip_shown = false;
        }

        let media_host = if layout.media_box.is_some() { self.ensure_media_host(cx) } else { None };
        if let (Some(media_box), Some(host)) = (layout.media_box.as_ref(), media_host) {
            self.is_media_host_shown = true;
            let wants_double_tap = seed.body == PostRowBodyKind::Standard;
            if wants_double_tap != self.media_double_tap_enabled {
                self.media_double_tap_enabled = wants_double_tap;
                if wants_double_tap {
                    let row = cx.weak_entity();
                    let on_double_tap: PostRowMediaDoubleTap = Rc::new(move |cx: &mut App| {
                        let _ =
                            row.update(cx, |row, cx| row.handle_double_tap(PostRowHeartOverlaySurface::Media, cx));
                    });
                    host.set_on_double_tap(Some(on_double_tap), cx);
                } else {
                    host.set_on_double_tap(None, cx);
                }
            }
            host.configure(prepared, media_box, context, registration.clone(), cx);
        } else if self.is_media_host_shown
            && let Some(host) = self.media_host.clone()
        {
            if let Some(anchor_view) = host.anchor_view(cx) {
                registration.unregister(&anchor_view, cx);
            }
            host.prepare_for_row_reuse(cx);
            self.is_media_host_shown = false;
        }

        if layout.caption.is_some() {
            self.is_caption_shown = true;
        } else if self.is_caption_shown {
            if let Some(view) = MediaCellScreenFrameRegistry::attached_view(&self.text_reader_id) {
                registration.unregister(&view, cx);
            }
            self.is_caption_shown = false;
        }
        self.register_drawn_anchors(cx);

        self.actions_view
            .update(cx, |view, cx| view.configure(&seed.post_id, &seed.counts, &layout.actions, context, cx));

        if let (Some(likes), false) = (layout.likes_summary.as_ref(), seed.liked_by.is_empty()) {
            let view = self.ensure_likes_summary_view(cx);
            self.is_likes_summary_shown = true;
            view.update(cx, |view, cx| {
                view.configure(&seed.post_id, &seed.liked_by, seed.counts.likes, likes, context, cx)
            });
        } else if self.is_likes_summary_shown
            && let Some(view) = self.likes_summary_view.as_ref()
        {
            view.update(cx, |view, cx| view.prepare_for_reuse(cx));
            self.is_likes_summary_shown = false;
        }

        if seed.body == PostRowBodyKind::Standard && seed.has_ambient_music && layout.media_box.is_none() {
            let driver = self.ensure_ambient_driver(cx);
            let token = PostListPlaybackToken {
                session: context.session,
                presentation_id: seed.presentation_id.clone(),
                kind: PostListPlaybackTokenKind::AmbientMusic,
            };
            let frame_view = self.frame_view.clone();
            let is_active = self.leaves_active;
            driver.update(cx, |driver, cx| {
                driver.configure(&prepared.post, token, context.focus.clone(), frame_view, cx);
                driver.set_active(is_active, cx);
            });
        } else if let Some(ambient_driver) = self.ambient_driver.as_ref() {
            ambient_driver.update(cx, |driver, cx| driver.stop(cx));
        }

        if let Some(heart_overlay) = self.heart_overlay.as_ref() {
            let surface = heart_overlay.read(cx).surface();
            if let Some(rect) = surface.and_then(|surface| Self::heart_rect(surface, &layout)) {
                heart_overlay.update(cx, |overlay, cx| overlay.place(rect, cx));
            }
        }

        cx.notify();
    }

    fn set_hosted_leaves_active(&mut self, is_active: bool, cx: &mut Context<Self>) {
        self.leaves_active = is_active;
        self.header_view.update(cx, |header, cx| header.set_hosted_leaves_active(is_active, cx));
        if let Some(ambient_driver) = self.ambient_driver.as_ref() {
            ambient_driver.update(cx, |driver, cx| driver.set_active(is_active, cx));
        }
        if self.is_media_host_shown
            && let Some(media_host) = self.media_host.as_ref()
        {
            media_host.set_hosted_leaves_active(is_active, cx);
        }
    }
}

impl RiseManualListReusableView for PostNativeRowView {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>) {
        self.leaves_active = false;
        if let Some(ambient_driver) = self.ambient_driver.as_ref() {
            ambient_driver.update(cx, |driver, cx| driver.stop(cx));
        }
        if let Some(anchors) = self.anchors.as_ref() {
            anchors.unregister_all(cx);
        }
        if let Some(heart_overlay) = self.heart_overlay.as_ref() {
            heart_overlay.update(cx, |overlay, cx| overlay.cancel(cx));
        }
        self.header_view.update(cx, |header, cx| header.prepare_for_reuse(cx));
        self.is_caption_shown = false;
        self.actions_view.update(cx, |actions, cx| actions.prepare_for_reuse(cx));
        self.is_banner_shown = false;
        if let Some(music_strip_view) = self.music_strip_view.as_ref() {
            music_strip_view.update(cx, |view, cx| view.prepare_for_reuse(cx));
        }
        self.is_music_strip_shown = false;
        if let Some(likes_summary_view) = self.likes_summary_view.as_ref() {
            likes_summary_view.update(cx, |view, cx| view.prepare_for_reuse(cx));
        }
        self.is_likes_summary_shown = false;
        if let Some(media_host) = self.media_host.as_ref() {
            media_host.prepare_for_row_reuse(cx);
        }
        self.is_media_host_shown = false;
        self.frame_view.clear_painted();
        self.has_pending_anchor_registration = false;
        self.is_media_lifted = false;
        self.is_text_lifted = false;
        self.prepared = None;
        self.row_layout = None;
        self.context = None;
        self.configured_presentation_id = None;
        cx.notify();
    }
}

impl RiseManualListViewportVisibilityReceiving for PostNativeRowView {
    fn set_animated_media_visible_in_viewport(&mut self, is_visible: bool, cx: &mut App) {
        if self.is_media_host_shown
            && let Some(media_host) = self.media_host.as_ref()
        {
            media_host.set_animated_media_visible_in_viewport(is_visible, cx);
        }
    }
}

impl Render for PostNativeRowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.has_pending_anchor_registration {
            let row = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = row.update(cx, |row, cx| row.register_drawn_anchors(cx));
            });
        }
        let (Some(layout), Some(palette)) = (self.row_layout.as_ref(), self.palette.as_ref()) else {
            return div().into_any_element();
        };
        let frame_view = self.frame_view.clone();
        let mut row = div()
            .relative()
            .size_full()
            .bg(palette.background100)
            .child(frame_view.probe())
            .child(post_row_frame(layout.separator).bg(palette.border100))
            .child(post_row_frame(layout.header.frame).child(post_row_part(self.header_view.clone())));
        if self.is_banner_shown
            && let (Some(banner), Some(view)) = (layout.moderation_banner.as_ref(), self.banner_view.as_ref())
        {
            row = row.child(post_row_frame(banner.frame).child(post_row_part(view.clone())));
        }
        if self.is_music_strip_shown
            && let (Some(strip), Some(view)) = (layout.music_strip.as_ref(), self.music_strip_view.as_ref())
        {
            row = row.child(post_row_frame(strip.frame).child(post_row_part(view.clone())));
        }
        if self.is_media_host_shown
            && let (Some(media_box), Some(host)) = (layout.media_box.as_ref(), self.media_host.as_ref())
        {
            row = row.child(post_row_frame(media_box.frame).child(host.host_view()));
        }
        if self.is_caption_shown
            && let (Some(caption), Some(context), Some(prepared)) =
                (layout.caption.as_ref(), self.context.as_ref(), self.prepared.as_ref())
        {
            let surface = Self::caption_surface_frame(caption, layout);
            let content_frame = Bounds::new(
                point(caption.frame.origin.x - surface.origin.x, caption.frame.origin.y - surface.origin.y),
                caption.frame.size,
            );
            let mut caption_surface =
                post_row_frame(surface).child(post_row_frame(content_frame).child(WindowScreenFrameReader::new(self.text_reader_id.clone())));
            if !self.is_text_lifted {
                caption_surface = caption_surface.child(
                    PostRowCaptionView::new(prepared.seed.post_id.clone(), caption.clone(), content_frame, context)
                        .on_double_tap(self.on_caption_double_tap.clone()),
                );
            }
            row = row.child(caption_surface);
        }
        row = row.child(post_row_frame(layout.actions.frame).child(post_row_part(self.actions_view.clone())));
        if self.is_likes_summary_shown
            && let (Some(likes), Some(view)) = (layout.likes_summary.as_ref(), self.likes_summary_view.as_ref())
        {
            row = row.child(post_row_frame(likes.frame).child(post_row_part(view.clone())));
        }
        if let Some(heart_overlay) = self.heart_overlay.as_ref() {
            row = row.child(heart_overlay.clone());
        }
        row.into_any_element()
    }
}

#[derive(Clone, PartialEq)]
pub struct PostRowTextLineViewSegment {
    pub text: String,
    pub font: RichFont,
    pub color: Hsla,
    pub truncates: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostRowTextLineViewAlignment {
    #[default]
    Leading,
    Center,
}

/// one line of main text segments shaped at paint time
/// frames come from the layout, nothing here measures
/// the parent draws it inside the line frame and it takes no mouse
#[derive(Default)]
pub struct PostRowTextLineView {
    segments: Rc<[PostRowTextLineViewSegment]>,
    spacing: f32,
    alignment: PostRowTextLineViewAlignment,
    drawn_segment_frames: Vec<Bounds<Pixels>>,
    shaped: Option<PostRowTextLineViewShaped>,
}

#[derive(Clone)]
struct PostRowTextLineViewShaped {
    width: Pixels,
    height: Pixels,
    scale: f32,
    lines: Rc<[(ShapedLine, f32)]>,
}

impl PostRowTextLineView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn drawn_segment_frames(&self) -> &[Bounds<Pixels>] {
        &self.drawn_segment_frames
    }

    pub fn set_text(
        &mut self,
        text: &str,
        font: RichFont,
        color: Hsla,
        alignment: PostRowTextLineViewAlignment,
        cx: &mut Context<Self>,
    ) {
        let segments = if text.is_empty() {
            Vec::new()
        } else {
            vec![PostRowTextLineViewSegment { text: text.to_owned(), font, color, truncates: true }]
        };
        self.set_segments(segments, 0.0, alignment, cx);
    }

    pub fn set_segments(
        &mut self,
        segments: Vec<PostRowTextLineViewSegment>,
        spacing: f32,
        alignment: PostRowTextLineViewAlignment,
        cx: &mut Context<Self>,
    ) {
        if *segments == *self.segments && spacing == self.spacing && alignment == self.alignment {
            return;
        }
        self.segments = segments.into();
        self.spacing = spacing;
        self.alignment = alignment;
        self.shaped = None;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if self.segments.is_empty() {
            return;
        }
        self.segments = Rc::from(Vec::new());
        self.drawn_segment_frames.clear();
        self.shaped = None;
        cx.notify();
    }

    fn lay_out_lines(
        segments: &[PostRowTextLineViewSegment],
        spacing: f32,
        alignment: PostRowTextLineViewAlignment,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> (Vec<(ShapedLine, f32)>, Vec<Bounds<Pixels>>) {
        let bounds_width = f32::from(bounds.size.width);
        if segments.is_empty() || bounds_width <= 0.0 {
            return (Vec::new(), Vec::new());
        }
        let scale = window.scale_factor();
        let mut lines: Vec<Option<ShapedLine>> = Vec::with_capacity(segments.len());
        let mut widths: Vec<f32> = Vec::with_capacity(segments.len());
        for segment in segments {
            let line = Self::shape(segment.text.trim_end(), segment, window);
            widths.push(f32::from(line.width()).max(0.0));
            lines.push(Some(line));
        }

        let gaps = segments.len().saturating_sub(1) as f32 * spacing;
        let mut overflow = widths.iter().sum::<f32>() + gaps - bounds_width;
        if overflow > 0.01 {
            let mut order: Vec<usize> = (0..segments.len()).filter(|index| segments[*index].truncates).collect();
            if order.is_empty() {
                order.push(segments.len() - 1);
            }
            for index in order {
                if overflow <= 0.01 {
                    break;
                }
                let allowed = widths[index] - overflow;
                let cut = (allowed > 0.0).then(|| Self::truncated(&segments[index], allowed, window, cx));
                if let Some(cut) = cut {
                    let cut_width = f32::from(cut.width());
                    overflow -= widths[index] - cut_width;
                    lines[index] = Some(cut);
                    widths[index] = cut_width;
                } else {
                    overflow -= widths[index];
                    lines[index] = None;
                    widths[index] = 0.0;
                }
            }
        }

        let total = bounds_width.min(widths.iter().sum::<f32>() + gaps);
        let mut x = match alignment {
            PostRowTextLineViewAlignment::Center => PostRowTextMetrics::pixel_round((bounds_width - total) / 2.0, scale),
            PostRowTextLineViewAlignment::Leading => 0.0,
        };
        let mut frames = Vec::with_capacity(lines.len());
        let mut shaped = Vec::with_capacity(lines.len());
        for (line, width) in lines.into_iter().zip(widths) {
            if let Some(line) = line {
                shaped.push((line, x));
            }
            frames.push(Bounds::new(point(px(x), px(0.0)), size(px(width), bounds.size.height)));
            x += width + spacing;
        }
        (shaped, frames)
    }

    fn shape(text: &str, segment: &PostRowTextLineViewSegment, window: &Window) -> ShapedLine {
        let run = Self::run(segment, text.len());
        window.text_system().shape_line(SharedString::from(text.to_owned()), px(segment.font.size), &[run], None)
    }

    fn truncated(
        segment: &PostRowTextLineViewSegment,
        allowed: f32,
        window: &Window,
        cx: &App,
    ) -> ShapedLine {
        let runs = [Self::run(segment, segment.text.len())];
        let mut wrapper = cx.text_system().line_wrapper(Self::gpui_font(&segment.font), px(segment.font.size));
        let text = SharedString::from(segment.text.clone());
        let (text, runs) = wrapper.truncate_line(text, px(allowed), "\u{2026}", &runs, TruncateFrom::End);
        window.text_system().shape_line(text, px(segment.font.size), &runs, None)
    }

    fn run(segment: &PostRowTextLineViewSegment, len: usize) -> TextRun {
        TextRun {
            len,
            font: Self::gpui_font(&segment.font),
            color: segment.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    fn gpui_font(font: &RichFont) -> GpuiFont {
        let face = Font {
            size: font.size,
            weight: font.weight,
            design: font.design,
            italic: font.italic,
            monospaced_digit: false,
        };
        GpuiFont {
            family: face.family(),
            features: face.features(),
            fallbacks: None,
            weight: face.weight,
            style: face.style(),
        }
    }
}

impl Render for PostRowTextLineView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let segments = self.segments.clone();
        let spacing = self.spacing;
        let alignment = self.alignment;
        let shaped = self.shaped.clone();
        let this = cx.weak_entity();
        canvas(
            move |bounds, window, cx| {
                let scale = window.scale_factor();
                if let Some(shaped) = shaped.as_ref().filter(|shaped| {
                    shaped.width == bounds.size.width && shaped.height == bounds.size.height && shaped.scale == scale
                }) {
                    return shaped.lines.clone();
                }
                let (lines, frames) = Self::lay_out_lines(&segments, spacing, alignment, bounds, window, cx);
                let lines: Rc<[(ShapedLine, f32)]> = lines.into();
                let fresh = PostRowTextLineViewShaped {
                    width: bounds.size.width,
                    height: bounds.size.height,
                    scale,
                    lines: lines.clone(),
                };
                let _ = this.update(cx, |this, _| {
                    this.drawn_segment_frames = frames;
                    this.shaped = Some(fresh);
                });
                lines
            },
            move |bounds, lines, window, cx| {
                for (line, x) in lines.iter() {
                    let origin = point(bounds.origin.x + px(*x), bounds.origin.y);
                    let _ = line.paint(origin, bounds.size.height, TextAlign::Left, None, window, cx);
                }
            },
        )
        .size_full()
    }
}

/// main text fonts for a spec
pub enum PostRowChromeFonts {}

impl PostRowChromeFonts {
    /// px plus text_delta, floored like the row text metrics font
    pub fn text(px: f32, weight: FontWeight, spec: &PostRowLayoutSpec) -> RichFont {
        Self::fixed(
            (px + spec.text_delta).max(PostRichTextCalibration::MINIMUM_FONT_SIZE),
            weight,
            spec.is_bold_text_enabled,
        )
    }

    /// fixed size system font, no text delta
    pub fn fixed(size: f32, weight: FontWeight, is_bold_text_enabled: bool) -> RichFont {
        PostRichTextRender::font(size, weight, is_bold_text_enabled)
    }
}

/// template symbol images, the caller tints them with text_color
pub enum PostRowChromeSymbols {}

impl PostRowChromeSymbols {
    pub fn image(name: &str, point_size: f32, _weight: FontWeight) -> Option<Svg> {
        IconUi::sized(name, px(point_size), white(), name.ends_with(".fill"))
    }
}

/// spring and ease out animation settings for a property of a part
pub enum PostRowChromeSpring {}

impl PostRowChromeSpring {
    pub fn animate(animated: &mut Animated<f32>, to: f32, response: f64, damping_fraction: f64, cx: &App) {
        animated.set_in(to, spring(response, damping_fraction), RiseClock::monotonic(), cx);
    }

    pub fn animate_ease_out(animated: &mut Animated<f32>, to: f32, duration: f64, cx: &App) {
        let spec = MotionSpec::curve(CubicBezier::EASE_OUT, Duration::from_secs_f64(duration));
        animated.set_in(to, spec, RiseClock::monotonic(), cx);
    }

    /// drops a running animation and pins the value
    pub fn reset(animated: &mut Animated<f32>, to: f32) {
        animated.jump(to);
    }
}
