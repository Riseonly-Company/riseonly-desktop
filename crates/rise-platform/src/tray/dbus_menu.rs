use super::tray::{TrayEvent, TrayMenuItem};

pub const DBUS_MENU_PATH: &str = "/MenuBar";
pub const DBUS_MENU_VERSION: u32 = 3;
pub const DBUS_MENU_TEXT_DIRECTION: &str = "ltr";
pub const DBUS_MENU_STATUS: &str = "normal";
pub const DBUS_MENU_ROOT_ID: i32 = 0;

const CLICKED_EVENT: &str = "clicked";
const CHILDREN_DISPLAY_PROPERTY: &str = "children-display";
const CHILDREN_DISPLAY_SUBMENU: &str = "submenu";
const LABEL_PROPERTY: &str = "label";
const TOGGLE_TYPE_PROPERTY: &str = "toggle-type";
const TOGGLE_TYPE_CHECKMARK: &str = "checkmark";
const TOGGLE_STATE_PROPERTY: &str = "toggle-state";
const ENABLED_PROPERTY: &str = "enabled";

/// one property value, the bus layer turns it into a dbus variant
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MenuProperty {
    Text(String),
    Number(i32),
    Flag(bool),
}

pub type MenuProperties = Vec<(&'static str, MenuProperty)>;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MenuNode {
    pub id: i32,
    pub properties: MenuProperties,
    pub children: Vec<MenuNode>,
}

pub fn escape_menu_label(label: &str) -> String {
    label.replace('_', "__")
}

fn item_properties(item: &TrayMenuItem) -> MenuProperties {
    let mut properties = vec![(LABEL_PROPERTY, MenuProperty::Text(escape_menu_label(&item.label)))];
    if let Some(checked) = item.checked {
        properties.push((
            TOGGLE_TYPE_PROPERTY,
            MenuProperty::Text(TOGGLE_TYPE_CHECKMARK.to_owned()),
        ));
        properties.push((TOGGLE_STATE_PROPERTY, MenuProperty::Number(i32::from(checked))));
    }
    if !item.enabled {
        properties.push((ENABLED_PROPERTY, MenuProperty::Flag(false)));
    }
    properties
}

fn root_properties() -> MenuProperties {
    vec![(
        CHILDREN_DISPLAY_PROPERTY,
        MenuProperty::Text(CHILDREN_DISPLAY_SUBMENU.to_owned()),
    )]
}

fn wire_id(item: &TrayMenuItem) -> i32 {
    item.id as i32
}

/// properties of one id, None when the menu doesnt hold it
pub fn menu_node_properties(items: &[TrayMenuItem], id: i32) -> Option<MenuProperties> {
    if id == DBUS_MENU_ROOT_ID {
        return Some(root_properties());
    }
    items
        .iter()
        .find(|item| wire_id(item) == id)
        .map(item_properties)
}

/// for GetLayout, depth 0 is the node alone and anything else takes its children since the menu is flat
pub fn menu_layout(items: &[TrayMenuItem], parent_id: i32, depth: i32) -> Option<MenuNode> {
    let properties = menu_node_properties(items, parent_id)?;
    let children = if parent_id == DBUS_MENU_ROOT_ID && depth != 0 {
        items
            .iter()
            .map(|item| MenuNode {
                id: wire_id(item),
                properties: item_properties(item),
                children: Vec::new(),
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(MenuNode {
        id: parent_id,
        properties,
        children,
    })
}

/// for GetLayout and GetGroupProperties, an empty names list means all of them
pub fn menu_property_wanted(name: &str, names: &[String]) -> bool {
    names.is_empty() || names.iter().any(|wanted| wanted == name)
}

/// for GetGroupProperties, an empty ids list means every node
pub fn menu_group_ids(items: &[TrayMenuItem], ids: &[i32]) -> Vec<i32> {
    if !ids.is_empty() {
        return ids.to_vec();
    }
    std::iter::once(DBUS_MENU_ROOT_ID)
        .chain(items.iter().map(wire_id))
        .collect()
}

/// what an Event call means
/// only a click on an item counts
/// hosts also send opened, closed and hovered
pub fn menu_event(items: &[TrayMenuItem], id: i32, event_id: &str) -> Option<TrayEvent> {
    if event_id != CLICKED_EVENT {
        return None;
    }
    items
        .iter()
        .find(|item| wire_id(item) == id)
        .map(|item| TrayEvent::Menu(item.id))
}
