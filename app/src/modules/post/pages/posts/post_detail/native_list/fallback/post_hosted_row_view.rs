use std::cell::Cell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, Context, ElementId, Entity, EntityId, Hsla, IntoElement, ParentElement, Pixels,
    Render, SharedString, Styled, Window, div, px,
};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::rise_app_events::RiseAppEvents;
use rise_i18n::language_store::Locale;
use rise_ui::animated_media_playback_scope::{
    ANIMATED_MEDIA_VIEWPORT_SCOPE_DID_CHANGE, AnimatedMediaViewportScopeProviding, AnimatedMediaViewportScopeViewKey,
};
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::prelude::measure;

use crate::app::router::navigation_store::NavigationStoreSettlingPush;
use crate::app::router::navigation_types::RootRoute;
use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementSurface;
use crate::core::ui::list_engine::rise_manual_list_integration::{
    RiseManualListConformances, RiseManualListInstallCost, RiseManualListPreparedHeightValidating,
    RiseManualListViewportVisibilityReceiving,
};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListItem, RiseManualListReusableView,
};
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::repost::repost_overlay::RepostTransitionNamespace;
use crate::modules::post::engine::rise_post_engine_models::RisePostViewSurface;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_leaf_mount_queue::PostListReusableRowConformances;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListHostedEnvironment, PostListHostedEnvironmentModifiers, PostListReusableRow, PostListReuseIdentifier, PostRowLayout, PostRowMainContext,
    PostRowNativeKind, PostRowPalette, PostRowRoute, PreparedPostRow, PreparedPostRowKey,
};
use crate::modules::post::pages::posts::post_detail::native_list::fallback::post_hosted_row_height_cache::PostHostedRowHeightCache;
use crate::modules::post::pages::posts::post_detail::native_list::fallback::post_hosted_row_measurer::{
    PostHostedRowMeasurePolicy, PostHostedRowMeasurer,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::shorts::components::shorts_zoom_namespace::ShortsZoomNamespaceId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostHostedRowViewRole {
    Live,
    Measurement,
}

/// content the hosted graph draws
type PostHostedRowRoot = Rc<dyn Fn() -> AnyElement>;

/// tells cells whether animated media is visible in the viewport, read through the environment
#[derive(Default)]
struct PostHostedRowViewportScope {
    is_visible: Cell<bool>,
}

impl AnimatedMediaViewportScopeProviding for PostHostedRowViewportScope {
    fn is_animated_media_visible_in_viewport(&self) -> bool {
        self.is_visible.get()
    }
}

/// the one content entity of a row
/// kept across configure, reset and measuring
struct PostHostedRowContent {
    root: Option<PostHostedRowRoot>,
    viewport_scope: Rc<PostHostedRowViewportScope>,
}

impl Render for PostHostedRowContent {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(root) = self.root.clone() else {
            return div().into_any_element();
        };
        let scope: Rc<dyn AnimatedMediaViewportScopeProviding> = self.viewport_scope.clone();
        env_scope::<AnimatedMediaViewportScopeViewKey>(Some(scope)).child(root()).into_any_element()
    }
}

#[derive(Clone, Copy)]
struct PostHostedRowChrome {
    background: Hsla,
    separator: Hsla,
}

/// whole row hosted fallback, a Post content entity drawn at natural height
/// the row never measures on the configure path
/// estimated height is audited at idle, controller commits the fix
pub struct PostHostedRowView {
    role: PostHostedRowViewRole,
    measurer: Weak<PostHostedRowMeasurer>,

    hosted_content: Option<Entity<PostHostedRowContent>>,
    configured_presentation_id: Option<String>,
    configured_key: Option<PreparedPostRowKey>,
    configured_post: Option<Arc<PreparedPostRow>>,
    configured_row_height: Option<f32>,
    is_configured_height_estimated: bool,
    viewport_scope: Rc<PostHostedRowViewportScope>,

