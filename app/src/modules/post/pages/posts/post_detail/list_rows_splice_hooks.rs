use gpui::{ListOffset, ListState};

/// tells the list which rows changed by row id, keeps
/// rows on screen in place when rows arrive above them
pub fn splice_list_rows(state: &ListState, rendered: &mut Vec<String>, rows: &[String]) {
    if rendered.as_slice() == rows {
        return;
    }
    let prefix = rendered.iter().zip(rows).take_while(|(old, new)| old == new).count();
    let suffix = rendered[prefix..]
        .iter()
        .rev()
        .zip(rows[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    let top = state.logical_scroll_top();
    let replaced = prefix..rendered.len() - suffix;
    state.splice(replaced.clone(), rows.len() - prefix - suffix);
    if prefix == 0 && suffix == rendered.len() && !rendered.is_empty() {
        state.scroll_to(ListOffset {
            item_ix: top.item_ix + rows.len() - suffix,
            offset_in_item: top.offset_in_item,
        });
    } else if replaced.contains(&top.item_ix)
        && let Some(item_ix) = rendered.get(top.item_ix).and_then(|id| rows.iter().position(|row| row == id))
    {
        state.scroll_to(ListOffset { item_ix, offset_in_item: top.offset_in_item });
    }
    *rendered = rows.to_vec();
}
