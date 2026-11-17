use crate::app::aside::aside_navigator;
use crate::modules::shorts::pages::shorts_tab::shorts_tab_aside_hooks::{
    ShortsTabAsideHooks, set_shorts_tab_aside_hooks,
};

pub fn install_shorts_tab_aside_hooks() {
    set_shorts_tab_aside_hooks(ShortsTabAsideHooks { section_visits: aside_navigator::section_visits });
}
