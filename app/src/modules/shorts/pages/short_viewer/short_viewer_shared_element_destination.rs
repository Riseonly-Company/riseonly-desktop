use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use gpui::{AnyWindowHandle, App, Bounds, Entity, Pixels, WeakEntity, Window};
use rise_media::video::av_player::AVLayerVideoGravity;

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementKey, SharedElementPart, SharedElementSurface,
};
use crate::core::shared_element_transition::renderer::shared_element_video_frame_grabber::SharedElementPlayerLayerView;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementDestinationContainer,
};
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementStackHost;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_engine_models::RisePostPageScope;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::components::shorts_feed_player::ShortsFeedPlayer;
use crate::modules::shorts::pages::shorts_vertical_pager::ShortsVerticalPagerController;
use crate::modules::shorts::pages::short_viewer::short_viewer_shared_element_capture::{
    ShortViewerSharedElementCapture, ShortViewerSharedElementHostView,
};
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

/// riff viewer as shared element destination, the opened riff on open and the active page on exit
/// nothing here is observed, page and cells write the active riff and slide
pub struct ShortViewerSharedElementDestination {
    pub session: u64,
    this: Weak<Self>,
    origin: RefCell<Option<RisePostPageScope>>,
    opened_post: RefCell<Option<GetPostFeedResponse>>,
    active_post_id: RefCell<Option<String>>,
    slide_by_post_id: RefCell<HashMap<String, i64>>,
    player_layer_by_post_id: RefCell<HashMap<String, SharedElementPlayerLayerView>>,
    host_frame: Cell<Option<(AnyWindowHandle, Bounds<Pixels>, f32)>>,
    pager: RefCell<Option<WeakEntity<ShortsVerticalPagerController>>>,
}

impl ShortViewerSharedElementDestination {
    pub const HEADER_ITEM_ID: &'static str = "viewer.header";

    pub fn new(session: u64) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            session,
            this: this.clone(),
            origin: RefCell::new(None),
            opened_post: RefCell::new(None),
            active_post_id: RefCell::new(None),
            slide_by_post_id: RefCell::new(HashMap::new()),
            player_layer_by_post_id: RefCell::new(HashMap::new()),
            host_frame: Cell::new(None),
            pager: RefCell::new(None),
        })
    }

    pub fn is_configured(&self) -> bool {
        self.origin.borrow().is_some()
    }

    pub fn configure(&self, origin: RisePostPageScope, opened_post: &GetPostFeedResponse) {
        if self.origin.borrow().as_ref() != Some(&origin) {
            *self.origin.borrow_mut() = Some(origin);
        }
        let is_same_post = self.opened_post.borrow().as_ref().is_some_and(|post| post.id == opened_post.id);
        if !is_same_post {
            *self.opened_post.borrow_mut() = Some(opened_post.clone());
        }
        if self.active_post_id.borrow().is_none() {
            *self.active_post_id.borrow_mut() = Some(opened_post.id.clone());
        }
    }

    pub fn update_active_post(&self, id: &str) {
        *self.active_post_id.borrow_mut() = Some(id.to_owned());
    }

    pub fn update_slide(&self, post_id: &str, index: i64) {
        self.slide_by_post_id.borrow_mut().insert(post_id.to_owned(), index);
    }

    /// player layer of the page written by its cell, there is no subview tree to search
    pub fn update_player_layer(&self, post_id: &str, view: Option<SharedElementPlayerLayerView>) {
        let mut layers = self.player_layer_by_post_id.borrow_mut();
        match view {
            Some(view) => {
                layers.insert(post_id.to_owned(), view);
            }
            None => {
                layers.remove(post_id);
            }
        }
    }

    /// viewer bounds written by the page root measurer
    pub fn update_host_frame(&self, frame: Bounds<Pixels>, window: &Window) {
        self.host_frame.set(Some((window.window_handle(), frame, window.scale_factor())));
    }

    pub fn update_pager(&self, pager: &Entity<ShortsVerticalPagerController>) {
        let is_same = self.pager.borrow().as_ref().is_some_and(|known| known.entity_id() == pager.entity_id());
        if !is_same {
            *self.pager.borrow_mut() = Some(pager.downgrade());
        }
    }

    pub fn request_exit(&self, pop: impl FnOnce(&mut App), cx: &mut App) {
        if self.is_configured()
            && let Some(destination) = self.as_container()
        {
            let _ = SharedElementTransitionCoordinator::arm_exit(&destination, false, cx);
        }
        pop(cx);
    }

    pub fn as_container(&self) -> Option<Rc<dyn SharedElementDestinationContainer>> {
        self.this.upgrade().map(|this| this as Rc<dyn SharedElementDestinationContainer>)
    }

    fn resolved_host_view(&self, cx: &App) -> Option<ShortViewerSharedElementHostView> {
        self.host_view_controller(cx)?;
        let (window, frame, scale) = self.host_frame.get()?;
        Some(ShortViewerSharedElementHostView {
            window,
            frame,
            scale,
            player_layers: self.player_layer_by_post_id.borrow().values().cloned().collect(),
            is_in_moving_scroll: self
                .pager
                .borrow()
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .is_some_and(|pager| pager.read(cx).is_moving()),
        })
    }

    fn key(&self, item_id: &str, part: SharedElementPart) -> SharedElementKey {
        SharedElementKey::new(self.surface(), item_id, part)
    }

    fn post_with_id(&self, item_id: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
        let profile_scope = self.origin.borrow().clone();
        if let Some(RisePostPageScope::Profile { user_id, tag, kind }) = profile_scope {
            let page = PostServicesStore::shared(cx)
                .update(cx, |store, cx| store.profile_page_with_tag(&user_id, &tag, kind, cx));
            if let Some(item) = page.items.into_iter().find(|item| item.id == item_id) {
                return Some(item);
            }
        }
        let opened_post = self.opened_post.borrow().clone().filter(|post| post.id == item_id)?;
        let short = ShortsServicesStore::shared(cx).update(cx, |store, cx| store.resolved_short(item_id, cx));
        if short.is_some() {
            return short;
        }
        let post = PostServicesStore::shared(cx).update(cx, |store, cx| store.resolved_post(item_id, cx));
        Some(post.unwrap_or(opened_post))
    }

    fn post_with_id_read_only(&self, item_id: &str, cx: &App) -> Option<GetPostFeedResponse> {
        let presentation = rise_ui::shared::try_read::<RisePostContentPresentation>(cx);
        if let Some(RisePostPageScope::Profile { user_id, tag, kind }) = self.origin.borrow().as_ref()
            && let Some(presentation) = presentation
            && let Some(item) = presentation
                .profile_page_with_tag(user_id, tag, *kind, cx)
                .items
                .iter()
                .find(|item| item.id == item_id)
                .cloned()
        {
            return Some(item);
        }
        let opened_post = self.opened_post.borrow().clone().filter(|post| post.id == item_id)?;
        Some(presentation.and_then(|presentation| presentation.resolved_post(item_id).cloned()).unwrap_or(opened_post))
    }
}

