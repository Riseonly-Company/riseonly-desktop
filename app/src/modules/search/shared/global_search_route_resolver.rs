use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::app::router::navigation_types::RootRoute;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;

pub enum GlobalSearchRouteResolver {}

impl GlobalSearchRouteResolver {
    pub fn route(row: &SearchEntityRow) -> Option<RootRoute> {
        if row.is_channel() || row.is_group() {
            let chat_id = Self::normalized(row.chat_id.as_deref())?;
            return Some(RootRoute::Chat { chat_id: Some(chat_id) });
        }

        let user_id = Self::normalized(row.user_id.as_deref()).or_else(|| Self::normalized(Some(&row.id)));
        let user_tag = Self::normalized(row.user_tag.as_deref());
        if user_id.is_some() || user_tag.is_some() {
            return Some(RootRoute::UserPage { user_id, tag: user_tag });
        }
        None
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(value?);
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }
}
