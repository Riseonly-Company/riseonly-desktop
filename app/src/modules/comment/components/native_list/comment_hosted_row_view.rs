use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext, AvailableSpace, Background, Bounds, Context, Entity, Hsla, IntoElement, ParentElement,
    PathBuilder, Pixels, Point, Render, Size, StyleRefinement, Styled, Subscription, Window, canvas, div, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::ThemeStore;
use rise_ui::frame_request::is_painted_on_screen;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment_values::{ColorScheme, color_scheme};
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::measure;

use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListConformances;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListReusableView;
use crate::modules::comment::components::native_list::comment_list_models::{CommentListRow, CommentListRowKind};
use crate::modules::comment::components::native_list::comment_row_heights::CommentRowHeightKey;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::comment::stores::comment_actions::comment_types::CommentPhotoUpload;
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::comment::stores::comment_services::comment_services::CommentServicesStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

/// one comment list row, pinned to the top of its cell at the height the list was told
/// never measures on configure, exact height comes with the row
/// estimates are audited while the list is idle
pub struct CommentHostedRowView {
    row_id: Option<String>,
    height_key: Option<CommentRowHeightKey>,
    is_height_estimated: bool,

    hosted_content_view: Option<AnyView>,
    content_version: Option<i64>,
    committed_height: f32,
    pinned_height: Option<f32>,
    is_audited: bool,
    bounds: Option<Bounds<Pixels>>,
    wash: Entity<CommentFocusWashView>,
    notified: bool,
}

impl CommentHostedRowView {
    pub const REUSE_IDENTIFIER: &'static str = "comment.hosted";

    pub fn new(cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_moved_replay_row::<Self>(cx);
        Self {
            row_id: None,
            height_key: None,
            is_height_estimated: false,
            hosted_content_view: None,
            content_version: None,
            committed_height: 0.0,
            pinned_height: None,
            is_audited: false,
            bounds: None,
            wash: cx.new(CommentFocusWashView::new),
            notified: false,
        }
    }

    pub fn row_id(&self) -> Option<&str> {
        self.row_id.as_deref()
    }

    pub fn height_key(&self) -> Option<&CommentRowHeightKey> {
        self.height_key.as_ref()
    }

    pub fn is_height_estimated(&self) -> bool {
        self.is_height_estimated
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        row: &CommentListRow,
        render_version: i64,
        height_key: CommentRowHeightKey,
        height: f32,
        is_height_estimated: bool,
        content: &Rc<dyn Fn(&CommentListRow, &mut Window, &mut App) -> AnyView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_new_subject = self.row_id.as_deref() != Some(row.id.as_str());
        if is_new_subject || self.content_version != Some(render_version) || self.hosted_content_view.is_none() {
            let root = content(row, window, cx);
            self.install(Some(root), cx);
        }
        let comment_id = match &row.kind {
            CommentListRowKind::Comment(comment, _) => Some(comment.id.clone()),
            CommentListRowKind::Accessory(_) => None,
        };
        self.wash.update(cx, |wash, cx| wash.show(comment_id, cx));
        if self.height_key.as_ref() != Some(&height_key) || self.committed_height != height {
            self.is_audited = false;
        }
        let draws_differently = self.row_id.as_deref() != Some(row.id.as_str()) || self.pinned_height != Some(height);
        self.row_id = Some(row.id.clone());
        self.content_version = Some(render_version);
        self.height_key = Some(height_key);
        self.is_height_estimated = is_height_estimated;
        self.committed_height = height;
        self.pinned_height = Some(height);
        if draws_differently {
            self.notified = true;
            cx.notify();
        }
    }

    /// one sizing pass over the realised content, ceiled to pixels
    pub fn natural_height(&self, window: &mut Window, cx: &mut App) -> Option<f32> {
        let hosted_content_view = self.hosted_content_view.as_ref()?;
        let width = self.bounds.map(|bounds| f32::from(bounds.size.width))?;
        if width <= 1.0 {
            return None;
        }
        let mut element = hosted_content_view.clone().into_any_element();
        let fitted = f32::from(
            element
                .layout_as_root(
                    Size { width: AvailableSpace::Definite(px(width)), height: AvailableSpace::MinContent },
                    window,
                    cx,
                )
                .height,
        );
        if !fitted.is_finite() || fitted <= 0.0 || fitted >= 100_000.0 {
            return None;
        }
        let scale = window.scale_factor();
        Some((fitted * scale).ceil() / scale)
    }

