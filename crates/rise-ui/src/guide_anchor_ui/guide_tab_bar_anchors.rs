use gpui::{AnyElement, IntoElement};
use rise_canon::RiseGuideAnchor;

use crate::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;

pub struct GuideTabBarAnchors {
    pub anchors: Vec<RiseGuideAnchor>,
}

impl GuideTabBarAnchors {
    pub fn new(anchors: Vec<RiseGuideAnchor>) -> Self {
        Self { anchors }
    }

    pub fn install(&self, buttons: Vec<AnyElement>) -> Vec<AnyElement> {
        if self.anchors.is_empty() || buttons.len() != self.anchors.len() {
            return buttons;
        }
        buttons
            .into_iter()
            .zip(self.anchors.iter().copied())
            .map(|(button, anchor)| button.guide_anchor(anchor).into_any_element())
            .collect()
    }
}
