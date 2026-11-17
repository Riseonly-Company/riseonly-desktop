use std::cell::Cell;
use std::rc::Rc;

use gpui::{ListOffset, ListState};

/// exact offset of one profile list kept outside view state so scroll ticks are never published
/// shared by reference
#[derive(Clone, Default)]
pub struct MapProfileScrollMemory {
    offset: Rc<Cell<Option<ListOffset>>>,
}

impl MapProfileScrollMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn offset(&self) -> Option<ListOffset> {
        self.offset.get()
    }

    pub fn set_offset(&self, offset: Option<ListOffset>) {
        self.offset.set(offset);
    }
}

/// follows one list without publishing scroll ticks
/// copies the list offset into the memory each frame
/// restores it once the list has enough rows again
pub struct MapProfileScrollAnchor {
    memory: MapProfileScrollMemory,
    scroll: Option<ListState>,
    pending_offset: Rc<Cell<Option<ListOffset>>>,
    restoring: bool,
}

impl MapProfileScrollAnchor {
    pub fn new(memory: MapProfileScrollMemory) -> Self {
        Self { memory, scroll: None, pending_offset: Rc::new(Cell::new(None)), restoring: false }
    }

    pub fn attach(&mut self, scroll: &ListState) {
        if self.scroll.is_none() {
            self.pending_offset.set(self.memory.offset());
            let pending_offset = self.pending_offset.clone();
            scroll.set_scroll_handler(move |_, _, _| pending_offset.set(None));
            self.scroll = Some(scroll.clone());
        }
        self.restore();
        self.record();
    }

    fn record(&self) {
        let Some(scroll) = &self.scroll else {
            return;
        };
        if self.restoring || self.pending_offset.get().is_some() {
            return;
        }
        self.memory.set_offset(Some(scroll.logical_scroll_top()));
    }

    fn restore(&mut self) {
        let (Some(scroll), Some(offset)) = (&self.scroll, self.pending_offset.get()) else {
            return;
        };
        if offset.item_ix >= scroll.item_count() {
            return;
        }
        self.restoring = true;
        scroll.scroll_to(offset);
        self.pending_offset.set(None);
        self.restoring = false;
    }

    pub fn detach(&mut self) {
        if let Some(scroll) = self.scroll.take() {
            scroll.set_scroll_handler(|_, _, _| {});
        }
    }
}

impl Drop for MapProfileScrollAnchor {
    fn drop(&mut self) {
        self.detach();
    }
}
