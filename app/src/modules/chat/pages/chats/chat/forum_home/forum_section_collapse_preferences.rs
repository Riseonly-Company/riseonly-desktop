use gpui::{App, Context, Entity};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::FORUM_COLLAPSED_SECTIONS;

/// forum sections folded on this device
/// local by design, folding is how one reader skims a group and not a fact about it
/// section ids are unique across chats so one set serves all
pub struct ForumSectionCollapsePreferences {
    collapsed_section_ids: Vec<String>,
}

impl ForumSectionCollapsePreferences {
    const COLLAPSED_KEY: &'static str = FORUM_COLLAPSED_SECTIONS;
    const MAX_STORED: usize = 512;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { collapsed_section_ids: UserDefaults::standard().string_array(Self::COLLAPSED_KEY).unwrap_or_default() }
    }

    pub fn collapsed_section_ids(&self) -> &[String] {
        &self.collapsed_section_ids
    }

    pub fn is_collapsed(&self, section_id: &str) -> bool {
        self.collapsed_section_ids.iter().any(|id| id == section_id)
    }

    pub fn toggle(&mut self, section_id: &str, cx: &mut Context<Self>) {
        let mut next = self.collapsed_section_ids.clone();
        if let Some(index) = next.iter().position(|id| id == section_id) {
            next.remove(index);
        } else {
            next.push(section_id.to_owned());
            if next.len() > Self::MAX_STORED {
                next.drain(..next.len() - Self::MAX_STORED);
            }
        }
        self.collapsed_section_ids = next.clone();
        UserDefaults::standard().set(Self::COLLAPSED_KEY, next);
        cx.notify();
    }
}
