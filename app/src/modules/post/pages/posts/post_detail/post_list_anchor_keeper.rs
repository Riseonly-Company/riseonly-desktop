use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::{AnyWindowHandle, App, ElementId, Empty, Entity, IntoElement, ListScrollEvent, ListState, RenderOnce, Window};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    ProfilePostListWindowPolicy, SharedElementSessionPolicyPhase,
};
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::PostListWindowOwner;
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_capture::PostListSharedElementScrollView;
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_destination::PostListSharedElementDestination;

/// mount it 0x0 next to the list, it draws nothing and takes no input
#[derive(IntoElement)]
pub struct PostListAnchorKeeper {
    destination: Rc<PostListSharedElementDestination>,
    revealed_from_id: Binding<String>,
    lower: usize,
    ids: Rc<Vec<String>>,
    floating_top_inset: f32,
    safe_bottom: f32,
    list_state: Option<ListState>,
}

impl PostListAnchorKeeper {
    pub fn new(
        destination: Rc<PostListSharedElementDestination>,
        revealed_from_id: Binding<String>,
        lower: usize,
        ids: Rc<Vec<String>>,
        floating_top_inset: f32,
        safe_bottom: f32,
    ) -> Self {
        Self { destination, revealed_from_id, lower, ids, floating_top_inset, safe_bottom, list_state: None }
    }

    /// the list this keeper widens
    pub fn list_state(mut self, list_state: ListState) -> Self {
        self.list_state = Some(list_state);
        self
    }

    /// registry key of one window row
    pub fn row_frame_key(session: u64, item_id: &str) -> String {
        format!("se-row|list.{session}|{item_id}")
    }
}

impl RenderOnce for PostListAnchorKeeper {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = ElementId::NamedInteger("post-list-anchor-keeper".into(), self.destination.session);
        let view = window.use_keyed_state(id.clone(), cx, |_, _| PostListAnchorKeeperView::new());
        let view = view.read(cx).clone();
        let navigation_store = env::<NavigationStoreKey>(cx);
        view.update(self, navigation_store, window, cx);
        Empty.on_disappear(id, move |_, _| view.tear_down())
    }
}

pub struct PostListAnchorKeeperView {
    this: Weak<Self>,
    destination: RefCell<Weak<PostListSharedElementDestination>>,
    revealed_from_id: RefCell<Option<Binding<String>>>,
    ids: RefCell<Rc<Vec<String>>>,
    lower: Cell<usize>,
    list: RefCell<Option<ListState>>,
    scroll_view: RefCell<Option<Rc<PostListSharedElementScrollView>>>,
    navigation_store: RefCell<Option<Entity<NavigationStore>>>,
    window: Cell<Option<AnyWindowHandle>>,
    open_settled: Cell<bool>,
    user_scrolled_down: Cell<bool>,
    reveal_turn_pending: Cell<bool>,
    evaluation_token: Cell<u64>,
    estimated_row_height: Cell<f32>,
}

