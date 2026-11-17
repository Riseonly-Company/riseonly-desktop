use gpui::{AnyWeakEntity, App, ScrollHandle, px};
use rise_widgets::wrappers::list_shell_header_collapse_hooks::ListShellHeaderCollapseHooks;

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;

pub fn install_rise_manual_list_list_shell_hooks() {
    ListShellHeaderCollapseHooks {
        integration_scroll_view,
        content_top_offset_y,
        settle_content_offset_from_top,
        item_count,
        set_content_offset_y,
    }
    .install();
}

fn integration_scroll_view(list: &AnyWeakEntity, cx: &App) -> Option<ScrollHandle> {
    let list = list.upgrade()?.downcast::<RiseManualListView>().ok()?;
    Some(list.read(cx).integration_scroll_handle())
}

fn content_top_offset_y(list: &AnyWeakEntity, cx: &App) -> Option<f32> {
    let list = list.upgrade()?.downcast::<RiseManualListView>().ok()?;
    Some(f32::from(list.read(cx).content_top_offset_y()))
}

fn settle_content_offset_from_top(list: &AnyWeakEntity, offset: f32, cx: &mut App) {
    let Some(list) = list.upgrade().and_then(|list| list.downcast::<RiseManualListView>().ok()) else {
        return;
    };
    let window_handle = list.read(cx).window_handle();
    let _ = window_handle.update(cx, |_, window, cx| {
        list.update(cx, |list, cx| list.settle_content_offset_from_top(px(offset), window, cx));
    });
}

fn item_count(list: &AnyWeakEntity, cx: &App) -> Option<usize> {
    let list = list.upgrade()?.downcast::<RiseManualListView>().ok()?;
    Some(list.read(cx).item_count())
}

fn set_content_offset_y(list: &AnyWeakEntity, value: f32, cx: &mut App) {
    let Some(list) = list.upgrade().and_then(|list| list.downcast::<RiseManualListView>().ok()) else {
        return;
    };
    let window_handle = list.read(cx).window_handle();
    let _ = window_handle.update(cx, |_, window, cx| {
        list.update(cx, |list, cx| {
            let _ = list.set_content_offset_y(px(value), false, true, window, cx);
        });
    });
}
