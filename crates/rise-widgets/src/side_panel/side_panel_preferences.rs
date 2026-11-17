use gpui::Pixels;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{
    DESKTOP_LIST_COLUMN_WIDTH_PREFIX, DESKTOP_SIDE_PANEL_CLOSED, DESKTOP_SIDE_PANEL_SECTION_WIDTH_PREFIX,
    DESKTOP_SIDE_PANEL_WIDTH,
};
use rise_theme::PanelMetrics;

/// what the right panel remembers between launches
/// contexts the person closed it for and the width they left it at, per section
#[derive(Clone)]
pub struct SidePanelPreferences {
    defaults: UserDefaults,
}

impl SidePanelPreferences {
    pub fn standard() -> Self {
        Self::new(UserDefaults::standard())
    }

    pub fn new(defaults: UserDefaults) -> Self {
        Self { defaults }
    }

    /// kept for this run only, previews and tests
    pub fn in_memory() -> Self {
        Self::new(UserDefaults::in_memory())
    }

    pub fn is_open(&self, context: &str) -> bool {
        !self.closed_contexts().iter().any(|closed| closed == context)
    }

    pub fn remember_open(&self, context: &str, open: bool) {
        let mut closed = self.closed_contexts();
        let was_closed = closed.iter().any(|entry| entry == context);
        match (open, was_closed) {
            (true, true) => closed.retain(|entry| entry != context),
            (false, false) => closed.push(context.to_owned()),
            _ => return,
        }
        self.defaults.set(DESKTOP_SIDE_PANEL_CLOSED, closed);
    }

    /// remembered width pulled back into the range the panel may take now, density may have changed
    pub fn width(&self, metrics: &PanelMetrics) -> Pixels {
        let stored = self.defaults.double(DESKTOP_SIDE_PANEL_WIDTH);
        if !stored.is_finite() || stored <= 0.0 {
            return metrics.default_width;
        }
        gpui::px(stored as f32).clamp(metrics.min_width, metrics.max_width)
    }

    pub fn set_width(&self, width: Pixels) {
        self.defaults
            .set(DESKTOP_SIDE_PANEL_WIDTH, f64::from(f32::from(width)));
    }

    /// panel width the person chose in this section, none if they never dragged it there
    /// then a reading centre hands the panel its spare room
    pub fn chosen_width(&self, scope: &str, metrics: &PanelMetrics) -> Option<Pixels> {
        Self::stored(&self.defaults, &Self::scoped(DESKTOP_SIDE_PANEL_SECTION_WIDTH_PREFIX, scope))
            .map(|width| width.clamp(metrics.min_width, metrics.max_width))
    }

    /// also stores it as the last width any section chose, which the pages reading width follow
    pub fn remember_chosen_width(&self, scope: &str, width: Pixels) {
        self.defaults.set(
            &Self::scoped(DESKTOP_SIDE_PANEL_SECTION_WIDTH_PREFIX, scope),
            f64::from(f32::from(width)),
        );
        self.set_width(width);
    }

    /// double click on the grip, the section panel goes back to its default width
    pub fn forget_chosen_width(&self, scope: &str) {
        self.defaults
            .remove_object(&Self::scoped(DESKTOP_SIDE_PANEL_SECTION_WIDTH_PREFIX, scope));
    }

    /// list column width chosen in this section, none for the default
    pub fn list_width(&self, scope: &str, metrics: &PanelMetrics) -> Option<Pixels> {
        Self::stored(&self.defaults, &Self::scoped(DESKTOP_LIST_COLUMN_WIDTH_PREFIX, scope))
            .map(|width| width.clamp(metrics.list_min_width, metrics.list_max_width))
    }

    pub fn remember_list_width(&self, scope: &str, width: Pixels) {
        self.defaults.set(
            &Self::scoped(DESKTOP_LIST_COLUMN_WIDTH_PREFIX, scope),
            f64::from(f32::from(width)),
        );
    }

    pub fn forget_list_width(&self, scope: &str) {
        self.defaults
            .remove_object(&Self::scoped(DESKTOP_LIST_COLUMN_WIDTH_PREFIX, scope));
    }

    fn scoped(prefix: &str, scope: &str) -> String {
        format!("{prefix}{scope}")
    }

    fn stored(defaults: &UserDefaults, key: &str) -> Option<Pixels> {
        let stored = defaults.double(key);
        (stored.is_finite() && stored > 0.0).then(|| gpui::px(stored as f32))
    }

    fn closed_contexts(&self) -> Vec<String> {
        self.defaults
            .string_array(DESKTOP_SIDE_PANEL_CLOSED)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn preferences() -> SidePanelPreferences {
        SidePanelPreferences::in_memory()
    }

    #[test]
    fn a_context_never_closed_shows_its_panel() {
        assert!(preferences().is_open("chat_profile"));
    }

    #[test]
    fn closing_is_remembered_per_context() {
        let preferences = preferences();
        preferences.remember_open("chat_profile", false);

        assert!(!preferences.is_open("chat_profile"));
        assert!(preferences.is_open("music_queue"));

        preferences.remember_open("chat_profile", true);
        assert!(preferences.is_open("chat_profile"));
    }

    #[test]
    fn closing_twice_keeps_one_entry() {
        let preferences = preferences();
        preferences.remember_open("sticker_pack", false);
        preferences.remember_open("sticker_pack", false);
        preferences.remember_open("sticker_pack", true);

        assert!(preferences.is_open("sticker_pack"));
    }

    #[test]
    fn the_width_comes_back_inside_the_allowed_range() {
        let preferences = preferences();
        let metrics = PanelMetrics::default();
        assert_eq!(preferences.width(&metrics), metrics.default_width);

        preferences.set_width(metrics.min_width + px(20.0));
        assert_eq!(preferences.width(&metrics), metrics.min_width + px(20.0));

        preferences.set_width(px(5_000.0));
        assert_eq!(preferences.width(&metrics), metrics.max_width);
    }

    #[test]
    fn each_section_keeps_its_own_panel_width() {
        let preferences = preferences();
        let metrics = PanelMetrics::default();
        assert_eq!(preferences.chosen_width("aside_posts", &metrics), None);

        preferences.remember_chosen_width("aside_posts", metrics.min_width + px(30.0));
        preferences.remember_chosen_width("aside_chats", metrics.max_width + px(30.0));

        assert_eq!(preferences.chosen_width("aside_posts", &metrics), Some(metrics.min_width + px(30.0)));
        assert_eq!(preferences.chosen_width("aside_chats", &metrics), Some(metrics.max_width));
        assert_eq!(preferences.chosen_width("aside_riffs", &metrics), None);

        preferences.forget_chosen_width("aside_posts");
        assert_eq!(preferences.chosen_width("aside_posts", &metrics), None);
    }

    #[test]
    fn the_list_width_is_kept_per_section_inside_its_range() {
        let preferences = preferences();
        let metrics = PanelMetrics::default();
        assert_eq!(preferences.list_width("aside_chats", &metrics), None);

        preferences.remember_list_width("aside_chats", metrics.list_min_width + px(12.0));
        preferences.remember_list_width("aside_settings", px(1.0));

        assert_eq!(preferences.list_width("aside_chats", &metrics), Some(metrics.list_min_width + px(12.0)));
        assert_eq!(preferences.list_width("aside_settings", &metrics), Some(metrics.list_min_width));

        preferences.forget_list_width("aside_chats");
        assert_eq!(preferences.list_width("aside_chats", &metrics), None);
    }
}
