use rise_ui::guide_anchor_ui::guide_anchor_scroller_hooks::{
    GuideAnchorScrollerHooks, set_guide_anchor_scroller_hooks,
};

use crate::core::ui::guide_anchor_ui::guide_anchor_scroller::GuideAnchorScroller;

pub fn install_guide_anchor_scroller_hooks() {
    set_guide_anchor_scroller_hooks(GuideAnchorScrollerHooks { note_mounted: GuideAnchorScroller::note_mounted });
}
