use gpui::{Context, Entity};
use rise_ui::swiftui::environment::EnvKey;

/// stable per list carrier, only the typing and story leaves observe its value
/// suspending a covered page must not replace every hosted chat row
pub struct ChatListRowActivity {
    is_active: bool,
}

impl ChatListRowActivity {
    pub fn new() -> Self {
        Self { is_active: true }
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.is_active == active {
            return;
        }
        self.is_active = active;
        cx.notify();
    }
}

impl Default for ChatListRowActivity {
    fn default() -> Self {
        Self::new()
    }
}

/// reads the chat list row activity from the environment
pub struct ChatListRowActivityKey;

impl EnvKey for ChatListRowActivityKey {
    type Value = Option<Entity<ChatListRowActivity>>;

    fn default_value() -> Self::Value {
        None
    }
}
