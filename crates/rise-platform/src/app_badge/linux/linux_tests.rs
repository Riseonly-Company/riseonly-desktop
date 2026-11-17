use zbus::zvariant::{Type, Value};

use super::linux::*;
use crate::app_badge::launcher_entry::{LAUNCHER_COUNT_PROPERTY, LAUNCHER_COUNT_VISIBLE_PROPERTY};

#[test]
fn the_update_signal_carries_the_uri_and_a_property_dictionary() {
    assert_eq!(<UpdateBody as Type>::SIGNATURE.to_string(), "(sa{sv})");
}

#[test]
fn the_body_holds_the_number_and_its_visibility() {
    let (uri, properties) = update_body("application://a.desktop", 12);
    assert_eq!(uri, "application://a.desktop");
    assert_eq!(properties[LAUNCHER_COUNT_PROPERTY], Value::from(12i64));
    assert_eq!(properties[LAUNCHER_COUNT_VISIBLE_PROPERTY], Value::from(true));
}

#[test]
fn zero_hides_the_number_on_the_wire() {
    let (_, properties) = update_body("application://a.desktop", 0);
    assert_eq!(properties[LAUNCHER_COUNT_VISIBLE_PROPERTY], Value::from(false));
}
