use gpui::{AnyView, App};

use crate::app::router::navigation_types::CreatePostIntent;
use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::media_picker::api::media_picker_result::MediaPickerTrackSelection;

pub struct FeedMediaPickerPageHooks {
    pub create_post_media_page:
        fn(CreatePostIntent, Option<MediaPickerTrackSelection>, Option<String>, &mut App) -> AnyView,
}

pub(crate) static FEED_MEDIA_PICKER_PAGE_HOOKS: FeatureHookSlot<FeedMediaPickerPageHooks> =
    FeatureHookSlot::new("FEED_MEDIA_PICKER_PAGE_HOOKS");

pub fn set_feed_media_picker_page_hooks(hooks: FeedMediaPickerPageHooks) {
    let _ = FEED_MEDIA_PICKER_PAGE_HOOKS.set(hooks);
}

pub fn feed_media_picker_page_hooks() -> Option<&'static FeedMediaPickerPageHooks> {
    FEED_MEDIA_PICKER_PAGE_HOOKS.get()
}
