use gpui::Entity;
use rise_ui::swiftui::environment::EnvKey;

use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerLiveState;

/// carrier of the suppression flag, not the flag itself
/// it is read in the tap handler and nothing draws from it
/// a bool in the environment would rerender on every folder swipe
pub struct ChatsListRowActionsSuppressorKey;

impl EnvKey for ChatsListRowActionsSuppressorKey {
    type Value = Option<Entity<AnimatedTabsPagerLiveState>>;

    fn default_value() -> Self::Value {
        None
    }
}
