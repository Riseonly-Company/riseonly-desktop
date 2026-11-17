use crate::tray::*;

#[test]
fn a_status_item_is_refused_off_the_main_thread_rather_than_touching_appkit() {
    if objc2::MainThreadMarker::new().is_some() {
        return;
    }

    assert_eq!(
        open(TrayConfig::new("R", "Riseonly")).err(),
        Some(TrayError::NotMainThread)
    );
}

#[test]
fn the_embedded_template_icon_decodes_at_menu_bar_size() {
    let image = super::macos::template_icon().expect("TrayIconTemplate.png must decode");
    assert!(image.isTemplate());
    assert_eq!(image.size().width, 18.0);
    assert_eq!(image.size().height, 18.0);
}
