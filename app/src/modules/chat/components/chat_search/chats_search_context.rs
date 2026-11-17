use std::rc::Rc;

use gpui::{App, Context, Window};
use rise_ui::swiftui::environment::EnvKey;

use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;

pub type ChatsSearchVisibleChats = Rc<dyn Fn(&str, &[ChatInfo], &mut App) -> Vec<ChatInfo>>;
pub type ChatsSearchApplyHoldAction = Rc<dyn Fn(&str, &str, &[ChatInfo], &mut Window, &mut App)>;
pub type ChatsSearchSetSearchOverlay = Rc<dyn Fn(bool, &mut Window, &mut App)>;
pub type ChatsSearchSetQuery = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// members of the chats screen state the overlay and its tabs use
#[derive(Clone)]
pub struct ChatsSearchContext {
    pub visible_chats: ChatsSearchVisibleChats,
    pub apply_hold_action: ChatsSearchApplyHoldAction,
    pub set_search_overlay: ChatsSearchSetSearchOverlay,
    pub set_query: ChatsSearchSetQuery,
}

/// puts the search context into the environment
pub struct ChatsSearchContextKey;

impl EnvKey for ChatsSearchContextKey {
    type Value = Option<ChatsSearchContext>;

    fn default_value() -> Self::Value {
        None
    }
}

pub type ChatSearchKeyboardActivate = Rc<dyn Fn(&mut Window, &mut App)>;

/// one row the arrows can stop on
#[derive(Clone)]
pub struct ChatSearchKeyboardTarget {
    pub row_id: String,
    pub list_index: usize,
    pub activate: ChatSearchKeyboardActivate,
}

/// arrows and enter over the results
/// focus stays in the header field which isnt an ancestor of the results
/// so the overlay takes the keys and the tab publishes its rows here
pub struct ChatSearchResultsKeyboard {
    targets: Vec<ChatSearchKeyboardTarget>,
    selected_row_id: Option<String>,
}

impl ChatSearchResultsKeyboard {
    pub fn new() -> Self {
        Self { targets: Vec::new(), selected_row_id: None }
    }

    pub fn selected_row_id(&self) -> Option<&str> {
        self.selected_row_id.as_deref()
    }

    pub fn selected_list_index(&self) -> Option<usize> {
        let selected = self.selected_row_id.as_deref()?;
        self.targets.iter().find(|target| target.row_id == selected).map(|target| target.list_index)
    }

    /// closures are rebuilt every frame of the tab, only a selection that lost its row is news
    /// notifying always would rerender the tab from its own render
    pub fn publish(&mut self, targets: Vec<ChatSearchKeyboardTarget>, cx: &mut Context<Self>) {
        self.targets = targets;
        let selection_lost = self
            .selected_row_id
            .as_deref()
            .is_some_and(|selected| !self.targets.iter().any(|target| target.row_id == selected));
        if selection_lost {
            self.selected_row_id = None;
            cx.notify();
        }
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.targets.clear();
        if self.selected_row_id.take().is_some() {
            cx.notify();
        }
    }

    pub fn move_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.targets.is_empty() {
            return;
        }
        let current =
            self.selected_row_id.as_deref().and_then(|selected| self.targets.iter().position(|target| target.row_id == selected));
        let last = self.targets.len() - 1;
        let next = match (current, forward) {
            (None, true) => 0,
            (None, false) => last,
            (Some(index), true) => (index + 1).min(last),
            (Some(index), false) => index.saturating_sub(1),
        };
        self.selected_row_id = Some(self.targets[next].row_id.clone());
        cx.notify();
    }

    pub fn selected_action(&self) -> Option<ChatSearchKeyboardActivate> {
        let selected = self.selected_row_id.as_deref()?;
        self.targets.iter().find(|target| target.row_id == selected).map(|target| target.activate.clone())
    }
}

impl Default for ChatSearchResultsKeyboard {
    fn default() -> Self {
        Self::new()
    }
}

/// keyboard channel of the overlay for the tab that lists rows
pub struct ChatSearchResultsKeyboardKey;

impl EnvKey for ChatSearchResultsKeyboardKey {
    type Value = Option<gpui::Entity<ChatSearchResultsKeyboard>>;

    fn default_value() -> Self::Value {
        None
    }
}
