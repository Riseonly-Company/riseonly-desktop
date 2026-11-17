use std::sync::OnceLock;

use rise_canon::RiseGuideAnchor;

use crate::guide_anchor_ui::guide_anchor_modifier::GuideAnchorBackingView;

pub struct GuideAnchorScrollerHooks {
    pub note_mounted: fn(anchor: RiseGuideAnchor, view: &GuideAnchorBackingView),
}

static GUIDE_ANCHOR_SCROLLER_HOOKS: OnceLock<GuideAnchorScrollerHooks> = OnceLock::new();

/// installs the hooks once, a second call keeps the first installer hooks
pub fn set_guide_anchor_scroller_hooks(hooks: GuideAnchorScrollerHooks) {
    let _ = GUIDE_ANCHOR_SCROLLER_HOOKS.set(hooks);
}

pub fn guide_anchor_scroller_hooks() -> Option<&'static GuideAnchorScrollerHooks> {
    GUIDE_ANCHOR_SCROLLER_HOOKS.get()
}