    chrome: Option<PostHostedRowChrome>,
    configured_environment: Option<PostHostedEnvironmentIdentity>,
    pinned_content_width: Option<f32>,
    height_generation: u64,
    install_generation: u64,
    validated_height_generation: Option<u64>,
    last_reported_row_height: f32,
    measurement_nonce: u64,

    bounds: Option<Bounds<Pixels>>,
    window_scale: Option<f32>,
    natural_content_height: Option<f32>,
}

impl PostHostedRowView {
    pub fn new(role: PostHostedRowViewRole, cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_viewport_visibility_receiving::<Self>(cx);
        RiseManualListConformances::register_prepared_height_validating::<Self>(cx);
        PostListReusableRowConformances::register::<Self>(cx);
        Self {
            role,
            measurer: Weak::new(),
            hosted_content: None,
            configured_presentation_id: None,
            configured_key: None,
            configured_post: None,
            configured_row_height: None,
            is_configured_height_estimated: false,
            viewport_scope: Rc::new(PostHostedRowViewportScope::default()),
            chrome: None,
            configured_environment: None,
            pinned_content_width: None,
            height_generation: 0,
            install_generation: 0,
            validated_height_generation: None,
            last_reported_row_height: 0.0,
            measurement_nonce: 0,
            bounds: None,
            window_scale: None,
            natural_content_height: None,
        }
    }

    pub fn role(&self) -> PostHostedRowViewRole {
        self.role
    }

    pub fn set_measurer(&mut self, measurer: Weak<PostHostedRowMeasurer>) {
        self.measurer = measurer;
    }

    pub fn hosted_content_view(&self) -> Option<AnyView> {
        self.hosted_content.clone().map(AnyView::from)
    }

    pub fn configured_key(&self) -> Option<&PreparedPostRowKey> {
        self.configured_key.as_ref()
    }

    pub fn configured_post(&self) -> Option<&GetPostFeedResponse> {
        self.configured_post.as_ref().map(|prepared| &prepared.post)
    }

    pub fn configured_row_height(&self) -> Option<f32> {
        self.configured_row_height
    }

    pub fn is_configured_height_estimated(&self) -> bool {
        self.is_configured_height_estimated
    }

    pub fn make_list_item(
        prepared: &Arc<PreparedPostRow>,
        measurer: Option<&Rc<PostHostedRowMeasurer>>,
        context: Rc<dyn Fn(&mut App) -> Rc<PostRowMainContext>>,
        cx: &mut App,
    ) -> RiseManualListItem {
        let current = context(cx);
        let background = current.palette.background100;
        let separator = current.palette.border100;
        let version = PostHostedRowHeightCache::shared().item_version(prepared, &current.palette.theme_id);
        let height = prepared.height();
        let mount_policy = prepared.route.mount_policy();
        let measurer = measurer.map_or_else(Weak::new, Rc::downgrade);
        let prepared = prepared.clone();
        RiseManualListItem::new(
            height,
            PostListReuseIdentifier::HOSTED.to_owned(),
            version,
            mount_policy,
            None,
            None,
            Some(Rc::new(move |_window: &mut Window, cx: &mut App| -> AnyView {
                cx.new(|_| PostHostedRowPlaceholderView { background, separator }).into()
            })),
            Some(RiseManualListInstallCost::HostedFallback),
            Rc::new(move |reusable_view: Option<AnyView>, _window: &mut Window, cx: &mut App| -> AnyView {
                let row = reusable_view
                    .and_then(|view| view.downcast::<PostHostedRowView>().ok())
                    .unwrap_or_else(|| cx.new(|cx| PostHostedRowView::new(PostHostedRowViewRole::Live, cx)));
                let current = context(cx);
                let measurer = measurer.clone();
                row.update(cx, |row, cx| {
                    row.set_measurer(measurer);
                    row.configure(&prepared, &current, cx);
                });
                row.into()
            }),
        )
    }

