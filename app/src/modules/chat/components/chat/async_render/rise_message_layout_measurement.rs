use gpui::App;
use rise_ui::swiftui::environment::{EnvKey, env};

/// true inside an off screen copy of a message laid out only to measure it
/// media in that copy draws its placeholder and starts no load
pub struct RiseMessageLayoutMeasurementKey;

impl EnvKey for RiseMessageLayoutMeasurementKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// is this a message layout measurement, from the environment
pub fn is_message_layout_measurement(cx: &App) -> bool {
    env::<RiseMessageLayoutMeasurementKey>(cx)
}
