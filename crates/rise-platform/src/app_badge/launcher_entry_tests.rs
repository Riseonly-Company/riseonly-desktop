use super::launcher_entry::*;

#[test]
fn the_app_uri_names_the_desktop_entry() {
    assert_eq!(launcher_app_uri("net.riseonly.desktop.dev"), "application://net.riseonly.desktop.dev.desktop");
}

#[test]
fn zero_hides_the_number() {
    assert_eq!(launcher_count(0), (0, false));
}

#[test]
fn the_number_is_shown_up_to_the_cap() {
    assert_eq!(launcher_count(1), (1, true));
    assert_eq!(launcher_count(999), (999, true));
    assert_eq!(launcher_count(1000), (999, true));
    assert_eq!(launcher_count(u32::MAX), (999, true));
}
