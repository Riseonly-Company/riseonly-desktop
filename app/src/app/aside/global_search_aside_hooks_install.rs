use crate::app::aside::aside_navigator;
use crate::modules::search::pages::global_search::global_search_aside_hooks::{
    GlobalSearchAsideHooks, set_global_search_aside_hooks,
};

pub fn install_global_search_aside_hooks() {
    set_global_search_aside_hooks(GlobalSearchAsideHooks { replace: aside_navigator::replace });
}
