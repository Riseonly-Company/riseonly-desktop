use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::{AnyWindowHandle, App, Bounds, Pixels};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementKey, SharedElementPart, SharedElementSurface,
    SharedElementVisibilityPolicy, SharedElementVisibilityPolicyCandidate,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementDestinationContainer,
};
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementStackHost;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsets;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::PostListWindowOwner;
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_capture::{
    PostListSharedElementCapture, PostListSharedElementHostView, PostListSharedElementScrollView,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

pub type PostListSharedElementItemsProvider = Rc<dyn Fn(&mut App) -> Vec<GetPostFeedResponse>>;

/// profile post list as a shared element destination, the open target and the most visible post at exit
/// nothing here is observed
/// list owners write layout facts and the coordinator reads them once per transition
pub struct PostListSharedElementDestination {
    pub session: u64,
    this: Weak<Self>,
    origin_scope_key: RefCell<Option<String>>,
    items_provider: RefCell<Option<PostListSharedElementItemsProvider>>,
    window_owner: RefCell<Option<Weak<dyn PostListWindowOwner>>>,
    scroll_view: RefCell<Option<Rc<PostListSharedElementScrollView>>>,
    window_lower: Cell<usize>,
    floating_top_inset: Cell<f32>,
    safe_bottom: Cell<f32>,
}

impl PostListSharedElementDestination {
    pub fn new(session: u64) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            session,
            this: this.clone(),
            origin_scope_key: RefCell::new(None),
            items_provider: RefCell::new(None),
            window_owner: RefCell::new(None),
            scroll_view: RefCell::new(None),
            window_lower: Cell::new(0),
            floating_top_inset: Cell::new(0.0),
            safe_bottom: Cell::new(0.0),
        })
    }

    pub fn is_configured(&self) -> bool {
        self.origin_scope_key.borrow().is_some()
    }

    pub fn configure(&self, origin_scope_key: String, items_provider: PostListSharedElementItemsProvider) {
        *self.origin_scope_key.borrow_mut() = Some(origin_scope_key);
        *self.items_provider.borrow_mut() = Some(items_provider);
    }

    pub fn attach_window_owner(&self, window_owner: Weak<dyn PostListWindowOwner>) {
        *self.window_owner.borrow_mut() = Some(window_owner);
    }

    pub fn detach_window_owner(&self, window_owner: &Weak<dyn PostListWindowOwner>) {
        let mut current = self.window_owner.borrow_mut();
        if current.as_ref().is_some_and(|current| current.ptr_eq(window_owner)) {
            *current = None;
        }
    }

    /// the list scroll surface, gpui list in the legacy body and the engine one in the native list
    pub fn attach_scroll_view(&self, scroll_view: Rc<PostListSharedElementScrollView>) {
        *self.scroll_view.borrow_mut() = Some(scroll_view);
    }

    pub fn detach_scroll_view(&self, scroll_view: &Rc<PostListSharedElementScrollView>) {
        let mut current = self.scroll_view.borrow_mut();
        if current.as_ref().is_some_and(|current| Rc::ptr_eq(current, scroll_view)) {
            *current = None;
        }
    }

    pub fn update_layout(&self, lower: usize, floating_top_inset: f32, safe_bottom: f32) {
        self.window_lower.set(lower);
        self.floating_top_inset.set(floating_top_inset);
        self.safe_bottom.set(safe_bottom);
    }

    pub fn window_top_row_did_appear(&self, cx: &mut App) {
        if let Some(window_owner) = self.resolved_window_owner() {
            window_owner.set_needs_reveal_evaluation(cx);
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

    pub fn surface(&self) -> SharedElementSurface {
        SharedElementSurface::PostList { session: self.session }
    }

    fn resolved_window_owner(&self) -> Option<Rc<dyn PostListWindowOwner>> {
        self.window_owner.borrow().as_ref()?.upgrade()
    }

    fn resolved_host_view(&self, cx: &App) -> Option<PostListSharedElementHostView> {
        self.host_view_controller(cx)?;
        let scroll_view = self.scroll_view.borrow().clone()?;
        Some(PostListSharedElementHostView::new(self.session, &scroll_view))
    }

    fn key(&self, item_id: &str, part: SharedElementPart) -> SharedElementKey {
        SharedElementKey::new(self.surface(), item_id, part)
    }
}

impl Drop for PostListSharedElementDestination {
    fn drop(&mut self) {
        PostListSharedElementCapture::remove_player_layers(self.session);
    }
}

impl SharedElementDestinationContainer for PostListSharedElementDestination {
    fn surface(&self) -> SharedElementSurface {
        PostListSharedElementDestination::surface(self)
    }

    fn origin_surface(&self) -> SharedElementSurface {
        SharedElementSurface::ProfileGrid { scope_key: self.origin_scope_key.borrow().clone().unwrap_or_default() }
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
        let host_view = self.resolved_host_view(cx)?;
        PostListSharedElementCapture::hosted_frame(&self.key(item_id, part).frame_registry_key(), &host_view)
    }

    fn open_landing_mode(
        &self,
        item_id: &str,
        content_aspect: f32,
        clip: Bounds<Pixels>,
        cx: &App,
    ) -> SharedElementContentMode {
        if let Some(host_view) = self.resolved_host_view(cx)
            && PostListSharedElementCapture::hosted_frame(
                &self.key(item_id, SharedElementPart::Media).frame_registry_key(),
                &host_view,
            )
            .is_none()
            && PostListSharedElementCapture::hosted_frame(
                &self.key(item_id, SharedElementPart::Text).frame_registry_key(),
                &host_view,
            )
            .is_some()
        {
            return SharedElementContentMode::AnchoredWidthFit;
        }
        if PostListSharedElementCapture::media_fills(content_aspect, Some(content_aspect), clip) {
            SharedElementContentMode::Fill
        } else {
            SharedElementContentMode::Fit { backdrop: true }
        }
    }

    fn current_capture(&self, cx: &mut App) -> Option<SharedElementCapture> {
        let items_provider = self.items_provider.borrow().clone()?;
        let items = items_provider(cx);
        if items.is_empty() {
            return None;
        }
        let host_view = self.resolved_host_view(cx)?;

        let bottom_edge = BottomChromeInsets::shared(cx).read(cx).top_edge_in_window(&[]);
        let band = SharedElementVisibilityPolicy::band(
            host_view.viewport,
            self.floating_top_inset.get(),
            bottom_edge,
            self.safe_bottom.get(),
        );

        let lower = self.window_lower.get().min(items.len() - 1);
        let candidates: Vec<SharedElementVisibilityPolicyCandidate> = items[lower..]
            .iter()
            .map(|item| {
                let media = PostListSharedElementCapture::hosted_frame(
                    &self.key(&item.id, SharedElementPart::Media).frame_registry_key(),
                    &host_view,
                );
                let text = if media.is_none() {
                    PostListSharedElementCapture::hosted_frame(
                        &self.key(&item.id, SharedElementPart::Text).frame_registry_key(),
                        &host_view,
                    )
                } else {
                    None
                };
                SharedElementVisibilityPolicyCandidate { item_id: item.id.clone(), media_frame: media, text_frame: text }
            })
            .collect();
        let (item_id, part) = SharedElementVisibilityPolicy::pick_current(&candidates, band)?;
        let post = items.iter().find(|item| item.id == item_id)?;

        let source_key = self.key(&item_id, part);
        let clip = PostListSharedElementCapture::hosted_frame(&source_key.frame_registry_key(), &host_view)?;
        PostListSharedElementCapture::capture(post, part, clip, &source_key, &host_view, cx)
    }

    fn can_begin_interactive_return(&self, cx: &App) -> bool {
        self.is_configured() && self.resolved_host_view(cx).is_some()
    }

    fn did_settle_open(&self, _epoch: u64, cx: &mut App) {
        if let Some(window_owner) = self.resolved_window_owner() {
            window_owner.open_did_settle(cx);
        }
    }
}