    pub fn can_audit_live(&self, key: &PreparedPostRowKey) -> bool {
        self.role == PostHostedRowViewRole::Live
            && self.configured_key.as_ref() == Some(key)
            && self.hosted_content.is_some()
            && self.bounds.is_some_and(|bounds| {
                let width = f32::from(bounds.size.width);
                width > 1.0 && (width - key.spec.width).abs() <= 0.5
            })
    }

    /// reads the natural height of the mounted graph,
    /// stores it in the height cache and tells the measurer
    /// returns the row height
    pub fn audit_natural_row_height(&mut self, cx: &mut App) -> Option<f32> {
        if self.role != PostHostedRowViewRole::Live || self.hosted_content.is_none() {
            return None;
        }
        let bounds = self.bounds?;
        if f32::from(bounds.size.width) <= 1.0 {
            return None;
        }
        let key = self.configured_key.clone()?;
        let prepared = self.configured_post.clone()?;
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "post.fallback.live_audit",
            HashMap::from([("surface".to_owned(), "post_list".to_owned())]),
        );
        let natural = self.natural_content_height;
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            1.0,
            HashMap::from([("result".to_owned(), if natural.is_none() { "none" } else { "ok" }.to_owned())]),
        );
        let natural = natural?;
        self.validated_height_generation = Some(self.height_generation);
        self.pinned_content_width = Some(f32::from(bounds.size.width));
        let row_height = Self::row_height_for_content(natural);
        PostHostedRowHeightCache::shared().insert(row_height, &key, &prepared.post);
        if let Some(measurer) = self.measurer.upgrade() {
            measurer.live_row_did_measure(&key, row_height, self.configured_row_height.unwrap_or(row_height), cx);
        }
        Some(row_height)
    }

    pub fn configure_for_measurement(
        &mut self,
        post: &GetPostFeedResponse,
        environment: &PostListHostedEnvironment,
        cx: &mut Context<Self>,
    ) {
        if self.role != PostHostedRowViewRole::Measurement {
            return;
        }
        self.measurement_nonce = self.measurement_nonce.wrapping_add(1);
        let snapshot: Rc<GetPostFeedResponse> = Rc::new(post.clone());
        let environment = environment.clone();
        let identity: SharedString = format!("{}#{}", post.presentation_id(), self.measurement_nonce).into();
        let root: PostHostedRowRoot = Rc::new(move || {
            let post = Post::new(ElementId::Name(identity.clone()), snapshot.clone())
                .feed_row(true)
                .view_surface(RisePostViewSurface::PostDetail)
                .records_visibility(false);
            post.applying_post_list_hosted_environment(&environment)
        });
        self.pinned_content_width = None;
        self.install(Some(root), cx);
    }

    pub fn prepare_measurement_frame(&mut self, width: f32) {
        self.bounds = Some(Bounds {
            origin: gpui::point(px(0.0), px(0.0)),
            size: gpui::size(px(width), px(PostHostedRowMeasurePolicy::SCRATCH_INITIAL_HEIGHT)),
        });
    }

    /// drops the measured graph (players
    /// images, observers)
    /// the host stays alive
    pub fn release_measurement_content(&mut self, cx: &mut Context<Self>) {
        if self.role != PostHostedRowViewRole::Measurement || self.hosted_content.is_none() {
            return;
        }
        self.install(None, cx);
    }

    fn bounds_did_change(&mut self, bounds: Bounds<Pixels>, scale: f32, cx: &mut App) {
        self.bounds = Some(bounds);
        self.window_scale = Some(scale);
        let width = f32::from(bounds.size.width);
        if self.role == PostHostedRowViewRole::Live
            && width > 1.0
            && self.pinned_content_width.is_some_and(|pinned| (pinned - width).abs() > 0.5)
        {
            self.pinned_content_width = None;
            self.height_generation = self.height_generation.wrapping_add(1);
            self.validated_height_generation = None;
            if let Some(measurer) = self.measurer.upgrade() {
                measurer.set_needs_pump(cx);
            }
        }
    }

    fn live_root(prepared: &PreparedPostRow, context: &PostRowMainContext) -> PostHostedRowRoot {
        let post_id = prepared.id.clone();
        let snapshot: Rc<GetPostFeedResponse> = Rc::new(prepared.post.clone());
        let navigation_store = context.hosted.navigation_store.clone();
        let router = Rc::downgrade(&context.router);
        let environment = context.hosted.clone();
        let identity: SharedString = prepared.key.presentation_id.clone().into();
        Rc::new(move || {
            let latest_post = {
                let router = router.clone();
                let post_id = post_id.clone();
                let snapshot = snapshot.clone();
                Rc::new(move |cx: &mut App| -> GetPostFeedResponse {
                    router
                        .upgrade()
                        .and_then(|router| router.post(&post_id, cx))
                        .unwrap_or_else(|| (*snapshot).clone())
                })
            };
            let comments_router = router.clone();
            let comments_post_id = post_id.clone();
            let avatar_post = latest_post.clone();
            let avatar_navigation_store = navigation_store.clone();
            let like_post = latest_post.clone();
            let favorite_post = latest_post;
            let post = Post::new(ElementId::Name(identity.clone()), snapshot.clone())
                .feed_row(true)
                .view_surface(RisePostViewSurface::PostDetail)
                .records_visibility(false)
                .on_open_comments(move |_window, cx| {
                    if let Some(router) = comments_router.upgrade() {
                        router.open_comments(&comments_post_id, cx);
                    }
                })
                .on_avatar_tap(move |_window, cx| {
                    let post = avatar_post(cx);
                    let route = RootRoute::UserPage { user_id: post.author.id.clone(), tag: Some(post.author.tag.clone()) };
                    avatar_navigation_store.update(cx, |navigation_store, cx| {
                        navigation_store.push(route, NavigationStoreSettlingPush::Drop, cx);
                    });
                })
                .on_like(move |_window, cx| {
                    let post = like_post(cx);
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&post, cx));
                })
                .on_fav(move |_window, cx| {
                    let post = favorite_post(cx);
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(&post, cx));
                });
            post.applying_post_list_hosted_environment(&environment)
        })
    }

    fn install(&mut self, root: Option<PostHostedRowRoot>, cx: &mut Context<Self>) {
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "post.hosted.install",
            HashMap::from([
                ("surface".to_owned(), "post_list".to_owned()),
                (
                    "reusing_content_view".to_owned(),
                    if self.hosted_content.is_none() { "0" } else { "1" }.to_owned(),
                ),
            ]),
        );
        self.natural_content_height = None;
        self.install_generation = self.install_generation.wrapping_add(1);
        match &self.hosted_content {
            Some(hosted_content) => hosted_content.update(cx, |content, cx| {
                content.root = root;
                cx.notify();
            }),
            None => {
                let viewport_scope = self.viewport_scope.clone();
                self.hosted_content = Some(cx.new(|_| PostHostedRowContent { root, viewport_scope }));
            }
        }
        cx.notify();
        ChatPerformanceTrace::shared().end_operation(trace_token, 0.75, HashMap::new());
    }

    fn apply_chrome(&mut self, palette: &PostRowPalette) {
        self.chrome = Some(PostHostedRowChrome { background: palette.background100, separator: palette.border100 });
    }

    /// engine item height for a natural content height
    pub(crate) fn row_height_for_content(natural: f32) -> f32 {
        natural + PostRowLayout::SEPARATOR_GAP + PostRowLayout::SEPARATOR_THICKNESS
    }

    pub fn route_name(route: &PostRowRoute) -> &'static str {
        match route {
            PostRowRoute::HostedFallback(reason) => reason.raw_value(),
            PostRowRoute::Native(kind) => match kind {
                PostRowNativeKind::Plain => "plain",
                PostRowNativeKind::PhotoGallery => "photo_gallery",
                PostRowNativeKind::VideoGallery => "video_gallery",
                PostRowNativeKind::RiffCard => "riff_card",
                PostRowNativeKind::MusicStrip => "music_strip",
            },
        }
    }
}

