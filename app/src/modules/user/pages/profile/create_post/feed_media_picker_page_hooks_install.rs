use gpui::{AnyView, AppContext};

use crate::modules::camera::components::feed_media_picker_page_hooks::{
    FeedMediaPickerPageHooks, set_feed_media_picker_page_hooks,
};
use crate::modules::user::pages::profile::create_post::create_post_media_page::CreatePostMediaPage;

pub fn install_feed_media_picker_page_hooks() {
    set_feed_media_picker_page_hooks(FeedMediaPickerPageHooks {
        create_post_media_page: |intent, track_seed, journey_id, cx| {
            AnyView::from(cx.new(|_| CreatePostMediaPage::new(intent, track_seed, journey_id)))
        },
    });
}
