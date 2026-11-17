use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_preparer::PostRowPreparer;
use crate::modules::user::components::profile_content::pages::grid_posts::post_row_warm_anchor_hooks::{
    PostRowWarmAnchorHooks, set_post_row_warm_anchor_hooks,
};

pub fn install_post_row_warm_anchor_hooks() {
    set_post_row_warm_anchor_hooks(PostRowWarmAnchorHooks { warm_anchor: PostRowPreparer::warm_anchor });
}