impl PostListAnchorKeeperView {
    fn new() -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            destination: RefCell::new(Weak::new()),
            revealed_from_id: RefCell::new(None),
            ids: RefCell::new(Rc::new(Vec::new())),
            lower: Cell::new(0),
            list: RefCell::new(None),
            scroll_view: RefCell::new(None),
            navigation_store: RefCell::new(None),
            window: Cell::new(None),
            open_settled: Cell::new(false),
            user_scrolled_down: Cell::new(false),
            reveal_turn_pending: Cell::new(false),
            evaluation_token: Cell::new(0),
            estimated_row_height: Cell::new(0.0),
        })
    }

    fn as_owner(&self) -> Weak<dyn PostListWindowOwner> {
        self.this.clone()
    }

    fn update(
        &self,
        keeper: PostListAnchorKeeper,
        navigation_store: Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let PostListAnchorKeeper {
            destination,
            revealed_from_id,
            lower,
            ids,
            floating_top_inset,
            safe_bottom,
            list_state,
        } = keeper;
        let current = self.destination.borrow().upgrade();
        if let Some(current) = current
            && !Rc::ptr_eq(&current, &destination)
        {
            current.detach_window_owner(&self.as_owner());
        }
        *self.destination.borrow_mut() = Rc::downgrade(&destination);
        destination.attach_window_owner(self.as_owner());
        destination.update_layout(lower, floating_top_inset, safe_bottom);
        *self.revealed_from_id.borrow_mut() = Some(revealed_from_id);
        *self.navigation_store.borrow_mut() = navigation_store;
        self.window.set(Some(window.window_handle()));
        if let Some(list_state) = list_state {
            self.bind_scroll_view(list_state, window);
        }
        if let Some(scroll_view) = self.scroll_view.borrow().as_ref() {
            scroll_view.update_scale(window.scale_factor());
            destination.attach_scroll_view(scroll_view.clone());
        }

        let page_changed = lower != self.lower.get() || ids.len() != self.ids.borrow().len();
        *self.ids.borrow_mut() = ids;
        self.lower.set(lower);
        let reveal_committed = self.reveal_turn_pending.replace(false);
        if page_changed || reveal_committed {
            self.set_needs_reveal_evaluation(cx);
        }
    }

    fn tear_down(&self) {
        self.evaluation_token.set(self.evaluation_token.get().wrapping_add(1));
        self.reveal_turn_pending.set(false);
        if let Some(list) = self.list.borrow_mut().take() {
            list.set_scroll_handler(|_, _, _| {});
        }
        let destination = self.destination.replace(Weak::new()).upgrade();
        let scroll_view = self.scroll_view.borrow_mut().take();
        if let Some(destination) = destination {
            destination.detach_window_owner(&self.as_owner());
            if let Some(scroll_view) = scroll_view {
                destination.detach_scroll_view(&scroll_view);
            }
        }
        *self.revealed_from_id.borrow_mut() = None;
        *self.navigation_store.borrow_mut() = None;
        self.window.set(None);
    }

    fn bind_scroll_view(&self, list: ListState, window: &Window) {
        if self.list.borrow().is_some() {
            return;
        }
        let this = self.this.clone();
        list.set_scroll_handler(move |_: &ListScrollEvent, _: &mut Window, cx: &mut App| {
            let this = this.clone();
            cx.defer(move |cx| {
                if let Some(this) = this.upgrade() {
                    this.scroll_view_did_scroll(cx);
                }
            });
        });
        let viewport_list = list.clone();
        *self.scroll_view.borrow_mut() = Some(Rc::new(PostListSharedElementScrollView::new(
            window.window_handle(),
            window.scale_factor(),
            move || viewport_list.viewport_bounds(),
        )));
        *self.list.borrow_mut() = Some(list);
    }

    fn scroll_view_did_scroll(&self, cx: &mut App) {
        let Some(list) = self.list.borrow().clone() else {
            return;
        };
        if !self.user_scrolled_down.get() {
            let distance_from_top = -f32::from(list.scroll_px_offset_for_scrollbar().y);
            if ProfilePostListWindowPolicy::latches_scrolled_down(distance_from_top, true, false) {
                self.user_scrolled_down.set(true);
            }
        }
        self.evaluate_reveal(cx);
    }

    fn evaluate_reveal(&self, cx: &mut App) {
        if self.reveal_turn_pending.get() || self.lower.get() == 0 {
            return;
        }
        let Some(list) = self.list.borrow().clone() else {
            return;
        };
        let Some(destination) = self.destination.borrow().upgrade() else {
            return;
        };
        if !self.open_settled.get()
            && SharedElementTransitionCoordinator::shared(cx).read(cx).phase() == SharedElementSessionPolicyPhase::Idle
        {
            self.open_settled.set(true);
        }
        let should_reveal = ProfilePostListWindowPolicy::should_reveal(
            self.open_settled.get(),
            self.user_scrolled_down.get(),
            self.content_above_viewport(&list, destination.session),
            f32::from(list.viewport_bounds().size.height),
            self.lower.get(),
            SharedElementMotion::REVEAL_ABOVE_VIEWPORTS,
        );
        if should_reveal && !self.is_in_navigation_transition(cx) {
            self.reveal_chunk(cx);
        }
    }

    fn reveal_chunk(&self, cx: &mut App) {
        let Some(revealed_from_id) = self.revealed_from_id.borrow().clone() else {
            return;
        };
        let Some(window) = self.window.get() else {
            return;
        };
        let ids = self.ids.borrow().clone();
        let Some(next) =
            ProfilePostListWindowPolicy::next_revealed_from_id(&ids, self.lower.get(), SharedElementMotion::REVEAL_CHUNK)
        else {
            return;
        };
        self.reveal_turn_pending.set(true);
        let updated = window.update(cx, |_, window, cx| revealed_from_id.set(next, window, cx));
        if updated.is_err() {
            self.reveal_turn_pending.set(false);
        }
    }

    /// content offset y plus adjusted top inset
    fn content_above_viewport(&self, list: &ListState, session: u64) -> f32 {
        let measured = -f32::from(list.scroll_px_offset_for_scrollbar().y);
        self.measure_mounted_rows(list, session);
        let rows_above = list.logical_scroll_top().item_ix as f32;
        measured.max(rows_above * self.estimated_row_height.get())
    }

    fn measure_mounted_rows(&self, list: &ListState, session: u64) {
        let ids = self.ids.borrow().clone();
        let start = (self.lower.get() + list.logical_scroll_top().item_ix).min(ids.len());
        let viewport = list.viewport_bounds();
        let limit = f32::from(viewport.bottom()) + f32::from(viewport.size.height);
        let mut total = 0.0;
        let mut count = 0_u32;
        for item_id in &ids[start..] {
            let key = PostListAnchorKeeper::row_frame_key(session, item_id);
            let Some(frame) = MediaCellScreenFrameRegistry::attached_frame(&key) else {
                if count > 0 {
                    break;
                }
                continue;
            };
            total += f32::from(frame.size.height);
            count += 1;
            if f32::from(frame.top()) > limit {
                break;
            }
        }
        if count > 0 {
            self.estimated_row_height.set(total / count as f32);
        }
    }

    /// rows inserted during a push or pop slide would lay out under a moving page
    fn is_in_navigation_transition(&self, cx: &App) -> bool {
        self.navigation_store
            .borrow()
            .as_ref()
            .is_some_and(|store| store.read(cx).navigation_window_has_running_transition(cx))
    }
}

impl PostListWindowOwner for PostListAnchorKeeperView {
    fn open_did_settle(&self, cx: &mut App) {
        self.open_settled.set(true);
        self.set_needs_reveal_evaluation(cx);
    }

    fn set_needs_reveal_evaluation(&self, cx: &mut App) {
        let token = self.evaluation_token.get().wrapping_add(1);
        self.evaluation_token.set(token);
        let this = self.this.clone();
        cx.defer(move |cx| {
            if let Some(this) = this.upgrade()
                && this.evaluation_token.get() == token
            {
                this.evaluate_reveal(cx);
            }
        });
    }
}