impl PostListReusableRow for PostHostedRowView {
    fn configured_presentation_id(&self) -> Option<&str> {
        self.configured_presentation_id.as_deref()
    }

    fn configure(&mut self, prepared: &Arc<PreparedPostRow>, context: &PostRowMainContext, cx: &mut Context<Self>) {
        if self.role != PostHostedRowViewRole::Live {
            return;
        }
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "post.hosted.configure",
            HashMap::from([
                ("surface".to_owned(), "post_list".to_owned()),
                ("route".to_owned(), Self::route_name(&prepared.route).to_owned()),
                ("estimated".to_owned(), if prepared.is_height_estimated { "1" } else { "0" }.to_owned()),
            ]),
        );

        self.apply_chrome(&context.palette);
        PostHostedRowHeightCache::shared().register_hosted_content(&prepared.post);

        let row_height = prepared.height();
        let presentation_id = prepared.key.presentation_id.clone();
        let environment = PostHostedEnvironmentIdentity::new(context);
        let is_new_subject = self.configured_presentation_id.as_deref() != Some(presentation_id.as_str());
        let needs_install = self.hosted_content.is_none()
            || is_new_subject
            || self.configured_environment.as_ref() != Some(&environment)
            || self.configured_post().is_none_or(|post| *post != prepared.post);

