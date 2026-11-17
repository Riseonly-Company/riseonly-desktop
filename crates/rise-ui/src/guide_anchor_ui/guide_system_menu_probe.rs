use gpui::{Bounds, Pixels, Point};

use crate::guide_anchor_ui::guide_anchor_registry::{GuideAnchorCoverKind, GuideAnchorRegistry};

pub enum GuideSystemMenuProbe {}

impl GuideSystemMenuProbe {
    /// menu frame in window coordinates or none when no menu is up
    /// a menu filling the window counts as none, dialogs are checked first and topmost wins within a kind
    pub fn menu_frame() -> Option<Bounds<Pixels>> {
        let covers = GuideAnchorRegistry::menu_covers();
        for kind in [GuideAnchorCoverKind::Alert, GuideAnchorCoverKind::Menu] {
            for cover in covers.iter().filter(|cover| cover.kind == kind) {
                let window = Bounds { origin: Point::default(), size: cover.viewport };
                let frame = cover.frame.intersect(&window);
                let width = f32::from(frame.size.width);
                let height = f32::from(frame.size.height);
                if width <= 1.0 || height <= 1.0 {
                    continue;
                }
                let fills_screen = width >= f32::from(window.size.width) * 0.98
                    && height >= f32::from(window.size.height) * 0.9;
                if fills_screen {
                    continue;
                }
                return Some(frame);
            }
        }
        None
    }
}
