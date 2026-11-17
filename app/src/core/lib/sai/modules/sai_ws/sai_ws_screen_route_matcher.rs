use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{
    SaiActiveScreen, SaiWsScreenRefreshParams,
};

pub enum SaiWsScreenRouteMatcher {}

impl SaiWsScreenRouteMatcher {
    pub fn matches(
        when_on_screen: &[String],
        route_params: Option<&SaiWsScreenRefreshParams>,
        current: &SaiActiveScreen,
    ) -> bool {
        let allowed: Vec<&str> = when_on_screen
            .iter()
            .map(|screen| trimming_whitespaces_and_newlines(screen))
            .filter(|screen| !screen.is_empty())
            .collect();
        if allowed.is_empty() {
            return true;
        }
        if !allowed.contains(&current.id.as_str()) {
            return false;
        }
        let Some(route_params) = route_params else {
            return true;
        };

        match route_params {
            SaiWsScreenRefreshParams::Flat(flat) => flat
                .iter()
                .all(|(key, required)| current.params.get(key) == Some(required)),
            SaiWsScreenRefreshParams::PerScreen(per_screen) => {
                let Some(required) = per_screen.get(&current.id) else {
                    return true;
                };
                required
                    .iter()
                    .all(|(key, value)| current.params.get(key) == Some(value))
            }
            SaiWsScreenRefreshParams::PerScreenPresentKeys(spec) => {
                let Some(keys) = spec.get(&current.id) else {
                    return true;
                };
                keys.iter().all(|key| {
                    current
                        .params
                        .get(key)
                        .is_some_and(|value| !trimming_whitespaces_and_newlines(value).is_empty())
                })
            }
        }
    }
}