        if self.configured_key.as_ref() != Some(&prepared.key)
            || self.configured_row_height != Some(row_height)
            || self.is_configured_height_estimated != prepared.is_height_estimated
            || needs_install
        {
            self.height_generation = self.height_generation.wrapping_add(1);
            self.validated_height_generation = None;
            self.last_reported_row_height = row_height;
        }
        self.configured_presentation_id = Some(presentation_id);
        self.configured_key = Some(prepared.key.clone());
        self.configured_post = Some(prepared.clone());
        self.configured_row_height = Some(row_height);
        self.is_configured_height_estimated = prepared.is_height_estimated;
        self.configured_environment = Some(environment);

        if needs_install {
            self.install(Some(Self::live_root(prepared, context)), cx);
        }

        self.pinned_content_width = Some(context.spec.width);
        cx.notify();

        if prepared.is_height_estimated
            && let Some(measurer) = self.measurer.upgrade()
        {
            measurer.note_mounted_estimated_row(prepared, cx);
        }
        ChatPerformanceTrace::shared().end_operation(trace_token, 1.0, HashMap::new());
    }

    fn set_hosted_leaves_active(&mut self, _is_active: bool, _cx: &mut Context<Self>) {}
}

impl RiseManualListReusableView for PostHostedRowView {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>) {
        self.configured_presentation_id = None;
        self.configured_key = None;
        self.configured_post = None;
        self.configured_row_height = None;
        self.is_configured_height_estimated = false;
        self.configured_environment = None;
        self.pinned_content_width = None;
        self.height_generation = self.height_generation.wrapping_add(1);
        self.validated_height_generation = None;
        self.last_reported_row_height = 0.0;
        self.bounds = None;
        self.set_animated_media_visible_in_viewport(false, cx);
        self.install(None, cx);
    }
}

impl RiseManualListViewportVisibilityReceiving for PostHostedRowView {
    fn set_animated_media_visible_in_viewport(&mut self, is_visible: bool, _cx: &mut App) {
        let resolved = self.role == PostHostedRowViewRole::Live && is_visible;
        if self.viewport_scope.is_visible.get() == resolved {
            return;
        }
        self.viewport_scope.is_visible.set(resolved);
        RiseAppEvents::default_center().post(ANIMATED_MEDIA_VIEWPORT_SCOPE_DID_CHANGE);
    }
}

impl AnimatedMediaViewportScopeProviding for PostHostedRowView {
    fn is_animated_media_visible_in_viewport(&self) -> bool {
        self.viewport_scope.is_visible.get()
    }
}

