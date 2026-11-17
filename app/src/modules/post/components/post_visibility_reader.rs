use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, App, AppContext, Bounds, Context, ElementId, Entity, IntoElement, Pixels, RenderOnce, Styled,
    Window, canvas,
};

use crate::core::desktop_platform::rise_app_lifecycle::RiseAppLifecycle;
use crate::modules::post::components::post_view_dwell_session::PostViewDwellSession;
use crate::modules::post::engine::rise_post_engine_models::RisePostViewSurface;

/// transparent layer over its parent that counts the post as viewed
/// no hitbox so it takes no clicks
#[derive(IntoElement)]
pub struct PostVisibilityReader {
    id: ElementId,
    pub post_id: String,
    pub surface: RisePostViewSurface,
    pub enabled: bool,
}

impl PostVisibilityReader {
    /// id keeps the session of this slot across frames
    pub fn new(id: impl Into<ElementId>, post_id: impl Into<String>, surface: RisePostViewSurface) -> Self {
        Self { id: id.into(), post_id: post_id.into(), surface, enabled: true }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl RenderOnce for PostVisibilityReader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, post_id, surface, enabled } = self;
        let coordinator = window.use_keyed_state(id, cx, |_, cx| PostVisibilityReaderCoordinator::new(cx));
        if !coordinator.read(cx).is_configured(&post_id, surface, enabled) {
            let handle = coordinator.clone();
            window.defer(cx, move |_, cx| {
                handle.update(cx, |state, cx| state.update(&post_id, surface, enabled, cx));
            });
        }
        let host = coordinator.read(cx).host.clone();
        canvas(
            move |bounds, window, cx| {
                let paint =
                    VisibilityPaint { window: window.window_handle(), frame: bounds, viewport_clip: window.content_mask().bounds };
                host.painted.set(Some(paint));
                if coordinator.read(cx).needs_evaluation(cx) {
                    let evaluated = coordinator.clone();
                    window.defer(cx, move |_, cx| evaluated.update(cx, |state, cx| state.evaluate(cx)));
                }
                let moved_host = host.clone();
                window.on_replayed_move(move |offset, mask_offset, window, cx| {
                    moved_host.painted.set(Some(VisibilityPaint {
                        frame: paint.frame + offset,
                        viewport_clip: paint.viewport_clip + mask_offset,
                        ..paint
                    }));
                    if coordinator.read(cx).needs_evaluation(cx) {
                        let evaluated = coordinator.clone();
                        window.defer(cx, move |_, cx| evaluated.update(cx, |state, cx| state.evaluate(cx)));
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PostVisibilityReaderConfiguration {
    post_id: String,
    surface: RisePostViewSurface,
    enabled: bool,
}

pub struct PostVisibilityReaderCoordinator {
    configuration: Option<PostVisibilityReaderConfiguration>,
    session: Option<Entity<PostViewDwellSession>>,
    host: Rc<PostVisibilityReaderVisibilityView>,
}

impl PostVisibilityReaderCoordinator {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|coordinator: &mut Self, cx: &mut App| coordinator.stop(cx)).detach();
        Self {
            configuration: None,
            session: None,
            host: Rc::new(PostVisibilityReaderVisibilityView { painted: Cell::new(None) }),
        }
    }

    fn is_configured(&self, post_id: &str, surface: RisePostViewSurface, enabled: bool) -> bool {
        self.configuration.as_ref().is_some_and(|configuration| {
            configuration.post_id == post_id && configuration.surface == surface && configuration.enabled == enabled
        })
    }

    fn needs_evaluation(&self, cx: &App) -> bool {
        let (Some(configuration), Some(session)) = (&self.configuration, &self.session) else {
            return false;
        };
        configuration.enabled && session.read(cx).needs_visibility_update(self.host.is_actually_visible(cx))
    }

    pub fn update(&mut self, post_id: &str, surface: RisePostViewSurface, enabled: bool, cx: &mut Context<Self>) {
        if !self.is_configured(post_id, surface, enabled) {
            self.configuration =
                Some(PostVisibilityReaderConfiguration { post_id: post_id.to_owned(), surface, enabled });
            let session = self.dwell_session(post_id, surface, cx);
            session.update(cx, |session, cx| session.reset_scope(post_id, surface, cx));
        }
        self.evaluate(cx);
    }

    fn evaluate(&mut self, cx: &mut App) {
        let qualified = self.host.is_actually_visible(cx);
        self.visibility_changed(qualified, cx);
    }

    fn visibility_changed(&mut self, qualified: bool, cx: &mut App) {
        let Some(session) = &self.session else {
            return;
        };
        if self.configuration.as_ref().is_some_and(|configuration| configuration.enabled) {
            session.update(cx, |session, cx| session.visibility_changed(qualified, cx));
        } else {
            session.update(cx, |session, cx| session.suspend(cx));
        }
    }

    /// reports a skip when due and then the session goes silent
    fn stop(&mut self, cx: &mut App) {
        self.visibility_changed(false, cx);
        if let Some(session) = &self.session {
            session.update(cx, |session, cx| session.stop(cx));
        }
    }

    fn dwell_session(
        &mut self,
        post_id: &str,
        surface: RisePostViewSurface,
        cx: &mut App,
    ) -> Entity<PostViewDwellSession> {
        if let Some(session) = &self.session {
            return session.clone();
        }
        let host = self.host.clone();
        let created = cx.new(|_| PostViewDwellSession::new(post_id, surface, move |cx: &App| host.is_actually_visible(cx)));
        self.session = Some(created.clone());
        created
    }
}

/// what the reader saw when it was painted last
#[derive(Clone, Copy, Debug, PartialEq)]
struct VisibilityPaint {
    window: AnyWindowHandle,
    frame: Bounds<Pixels>,
    viewport_clip: Bounds<Pixels>,
}

/// the readers backing view
/// holds only cells so the session closure can read it while the coordinator or session is
/// being updated
pub struct PostVisibilityReaderVisibilityView {
    painted: Cell<Option<VisibilityPaint>>,
}

impl PostVisibilityReaderVisibilityView {
    /// app is active and this window is the key one
    /// so a post in a window behind another stops counting
    pub fn is_actually_visible(&self, cx: &App) -> bool {
        let Some(paint) = self.painted.get() else {
            return false;
        };
        if !RiseAppLifecycle::is_foreground_active(paint.window, cx) {
            return false;
        }
        Self::qualifies(paint.frame, paint.viewport_clip)
    }

    /// mounted but offscreen rows are already false, the area math is skipped for them
    fn qualifies(content_frame: Bounds<Pixels>, viewport_frame: Bounds<Pixels>) -> bool {
        if f32::from(content_frame.size.width) <= 1.0 || f32::from(content_frame.size.height) <= 1.0 {
            return false;
        }
        if !content_frame.intersects(&viewport_frame) {
            return false;
        }
        let area = |frame: Bounds<Pixels>| f32::from(frame.size.width) * f32::from(frame.size.height);
        let visible_area = area(content_frame.intersect(&viewport_frame));
        if visible_area <= 0.0 {
            return false;
        }
        let qualifying_area = area(content_frame).min(area(viewport_frame));
        qualifying_area > 4.0 && visible_area / qualifying_area >= 0.5
    }
}
