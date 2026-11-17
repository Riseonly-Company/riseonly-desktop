use std::rc::{Rc, Weak};

use rise_ui::swiftui::environment::EnvKey;

use crate::core::shared_element_transition::runtime::shared_element_containers::SharedElementReturnScroller;

/// lets a profile page reach its return scroller without changing the initializer
#[derive(Clone)]
pub struct SharedElementReturnScrollerRef {
    coordinator: Weak<dyn SharedElementReturnScroller>,
    pub tab_key: i64,
    pub bottom_inset: f32,
}

impl SharedElementReturnScrollerRef {
    pub fn new(coordinator: &Rc<dyn SharedElementReturnScroller>, tab_key: i64, bottom_inset: f32) -> Self {
        Self { coordinator: Rc::downgrade(coordinator), tab_key, bottom_inset }
    }

    pub fn scroller(&self) -> Option<Rc<dyn SharedElementReturnScroller>> {
        self.coordinator.upgrade()
    }
}

impl PartialEq for SharedElementReturnScrollerRef {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.coordinator, &other.coordinator)
            && self.tab_key == other.tab_key
            && self.bottom_inset == other.bottom_inset
    }
}

pub struct SharedElementReturnScrollerKey;

impl EnvKey for SharedElementReturnScrollerKey {
    type Value = Option<SharedElementReturnScrollerRef>;

    fn default_value() -> Self::Value {
        None
    }
}
