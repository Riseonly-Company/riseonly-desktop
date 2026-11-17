use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{App, Bounds, Entity};
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReaderFrameView};

use crate::core::media_viewer::media_viewer_types::MediaViewerFrameKeys;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementKey, SharedElementPart, SharedElementSurface,
};
use crate::core::shared_element_transition::runtime::shared_element_lift_board::{
    SharedElementLiftBoard, SharedElementLiftObservation, SharedElementLiftSlot,
};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_media_viewer_lift_observer::{
    PostListMediaViewerLiftObservation, PostListMediaViewerLiftObserver,
};
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowAnchorLiftChange, PostRowAnchorRegistering, PostRowAnchorSlideLiftChange,
};

struct PostRowAnchorRegistrationPartRecord {
    key: SharedElementKey,
    view: WindowScreenFrameReaderFrameView,
    slot: Entity<SharedElementLiftSlot>,
    observation: SharedElementLiftObservation,
    applied_hidden: Cell<bool>,
}

struct PostRowAnchorRegistrationSlideRecord {
    index: i64,
    view: WindowScreenFrameReaderFrameView,
    observation: Option<PostListMediaViewerLiftObservation>,
    applied_hidden: Cell<bool>,
}

fn same_view(left: &WindowScreenFrameReaderFrameView, right: &WindowScreenFrameReaderFrameView) -> bool {
    Rc::ptr_eq(&left.owner(), &right.owner())
}

/// registry records and lift observing for one native row view
/// media and text records are view backed entries keyed se|list.session|id|part
pub struct PostRowAnchorRegistration {
    state: Rc<PostRowAnchorRegistrationState>,
}

impl PostRowAnchorRegistration {
    /// surface comes from the shared element destination, media_viewer_lift is the lists one observer
    pub fn new(
        surface: Option<SharedElementSurface>,
        media_viewer_lift: Option<Rc<PostListMediaViewerLiftObserver>>,
    ) -> Self {
        let media_viewer_lift = media_viewer_lift.as_ref().map(Rc::downgrade);
        let state = Rc::new_cyclic(|this| PostRowAnchorRegistrationState {
            this: this.clone(),
            surface,
            media_viewer_lift,
            parts: RefCell::new(HashMap::new()),
            slides: RefCell::new(HashMap::new()),
            on_lift_change: RefCell::new(None),
            on_slide_lift_change: RefCell::new(None),
        });
        Self { state }
    }
}

struct PostRowAnchorRegistrationState {
    this: Weak<Self>,
    surface: Option<SharedElementSurface>,
    media_viewer_lift: Option<Weak<PostListMediaViewerLiftObserver>>,
    parts: RefCell<HashMap<SharedElementPart, PostRowAnchorRegistrationPartRecord>>,
    slides: RefCell<HashMap<String, PostRowAnchorRegistrationSlideRecord>>,
    on_lift_change: RefCell<Option<PostRowAnchorLiftChange>>,
    on_slide_lift_change: RefCell<Option<PostRowAnchorSlideLiftChange>>,
}

impl PostRowAnchorRegistrationState {
    fn media_viewer_lift(&self) -> Option<Rc<PostListMediaViewerLiftObserver>> {
        self.media_viewer_lift.as_ref().and_then(Weak::upgrade)
    }