    /// for the scratch host, measures a row that was never on screen
    pub fn measure(&mut self, width: f32, window: &mut Window, cx: &mut Context<Self>) -> Option<f32> {
        let height = self.bounds.map(|bounds| bounds.size.height).unwrap_or(px(1.0)).max(px(1.0));
        self.bounds = Some(Bounds { origin: gpui::point(px(0.0), px(0.0)), size: Size { width: px(width), height } });
        self.pinned_height = None;
        self.natural_height(window, cx)
    }

    fn install(&mut self, root: Option<AnyView>, cx: &mut Context<Self>) {
        self.hosted_content_view = root;
        self.notified = true;
        cx.notify();
    }
}

impl RiseManualListReusableView for CommentHostedRowView {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>) {
        self.row_id = None;
        self.height_key = None;
        self.content_version = None;
        self.is_height_estimated = false;
        self.committed_height = 0.0;
        self.pinned_height = None;
        self.is_audited = false;
        self.bounds = None;
        self.wash.update(cx, |wash, cx| wash.show(None, cx));
        self.install(None, cx);
    }
}

impl CommentHostedRowView {
    pub fn needs_prepared_viewport_height_validation(&self) -> bool {
        self.is_height_estimated && !self.is_audited && self.hosted_content_view.is_some() && self.bounds.is_some()
    }

    pub fn prepared_viewport_natural_height_mismatch(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<f32> {
        if !self.needs_prepared_viewport_height_validation() {
            return None;
        }
        let natural = self.natural_height(window, cx)?;
        self.is_audited = true;
        let scale = window.scale_factor();
        if (natural - self.committed_height).abs() <= 0.5_f32.max(1.0 / scale.max(1.0)) {
            return None;
        }
        Some(natural)
    }
}

impl Render for CommentHostedRowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.notified {
            tracing::warn!(
                target: "riseonly::X1656",
                "hosted row render without notify row={:?} bounds={:?}",
                self.row_id,
                self.bounds.map(|bounds| bounds.origin)
            );
        }
        self.notified = false;
        let this = cx.weak_entity();
        let wash = AnyView::from(self.wash.clone()).cached(StyleRefinement::default().size_full());
        let wash = if self.wash.read(cx).is_quiet() { wash.replay_when_moved() } else { wash };
        let mut row = div().relative().size_full().overflow_hidden().child(div().absolute().inset_0().child(wash));
        if let Some(hosted_content_view) = self.hosted_content_view.clone() {
            let host = div().absolute().top_0().left_0().w_full();
            let host = match self.pinned_height {
                Some(height) => host.h(px(height)),
                None => host.h_full(),
            };
            let content = hosted_content_view.cached(StyleRefinement::default().size_full()).replay_when_moved();
            row = row.child(host.child(content));
        }
        row.child(measure("comment-hosted-row-bounds", move |bounds, _window, cx| {
            this.update(cx, |view, _| view.bounds = Some(bounds)).ok();
        }))
    }
}

/// the store facts a comment row draws, copied out so render doesnt read the stores
pub struct CommentRowFacts {
    pub uploads: HashMap<String, CommentPhotoUpload>,
    pub expanded: HashSet<String>,
    pub viewer: Option<String>,
    pub acts_as_author: bool,
    pub shows_dislike_counts: bool,
    pub list_owner_post_id: Option<String>,
    _subscriptions: Vec<Subscription>,
}

#[derive(PartialEq)]
struct CommentRowFactsSnapshot {
    uploads: HashMap<String, CommentPhotoUpload>,
    expanded: HashSet<String>,
    viewer: Option<String>,
    acts_as_author: bool,
    shows_dislike_counts: bool,
    list_owner_post_id: Option<String>,
}

