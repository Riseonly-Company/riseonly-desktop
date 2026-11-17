use crate::app::aside::aside_navigator;
use crate::modules::comment::widgets::comments_sheet_aside_hooks::{
    CommentsSheetAsideHooks, set_comments_sheet_aside_hooks,
};

pub fn install_comments_sheet_aside_hooks() {
    set_comments_sheet_aside_hooks(CommentsSheetAsideHooks {
        steps_back: aside_navigator::steps_back,
        docks_comments: aside_navigator::docks_comments,
    });
}
