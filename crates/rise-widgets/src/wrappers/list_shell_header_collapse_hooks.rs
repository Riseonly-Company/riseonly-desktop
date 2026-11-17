use std::sync::OnceLock;

use gpui::{AnyWeakEntity, App, ScrollHandle};

#[derive(Clone, Copy)]
pub struct ListShellHeaderCollapseHooks {
    pub integration_scroll_view: fn(&AnyWeakEntity, &App) -> Option<ScrollHandle>,
    pub content_top_offset_y: fn(&AnyWeakEntity, &App) -> Option<f32>,
    pub settle_content_offset_from_top: fn(&AnyWeakEntity, f32, &mut App),
    pub item_count: fn(&AnyWeakEntity, &App) -> Option<usize>,
    pub set_content_offset_y: fn(&AnyWeakEntity, f32, &mut App),
}

static LIST_SHELL_HEADER_COLLAPSE_HOOKS: OnceLock<ListShellHeaderCollapseHooks> = OnceLock::new();

impl ListShellHeaderCollapseHooks {
    pub fn install(self) {
        let _ = LIST_SHELL_HEADER_COLLAPSE_HOOKS.set(self);
    }

    pub fn installed() -> Option<&'static ListShellHeaderCollapseHooks> {
        LIST_SHELL_HEADER_COLLAPSE_HOOKS.get()
    }
}