impl CommentRowFacts {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let interactions = CommentInteractionsStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let post = PostServicesStore::shared(cx);
        let services = CommentServicesStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&interactions, |facts, _, cx| facts.refresh(cx)),
            cx.observe(&auth, |facts, _, cx| facts.refresh(cx)),
            cx.observe(&post, |facts, _, cx| facts.refresh(cx)),
            cx.observe(&services, |facts, _, cx| facts.refresh(cx)),
        ];
        let snapshot = Self::capture(cx);
        let mut facts = Self {
            uploads: HashMap::new(),
            expanded: HashSet::new(),
            viewer: None,
            acts_as_author: false,
            shows_dislike_counts: true,
            list_owner_post_id: None,
            _subscriptions: subscriptions,
        };
        facts.apply(snapshot);
        facts
    }

    fn capture(cx: &mut App) -> CommentRowFactsSnapshot {
        let interactions = CommentInteractionsStore::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let post = PostServicesStore::shared(cx);
        let services = CommentServicesStore::shared(cx);
        let (viewer, acts_as_author) = {
            let auth = auth.read(cx);
            (auth.effective_user_id_for_wire(), auth.viewer_acts_as_author())
        };
        let shows_dislike_counts = post
            .read(cx)
            .selected_post
            .as_ref()
            .is_none_or(|selected| selected.shows_count(|visibility| visibility.comment_dislikes, viewer.as_deref()));
        let interactions = interactions.read(cx);
        CommentRowFactsSnapshot {
            uploads: interactions.photo_uploads().clone(),
            expanded: interactions.expanded_comment_ids().clone(),
            viewer,
            acts_as_author,
            shows_dislike_counts,
            list_owner_post_id: services.read(cx).list_owner_post_id().map(str::to_owned),
        }
    }

    fn snapshot(&self) -> CommentRowFactsSnapshot {
        CommentRowFactsSnapshot {
            uploads: self.uploads.clone(),
            expanded: self.expanded.clone(),
            viewer: self.viewer.clone(),
            acts_as_author: self.acts_as_author,
            shows_dislike_counts: self.shows_dislike_counts,
            list_owner_post_id: self.list_owner_post_id.clone(),
        }
    }

    fn apply(&mut self, snapshot: CommentRowFactsSnapshot) {
        self.uploads = snapshot.uploads;
        self.expanded = snapshot.expanded;
        self.viewer = snapshot.viewer;
        self.acts_as_author = snapshot.acts_as_author;
        self.shows_dislike_counts = snapshot.shows_dislike_counts;
        self.list_owner_post_id = snapshot.list_owner_post_id;
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let next = Self::capture(cx);
        if next != self.snapshot() {
            self.apply(next);
            cx.notify();
        }
    }
}

/// backdrop of a comment row and the wash of the comment a link opened on, until the store
/// lets it go
/// reads the highlight in render so a change dirties only this view
pub struct CommentFocusWashView {
    comment_id: Option<String>,
    opacity: Animated<f32>,
    wash: Hsla,
    painted_size: Option<Size<Pixels>>,
    tick_queued: bool,
    is_lit: bool,
    _interactions: Subscription,
}

impl CommentFocusWashView {
    fn new(cx: &mut Context<Self>) -> Self {
        let interactions = CommentInteractionsStore::shared(cx);
        let subscription = cx.observe(&interactions, |wash, interactions, cx| {
            let is_lit = Self::is_lit_in(&interactions, wash.comment_id.as_deref(), cx);
            if wash.is_lit != is_lit {
                wash.is_lit = is_lit;
                cx.notify();
            }
        });
        Self {
            comment_id: None,
            opacity: Animated::new(0.0),
            wash: Hsla::default(),
            painted_size: None,
            tick_queued: false,
            is_lit: false,
            _interactions: subscription,
        }
    }

    fn is_lit_in(interactions: &Entity<CommentInteractionsStore>, comment_id: Option<&str>, cx: &App) -> bool {
        comment_id.is_some() && interactions.read(cx).highlighted_comment_id.as_deref() == comment_id
    }

