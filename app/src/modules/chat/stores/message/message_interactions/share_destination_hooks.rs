use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

/// message service canonicalizes chat and topic and message ids before echoing a forward result
/// so compare canonical forms, else a delivered batch is reported as not sent
pub enum ShareForwardIdentity {}

impl ShareForwardIdentity {
    pub fn canonical(raw: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw?);
        if value.is_empty() {
            return None;
        }
        Some(
            Uuid::from_uuid_string(value)
                .map(|uuid| uuid.uuid_string().to_lowercase())
                .unwrap_or_else(|| value.to_owned()),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShareSheetDestination {
    pub chat_id: String,
    pub topic_id: Option<String>,
}

impl ShareSheetDestination {
    pub fn new(chat_id: &str, topic_id: Option<&str>) -> Self {
        Self {
            chat_id: ShareForwardIdentity::canonical(Some(chat_id)).unwrap_or_else(|| chat_id.to_owned()),
            topic_id: ShareForwardIdentity::canonical(topic_id),
        }
    }

    pub fn id(&self) -> String {
        format!("{}:{}", self.chat_id, self.topic_id.as_deref().unwrap_or_default())
    }
}
