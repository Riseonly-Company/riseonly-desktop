use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::post::stores::post_interactions::post_interactions_hooks::{
    PostInteractionsCommentHooks, PostInteractionsHooks, set_post_interactions_hooks,
};

pub fn install_post_interactions_hooks() {
    set_post_interactions_hooks(PostInteractionsHooks {
        comment: PostInteractionsCommentHooks {
            open_comments_sheet: |post, caption_expanded, cx| {
                CommentInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.open_comments_sheet(post, None, caption_expanded, false, cx);
                });
            },
        },
    });
}