    fn show(&mut self, comment_id: Option<String>, cx: &mut Context<Self>) {
        if self.comment_id != comment_id {
            let interactions = CommentInteractionsStore::shared(cx);
            self.is_lit = Self::is_lit_in(&interactions, comment_id.as_deref(), cx);
            self.comment_id = comment_id;
            self.opacity = Animated::new(0.0);
            cx.notify();
        }
    }

    /// true when no wash is drawn and none is fading, so the view paints a flat fill
    fn is_quiet(&self) -> bool {
        let now = RiseClock::monotonic();
        self.comment_id.is_none()
            || (*self.opacity.target() <= 0.0 && self.opacity.value(now) <= 0.0 && !self.opacity.is_running(now))
    }

    /// swap key of the wash path, the view entity mixed with a salt and never zero
    fn swap_key(view: &Entity<Self>) -> u64 {
        (view.entity_id().as_u64().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xC0_77E7_3A54).max(1)
    }

    fn wash_path(origin: Point<Pixels>, size: Size<Pixels>) -> Option<gpui::Path<Pixels>> {
        let mut builder = PathBuilder::fill();
        builder.move_to(origin);
        builder.line_to(point(origin.x + size.width, origin.y));
        builder.line_to(point(origin.x + size.width, origin.y + size.height));
        builder.line_to(point(origin.x, origin.y + size.height));
        builder.close();
        builder.build().ok()
    }

    fn paint_wash(view: &Entity<Self>, opacity: f32, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let Some(path) = Self::wash_path(bounds.origin, bounds.size) else {
            return;
        };
        let (wash, running, queued) = {
            let state = view.read(cx);
            (state.wash, state.opacity.is_running(RiseClock::monotonic()), state.tick_queued)
        };
        window.paint_swappable_path(path, wash.opacity(opacity), Self::swap_key(view), bounds.origin);
        let schedules = running && !queued && is_painted_on_screen(bounds, window);
        view.update(cx, |state, _| {
            state.painted_size = Some(bounds.size);
            if schedules {
                state.tick_queued = true;
            }
        });
        if schedules {
            let view = view.clone();
            window.on_next_frame(move |window, cx| Self::fade_tick(&view, window, cx));
        }
    }

    /// one frame of the fade, recolours the painted wash by a path swap with no render
    fn fade_tick(view: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let now = RiseClock::monotonic();
        let (opacity, running, wash, size) = {
            let state = view.read(cx);
            (state.opacity.value(now), state.opacity.is_running(now), state.wash, state.painted_size)
        };
        view.update(cx, |state, _| state.tick_queued = false);
        let swapped = size.and_then(|size| Self::wash_path(Point::default(), size)).is_some_and(|path| {
            window.swap_paths(vec![(Self::swap_key(view), path, Background::from(wash.opacity(opacity)))])
        });
        if !swapped {
            view.update(cx, |_, cx| cx.notify());
            return;
        }
        if running {
            view.update(cx, |state, _| state.tick_queued = true);
            let view = view.clone();
            window.on_next_frame(move |window, cx| Self::fade_tick(&view, window, cx));
        }
    }
}

impl Render for CommentFocusWashView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.comment_id.is_none() {
            return div();
        }
        let theme_store = ThemeStore::shared(cx);
        let theme = theme_store.read(cx).current_theme();
        let (backdrop, wash) = (theme.bg._100, theme.text.primary);
        let is_lit = self.is_lit;
        let target = match (is_lit, color_scheme(cx)) {
            (false, _) => 0.0_f32,
            (true, ColorScheme::Dark) => 0.14,
            (true, _) => 0.10,
        };
        let now = RiseClock::monotonic();
        if *self.opacity.target() != target {
            self.opacity.set_in(target, ease_out(0.4), now, cx);
        }
        self.wash = wash;
        let opacity = self.opacity.value(now);
        let row = div().size_full().bg(backdrop);
        if opacity <= 0.0 && !self.opacity.is_running(now) {
            return row;
        }
        let view = cx.entity();
        row.child(
            canvas(|_, _, _| (), move |bounds, _, window, cx| Self::paint_wash(&view, opacity, bounds, window, cx))
                .size_full(),
        )
    }
}