impl SharedElementDestinationContainer for ShortViewerSharedElementDestination {
    fn surface(&self) -> SharedElementSurface {
        SharedElementSurface::ShortViewer { session: self.session }
    }

    fn origin_surface(&self) -> SharedElementSurface {
        SharedElementSurface::ProfileGrid {
            scope_key: self.origin.borrow().as_ref().map(RisePostPageScope::key).unwrap_or_default(),
        }
    }

    fn host_view_controller(&self, cx: &App) -> Option<SharedElementStackHost> {
        let destination = self.as_container()?;
        rise_ui::shared::try_read::<SharedElementTransitionCoordinator>(cx)?
            .registered_host_of_destination(&destination)
    }

    fn host_view(&self, cx: &App) -> Option<AnyWindowHandle> {
        self.resolved_host_view(cx).map(|host_view| host_view.window)
    }

    fn open_target(&self, item_id: &str, part: SharedElementPart, cx: &App) -> Option<Bounds<Pixels>> {
        if part != SharedElementPart::Media {
            return None;
        }
        let host_view = self.resolved_host_view(cx)?;
        ShortViewerSharedElementCapture::page_frame(
            &self.key(item_id, SharedElementPart::Media).frame_registry_key(),
            &host_view,
        )
    }

    fn open_landing_mode(
        &self,
        _item_id: &str,
        content_aspect: f32,
        _clip: Bounds<Pixels>,
        _cx: &App,
    ) -> SharedElementContentMode {
        if ShortsFeedPlayer::gravity(content_aspect) == AVLayerVideoGravity::ResizeAspectFill {
            SharedElementContentMode::Fill
        } else {
            SharedElementContentMode::Fit { backdrop: false }
        }
    }

    fn current_capture(&self, cx: &mut App) -> Option<SharedElementCapture> {
        if !self.is_configured() {
            return None;
        }
        let Some(host_view) = self.resolved_host_view(cx) else {
            tracing::warn!(target: "riseonly::X1625", has_frame = self.host_frame.get().is_some(), "viewer capture refused: no host view");
            return None;
        };
        let post_id = self.active_post_id.borrow().clone()?;
        let Some(post) = self.post_with_id(&post_id, cx) else {
            tracing::warn!(target: "riseonly::X1625", item = %post_id, "viewer capture refused: post not in profile page");
            return None;
        };
        if post.shows_upload_overlay() {
            return None;
        }
        let slide = self.slide_by_post_id.borrow().get(&post_id).copied().unwrap_or(0);
        let source_key = self.key(&post_id, SharedElementPart::Media);
        ShortViewerSharedElementCapture::capture(&post, slide, &source_key, &host_view, cx)
    }

    fn can_begin_interactive_return(&self, cx: &App) -> bool {
        self.is_configured() && self.resolved_host_view(cx).is_some()
    }

    fn did_settle_open(&self, _epoch: u64, _cx: &mut App) {}

    fn lift_keys(&self, item_id: &str, _part: SharedElementPart, _cx: &App) -> HashSet<SharedElementKey> {
        HashSet::from([
            self.key(item_id, SharedElementPart::Media),
            self.key(item_id, SharedElementPart::Text),
            self.key(Self::HEADER_ITEM_ID, SharedElementPart::Text),
        ])
    }

    fn open_landing_is_ready(&self, item_id: &str, cx: &App) -> Option<bool> {
        let post = self.post_with_id_read_only(item_id, cx)?;
        let host_view = self.resolved_host_view(cx);
        Some(ShortViewerSharedElementCapture::landing_is_ready(&post, host_view.as_ref(), cx))
    }
}