impl RiseManualListPreparedHeightValidating for PostHostedRowView {
    fn needs_prepared_viewport_height_validation(&self, _cx: &App) -> bool {
        self.role == PostHostedRowViewRole::Live
            && self.is_configured_height_estimated
            && self.hosted_content.is_some()
            && self.validated_height_generation != Some(self.height_generation)
    }

    fn prepared_viewport_natural_height_mismatch(&mut self, cx: &mut App) -> Option<Pixels> {
        if !self.needs_prepared_viewport_height_validation(cx) {
            return None;
        }
        let committed = self.configured_row_height?;
        let row_height = self.audit_natural_row_height(cx)?;
        let scale = self
            .window_scale
            .or_else(|| self.configured_key.as_ref().map(|key| key.spec.display_scale))
            .unwrap_or(1.0);
        let tolerance = PostHostedRowMeasurePolicy::height_tolerance(scale);
        if (row_height - committed).abs() <= tolerance
            || (row_height - self.last_reported_row_height).abs() <= tolerance
        {
            return None;
        }
        self.last_reported_row_height = row_height;
        Some(px(row_height))
    }
}

impl Render for PostHostedRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let install_generation = self.install_generation;
        let mut row = div().relative().size_full().overflow_hidden();
        if let Some(chrome) = self.chrome {
            row = row.bg(chrome.background);
        }
        if let Some(hosted_content) = self.hosted_content.clone() {
            let content_this = this.clone();
            let host = div().absolute().top_0().left_0().w_full().child(hosted_content).child(measure(
                ("post-hosted-row-content", install_generation),
                move |bounds, window, cx| {
                    let scale = window.scale_factor();
                    let natural = PostHostedRowMeasurer::pixel_ceiled_height(f32::from(bounds.size.height), scale);
                    let _ = content_this.update(cx, |view, _| view.natural_content_height = natural);
                },
            ));
            row = row.child(host);
        }
        if self.role == PostHostedRowViewRole::Live
            && let Some(chrome) = self.chrome
        {
            row = row.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .w_full()
                    .h(px(PostRowLayout::SEPARATOR_THICKNESS))
                    .bg(chrome.separator),
            );
        }
        row.child(measure(("post-hosted-row-bounds", install_generation), move |bounds, window, cx| {
            let scale = window.scale_factor();
            let _ = this.update(cx, |view, cx| view.bounds_did_change(bounds, scale, cx));
        }))
    }
}

/// environment facts that must reinstall the hosted graph when they change
/// open_url cant be compared so it just follows the navigation store
#[derive(PartialEq)]
struct PostHostedEnvironmentIdentity {
    router: *const (),
    navigation_store: EntityId,
    media_viewer_presentation: EntityId,
    theme_store: EntityId,
    repost_transition_namespace: Option<RepostTransitionNamespace>,
    shorts_zoom_namespace: Option<ShortsZoomNamespaceId>,
    locale: Locale,
    calendar: Calendar,
    time_zone: String,
    shared_element_surface: Option<SharedElementSurface>,
}

impl PostHostedEnvironmentIdentity {
    fn new(context: &PostRowMainContext) -> Self {
        Self {
            router: Rc::as_ptr(&context.router) as *const (),
            navigation_store: context.hosted.navigation_store.entity_id(),
            media_viewer_presentation: context.hosted.media_viewer_presentation.entity_id(),
            theme_store: context.hosted.theme_store.entity_id(),
            repost_transition_namespace: context.hosted.repost_transition_namespace.clone(),
            shorts_zoom_namespace: context.hosted.shorts_zoom_namespace,
            locale: context.hosted.locale.clone(),
            calendar: context.hosted.calendar.clone(),
            time_zone: format!("{:?}", context.hosted.time_zone),
            shared_element_surface: context.hosted.shared_element_surface.clone(),
        }
    }
}

struct PostHostedRowPlaceholderView {
    background: Hsla,
    separator: Hsla,
}

impl Render for PostHostedRowPlaceholderView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().relative().size_full().bg(self.background).child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .w_full()
                .h(px(PostRowLayout::SEPARATOR_THICKNESS))
                .bg(self.separator),
        )
    }
}
