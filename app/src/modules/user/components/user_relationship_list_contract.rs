use rise_canon::RiseLimits;
use rise_ui::swiftui::motion::{MotionSpec, spring};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

pub enum UserRelationshipSearchHero {}

impl UserRelationshipSearchHero {
    pub const FIELD_ID: &'static str = "userRelationshipSearchField";

    pub fn animation() -> MotionSpec {
        spring(0.42, 0.86)
    }
}

pub enum UserRelationshipRowMetrics {}

impl UserRelationshipRowMetrics {
    pub const AVATAR_SIZE: f32 = 50.0;
    pub const CONTENT_SPACING: f32 = 12.0;
    pub const HORIZONTAL_PADDING: f32 = 12.0;
    pub const VERTICAL_PADDING: f32 = 10.0;

    pub fn relationship_button_minimum_height() -> f32 {
        RiseLimits::client_value("user_relationship_row_metrics_relationship_button_minimum_height") as f32
    }
}
