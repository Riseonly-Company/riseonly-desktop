use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, WeakEntity};

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;
use crate::modules::chat::pages::chats::rise_chat_native_list::RiseChatNativeListControl;

#[derive(Clone)]
struct ChatsPageViewportAnchor {
    id: String,
    offset: f32,
}

#[derive(Clone)]
struct ChatsPageViewportBookmark {
    anchors: Vec<ChatsPageViewportAnchor>,
    offset_from_top: f32,
}

struct ChatsPageViewportPage {
    list: Option<WeakEntity<RiseManualListView>>,
    row_ids: HashSet<String>,
    bookmark: Option<ChatsPageViewportBookmark>,
    control: Rc<RiseChatNativeListControl>,
}

impl ChatsPageViewportPage {
    fn new() -> Self {
        Self {
            list: None,
            row_ids: HashSet::new(),
            bookmark: None,
            control: Rc::new(RiseChatNativeListControl::new()),
        }
    }
}

/// only exact pixel bookmarks survive folder eviction
/// not observable so reading scroll never redraws the chats body
#[derive(Default)]
pub struct ChatsPageViewportStore {
    pages: RefCell<HashMap<String, ChatsPageViewportPage>>,
    account_key: RefCell<Option<String>>,
}

impl ChatsPageViewportStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn activate_account(&self, key: &str) {
        let mut account_key = self.account_key.borrow_mut();
        if account_key.as_deref() == Some(key) {
            return;
        }
        if account_key.is_some() {
            self.pages.borrow_mut().clear();
        }
        *account_key = Some(key.to_owned());
    }

    pub fn retain(&self, ids: &HashSet<String>) {
        self.pages.borrow_mut().retain(|id, _| ids.contains(id));
    }

    pub fn has_bookmark(&self, id: &str) -> bool {
        self.pages.borrow().get(id).is_some_and(|page| page.bookmark.is_some())
    }

    pub fn control(&self, id: &str) -> Rc<RiseChatNativeListControl> {
        self.pages
            .borrow_mut()
            .entry(id.to_owned())
            .or_insert_with(ChatsPageViewportPage::new)
            .control
            .clone()
    }

    pub fn restoration_ids(&self, id: &str) -> Vec<String> {
        self.pages
            .borrow()
            .get(id)
            .and_then(|page| page.bookmark.as_ref())
            .map(|bookmark| bookmark.anchors.iter().map(|anchor| anchor.id.clone()).collect())
            .unwrap_or_default()
    }

    pub fn restoration(&self, id: &str, among: Option<&HashSet<String>>) -> Option<(String, f32)> {
        let pages = self.pages.borrow();
        let bookmark = pages.get(id)?.bookmark.as_ref()?;
        bookmark
            .anchors
            .iter()
            .find(|anchor| among.is_none_or(|ids| ids.contains(&anchor.id)))
            .map(|anchor| (anchor.id.clone(), anchor.offset))
    }

    pub fn resting_offset(&self, id: &str) -> Option<f32> {
        self.pages.borrow().get(id)?.bookmark.as_ref().map(|bookmark| bookmark.offset_from_top)
    }

    pub fn capture(&self, id: &str, cx: &App) {
        let mut pages = self.pages.borrow_mut();
        let Some(page) = pages.get_mut(id) else {
            return;
        };
        let Some(list) = page.list.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let list = list.read(cx);
        let top = list.viewport_insets().top;
        let bottom = list.bounds().size.height - list.viewport_insets().bottom;
        let mut anchors: Vec<ChatsPageViewportAnchor> = list
            .visible_ids()
            .iter()
            .filter_map(|id| {
                if !page.row_ids.contains(id) {
                    return None;
                }
                let frame = list.viewport_frame(id)?;
                if frame.bottom() <= top || frame.origin.y >= bottom {
                    return None;
                }
                Some(ChatsPageViewportAnchor { id: id.clone(), offset: f32::from(frame.origin.y - top) })
            })
            .collect();
        anchors.sort_by(|left, right| left.offset.total_cmp(&right.offset));
        if anchors.is_empty() && page.bookmark.is_some() {
            return;
        }
        if anchors.is_empty() && f32::from(list.scroll_metrics().content_height) <= 0.0 {
            return;
        }
        page.bookmark = Some(ChatsPageViewportBookmark {
            anchors,
            offset_from_top: f32::from(list.scroll_metrics().distance_from_top),
        });
    }

    pub fn capture_all(&self, cx: &App) {
        let ids: Vec<String> = self.pages.borrow().keys().cloned().collect();
        for id in ids {
            self.capture(&id, cx);
        }
    }

    pub fn attach_native(&self, list: WeakEntity<RiseManualListView>, ids: HashSet<String>, page_id: &str) {
        let mut pages = self.pages.borrow_mut();
        let page = pages.entry(page_id.to_owned()).or_insert_with(ChatsPageViewportPage::new);
        page.list = Some(list);
        page.row_ids = ids;
    }

    pub fn detach(&self, list: &WeakEntity<RiseManualListView>, page_id: &str, cx: &App) {
        let is_attached = self.pages.borrow().get(page_id).is_some_and(|page| page.list.as_ref() == Some(list));
        if !is_attached {
            return;
        }
        self.capture(page_id, cx);
        if let Some(page) = self.pages.borrow_mut().get_mut(page_id) {
            page.list = None;
            page.row_ids.clear();
        }
    }
}