    fn register_part(
        &self,
        part: SharedElementPart,
        view: &WindowScreenFrameReaderFrameView,
        post_id: &str,
        cx: &mut App,
    ) {
        let Some(surface) = self.surface.clone() else {
            return;
        };
        let key = SharedElementKey::new(surface, post_id, part);
        let is_same = self.parts.borrow().get(&part).map(|existing| existing.key == key && same_view(&existing.view, view));
        match is_same {
            Some(true) => {
                let hidden = self.parts.borrow().get(&part).is_some_and(|existing| existing.slot.read(cx).is_hidden());
                self.apply_part(part, hidden, cx);
                return;
            }
            Some(false) => self.remove_part(part, cx),
            None => {}
        }
        MediaCellScreenFrameRegistry::set(&key.frame_registry_key(), Bounds::default(), Some(view));
        let slot = SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.slot(&key, cx));
        let this = self.this.clone();
        let observed_view = view.clone();
        let observation = slot.update(cx, |slot, cx| {
            slot.add_observer(
                move |is_hidden, cx| {
                    let Some(this) = this.upgrade() else {
                        return;
                    };
                    let is_owned = this.parts.borrow().get(&part).is_some_and(|record| {
                        let is_owner = same_view(&record.view, &observed_view);
                        if is_owner {
                            record.applied_hidden.set(is_hidden);
                        }
                        is_owner
                    });
                    if is_owned {
                        this.notify_part(part, is_hidden, cx);
                    }
                },
                cx,
            )
        });
        let hidden = slot.read(cx).is_hidden();
        self.parts.borrow_mut().insert(
            part,
            PostRowAnchorRegistrationPartRecord {
                key,
                view: view.clone(),
                slot,
                observation,
                applied_hidden: Cell::new(false),
            },
        );
        self.apply_part(part, hidden, cx);
    }

    fn remove_part(&self, part: SharedElementPart, cx: &mut App) {
        let Some(mut record) = self.parts.borrow_mut().remove(&part) else {
            return;
        };
        record.observation.cancel(cx);
        MediaCellScreenFrameRegistry::remove_if_owned_by(&record.key.frame_registry_key(), &record.view);
        if record.applied_hidden.get() {
            self.notify_part(part, false, cx);
        }
    }

    fn remove_slide(&self, registry_key: &str, cx: &mut App) {
        let Some(record) = self.slides.borrow_mut().remove(registry_key) else {
            return;
        };
        if let Some(observation) = record.observation.as_ref() {
            observation.cancel();
        }
        MediaCellScreenFrameRegistry::remove_if_owned_by(registry_key, &record.view);
        if record.applied_hidden.get() {
            self.notify_slide(record.index, false, cx);
        }
    }

    /// applies hidden state to a part, nothing to do when the row already drew this state
    fn apply_part(&self, part: SharedElementPart, hidden: bool, cx: &mut App) {
        let changed = self.parts.borrow().get(&part).is_some_and(|record| record.applied_hidden.replace(hidden) != hidden);
        if changed {
            self.notify_part(part, hidden, cx);
        }
    }

    fn apply_slide(&self, registry_key: &str, hidden: bool, cx: &mut App) {
        let changed_index = self.slides.borrow().get(registry_key).and_then(|record| {
            (record.applied_hidden.replace(hidden) != hidden).then_some(record.index)
        });
        if let Some(index) = changed_index {
            self.notify_slide(index, hidden, cx);
        }
    }

    fn notify_part(&self, part: SharedElementPart, hidden: bool, cx: &mut App) {
        let on_lift_change = self.on_lift_change.borrow().clone();
        if let Some(on_lift_change) = on_lift_change {
            on_lift_change(part, hidden, cx);
        }
    }

    fn notify_slide(&self, index: i64, hidden: bool, cx: &mut App) {
        let on_slide_lift_change = self.on_slide_lift_change.borrow().clone();
        if let Some(on_slide_lift_change) = on_slide_lift_change {
            on_slide_lift_change(index, hidden, cx);
        }
    }
}

impl PostRowAnchorRegistrationState {
    fn register_media(&self, view: &WindowScreenFrameReaderFrameView, post_id: &str, cx: &mut App) {
        self.register_part(SharedElementPart::Media, view, post_id, cx);
    }

    fn register_text(&self, view: &WindowScreenFrameReaderFrameView, post_id: &str, cx: &mut App) {
        self.register_part(SharedElementPart::Text, view, post_id, cx);
    }

