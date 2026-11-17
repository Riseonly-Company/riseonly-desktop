use gpui::Size;

use crate::modules::media_picker::library::photo_library_fetcher::MediaPickerGridThumbnailPolicy;

#[test]
fn direct_grid_request_uses_telegram_sized_thumbnail() {
    assert_eq!(
        MediaPickerGridThumbnailPolicy::TARGET_SIZE,
        Size {
            width: 256.0,
            height: 256.0,
        }
    );
}

#[test]
fn direct_grid_request_stays_within_decode_budget() {
    let target = MediaPickerGridThumbnailPolicy::TARGET_SIZE;
    assert!(target.width.max(target.height) <= 256.0);
}
