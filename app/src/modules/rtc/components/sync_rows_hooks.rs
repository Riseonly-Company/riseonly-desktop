use rise_ui::list_ui::list_ui::ListUi;

pub fn sync_rows(list: &ListUi, old_keys: &[String], new_keys: &[String]) {
    let prefix = old_keys.iter().zip(new_keys).take_while(|(old, new)| old == new).count();
    if prefix == 0 {
        list.replaced(new_keys.len());
        return;
    }
    let suffix = old_keys[prefix..]
        .iter()
        .rev()
        .zip(new_keys[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    list.state().splice(prefix..old_keys.len() - suffix, new_keys.len() - prefix - suffix);
}
