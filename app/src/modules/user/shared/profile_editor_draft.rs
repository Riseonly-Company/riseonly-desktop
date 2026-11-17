use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::PROFILE_EDITOR_DRAFT_PREFIX;
use serde::{Deserialize, Serialize};

/// unconfirmed profile editor input,
/// device local and per account,
/// never sent. profile stays unchanged until confirmed
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileEditorDraft {
    pub name: String,
    pub tag: String,
    pub description: String,
    #[serde(rename = "hbISO", default)]
    pub hb_iso: Option<String>,
    #[serde(rename = "hasHb")]
    pub has_hb: bool,
}

pub enum ProfileEditorDraftStore {}

impl ProfileEditorDraftStore {
    const KEY_PREFIX: &'static str = PROFILE_EDITOR_DRAFT_PREFIX;

    fn key(user_id: &str) -> String {
        format!("{}{user_id}", Self::KEY_PREFIX)
    }

    pub fn load(user_id: &str) -> Option<ProfileEditorDraft> {
        if user_id.is_empty() {
            return None;
        }
        let data = UserDefaults::standard().data(&Self::key(user_id))?;
        serde_json::from_slice(&data).ok()
    }

    pub fn save(draft: &ProfileEditorDraft, user_id: &str) {
        if user_id.is_empty() {
            return;
        }
        let Ok(data) = serde_json::to_vec(draft) else {
            return;
        };
        UserDefaults::standard().set(&Self::key(user_id), data);
    }

    pub fn clear(user_id: &str) {
        if user_id.is_empty() {
            return;
        }
        UserDefaults::standard().remove_object(&Self::key(user_id));
    }
}