    fn register_slide(&self, view: &WindowScreenFrameReaderFrameView, post_id: &str, index: i64, cx: &mut App) {
        let registry_key = MediaViewerFrameKeys::post_image(post_id, index.max(0) as usize);
        let media_viewer_lift = self.media_viewer_lift();
        let is_lifted = |lift: &Option<Rc<PostListMediaViewerLiftObserver>>| {
            lift.as_ref().is_some_and(|lift| lift.is_lifted(&registry_key))
        };
        let is_existing =
            self.slides.borrow().get(&registry_key).is_some_and(|existing| same_view(&existing.view, view));
        if is_existing {
            self.apply_slide(&registry_key, is_lifted(&media_viewer_lift), cx);
            return;
        }
        self.remove_slide(&registry_key, cx);
        MediaCellScreenFrameRegistry::set(&registry_key, Bounds::default(), Some(view));
        let observation = media_viewer_lift.as_ref().map(|lift| {
            let this = self.this.clone();
            let observed_view = view.clone();
            let observed_key = registry_key.clone();
            lift.observe_lift(&registry_key, move |is_hidden, cx| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                let is_owned = this.slides.borrow().get(&observed_key).is_some_and(|record| {
                    let is_owner = same_view(&record.view, &observed_view);
                    if is_owner {
                        record.applied_hidden.set(is_hidden);
                    }
                    is_owner
                });
                if is_owned {
                    this.notify_slide(index, is_hidden, cx);
                }
            })
        });
        let hidden = is_lifted(&media_viewer_lift);
        self.slides.borrow_mut().insert(
            registry_key.clone(),
            PostRowAnchorRegistrationSlideRecord { index, view: view.clone(), observation, applied_hidden: Cell::new(false) },
        );
        self.apply_slide(&registry_key, hidden, cx);
    }

    fn unregister(&self, if_owned_by: &WindowScreenFrameReaderFrameView, cx: &mut App) {
        let parts: Vec<SharedElementPart> = self
            .parts
            .borrow()
            .iter()
            .filter(|(_, record)| same_view(&record.view, if_owned_by))
            .map(|(part, _)| *part)
            .collect();
        for part in parts {
            self.remove_part(part, cx);
        }
        let slides: Vec<String> = self
            .slides
            .borrow()
            .iter()
            .filter(|(_, record)| same_view(&record.view, if_owned_by))
            .map(|(registry_key, _)| registry_key.clone())
            .collect();
        for registry_key in slides {
            self.remove_slide(&registry_key, cx);
        }
    }

    fn unregister_all(&self, cx: &mut App) {
        let parts: Vec<SharedElementPart> = self.parts.borrow().keys().copied().collect();
        for part in parts {
            self.remove_part(part, cx);
        }
        let slides: Vec<String> = self.slides.borrow().keys().cloned().collect();
        for registry_key in slides {
            self.remove_slide(&registry_key, cx);
        }
    }

    fn is_lifted(&self, part: SharedElementPart, cx: &App) -> bool {
        self.parts.borrow().get(&part).is_some_and(|record| record.slot.read(cx).is_hidden())
    }
}

impl PostRowAnchorRegistering for PostRowAnchorRegistration {
    fn surface(&self) -> Option<SharedElementSurface> {
        self.state.surface.clone()
    }

    fn register_media(&self, view: &WindowScreenFrameReaderFrameView, post_id: &str, cx: &mut App) {
        self.state.register_media(view, post_id, cx);
    }

    fn register_text(&self, view: &WindowScreenFrameReaderFrameView, post_id: &str, cx: &mut App) {
        self.state.register_text(view, post_id, cx);
    }

    fn register_slide(&self, view: &WindowScreenFrameReaderFrameView, post_id: &str, index: i64, cx: &mut App) {
        self.state.register_slide(view, post_id, index, cx);
    }

    fn unregister(&self, if_owned_by: &WindowScreenFrameReaderFrameView, cx: &mut App) {
        self.state.unregister(if_owned_by, cx);
    }

    fn unregister_all(&self, cx: &mut App) {
        self.state.unregister_all(cx);
    }

    fn is_lifted(&self, part: SharedElementPart, cx: &App) -> bool {
        self.state.is_lifted(part, cx)
    }

    fn on_lift_change(&self) -> Option<PostRowAnchorLiftChange> {
        self.state.on_lift_change.borrow().clone()
    }

    fn set_on_lift_change(&self, on_lift_change: Option<PostRowAnchorLiftChange>) {
        *self.state.on_lift_change.borrow_mut() = on_lift_change;
    }

    fn on_slide_lift_change(&self) -> Option<PostRowAnchorSlideLiftChange> {
        self.state.on_slide_lift_change.borrow().clone()
    }

    fn set_on_slide_lift_change(&self, on_slide_lift_change: Option<PostRowAnchorSlideLiftChange>) {
        *self.state.on_slide_lift_change.borrow_mut() = on_slide_lift_change;
    }
}
