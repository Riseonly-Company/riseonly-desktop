use super::dbus_menu::*;
use super::tray::{TrayEvent, TrayMenuItem};

fn items() -> Vec<TrayMenuItem> {
    vec![TrayMenuItem::plain(7, "Open"), TrayMenuItem::check(9, "Start_at_login", true)]
}

#[test]
fn an_empty_menu_is_a_root_without_children() {
    let root = menu_layout(&[], DBUS_MENU_ROOT_ID, -1).expect("the root");
    assert_eq!(root.id, DBUS_MENU_ROOT_ID);
    assert!(root.children.is_empty());
}

#[test]
fn the_root_lists_the_items_unless_asked_for_no_depth() {
    let all = menu_layout(&items(), DBUS_MENU_ROOT_ID, -1).expect("the root");
    assert_eq!(all.id, DBUS_MENU_ROOT_ID);
    assert_eq!(all.children.iter().map(|node| node.id).collect::<Vec<_>>(), vec![7, 9]);
    assert!(menu_layout(&items(), DBUS_MENU_ROOT_ID, 0).expect("the root").children.is_empty());
    assert!(menu_layout(&[], DBUS_MENU_ROOT_ID, -1).expect("the root").children.is_empty());
}

#[test]
fn an_item_has_no_children_and_an_unknown_id_has_no_layout() {
    assert!(menu_layout(&items(), 7, -1).expect("an item").children.is_empty());
    assert_eq!(menu_layout(&items(), 8, -1), None);
}

#[test]
fn a_check_item_carries_its_toggle_and_a_plain_one_does_not() {
    let checked = menu_node_properties(&items(), 9).expect("the check item");
    assert!(checked.contains(&("toggle-type", MenuProperty::Text("checkmark".to_owned()))));
    assert!(checked.contains(&("toggle-state", MenuProperty::Number(1))));
    let off = menu_node_properties(&[TrayMenuItem::check(9, "x", false)], 9).expect("the item");
    assert!(off.contains(&("toggle-state", MenuProperty::Number(0))));
    let plain = menu_node_properties(&items(), 7).expect("the plain item");
    assert_eq!(plain, vec![("label", MenuProperty::Text("Open".to_owned()))]);
}

#[test]
fn an_underscore_in_a_label_stays_visible() {
    let checked = menu_node_properties(&items(), 9).expect("the check item");
    assert!(checked.contains(&("label", MenuProperty::Text("Start__at__login".to_owned()))));
}

#[test]
fn an_empty_list_of_names_or_ids_means_all() {
    assert!(menu_property_wanted("label", &[]));
    assert!(menu_property_wanted("label", &["label".to_owned()]));
    assert!(!menu_property_wanted("label", &["type".to_owned()]));
    assert_eq!(menu_group_ids(&items(), &[]), vec![DBUS_MENU_ROOT_ID, 7, 9]);
    assert_eq!(menu_group_ids(&items(), &[9]), vec![9]);
}

#[test]
fn only_a_click_on_a_known_item_is_an_event() {
    assert_eq!(menu_event(&items(), 7, "clicked"), Some(TrayEvent::Menu(7)));
    assert_eq!(menu_event(&items(), 7, "hovered"), None);
    assert_eq!(menu_event(&items(), 8, "clicked"), None);
    assert_eq!(menu_event(&items(), DBUS_MENU_ROOT_ID, "clicked"), None);
}

#[test]
fn a_disabled_item_says_so_and_an_enabled_one_stays_silent() {
    let greyed = menu_node_properties(&[TrayMenuItem::plain(5, "Mic").disabled()], 5).expect("the item");
    assert!(greyed.contains(&("enabled", MenuProperty::Flag(false))));
    let plain = menu_node_properties(&items(), 7).expect("the plain item");
    assert!(!plain.iter().any(|(name, _)| *name == "enabled"));
}
