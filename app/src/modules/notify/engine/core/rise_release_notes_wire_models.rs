use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde::{Deserialize, Serialize};

pub type RiseReleaseNoteDTO = rise_canon::rise_proto::notify::ProtoAppRelease;
pub type RiseReleaseSlideDTO = rise_canon::rise_proto::notify::ProtoAppReleaseSlide;

/// engine model separate from the wire dto since it is written to the encrypted store
/// a later wire rename must not invalidate cached releases
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseReleaseNote {
    pub id: i64,
    pub version: String,
    pub is_blocking: bool,
    pub locale: String,
    pub title: String,
    pub published_at: i64,
    pub slides: Vec<RiseReleaseSlide>,
}

impl RiseReleaseNote {
    pub fn from_dto(dto: &RiseReleaseNoteDTO) -> Self {
        let mut ordered: Vec<&RiseReleaseSlideDTO> = dto.slides.iter().collect();
        ordered.sort_by_key(|slide| (slide.position, slide.id));
        Self {
            id: dto.id,
            version: dto.version.clone(),
            is_blocking: dto.is_blocking,
            locale: dto.locale.clone(),
            title: dto.title.clone(),
            published_at: dto.published_at,
            slides: ordered.into_iter().map(RiseReleaseSlide::from_dto).collect(),
        }
    }

    pub fn new(
        id: i64,
        version: String,
        is_blocking: bool,
        locale: String,
        title: String,
        published_at: i64,
        slides: Vec<RiseReleaseSlide>,
    ) -> Self {
        Self { id, version, is_blocking, locale, title, published_at, slides }
    }

    /// a release with no slides has nothing to show
    /// server skips these too, this is the client half
    pub fn is_presentable(&self) -> bool {
        self.id > 0 && !self.slides.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseReleaseSlide {
    pub id: i64,
    pub position: i64,
    #[serde(rename = "mediaURL")]
    pub media_url: String,
    pub media_kind: String,
    pub title: String,
    pub body: String,
    pub action_title: String,
    #[serde(rename = "actionURL")]
    pub action_url: String,
}

impl RiseReleaseSlide {
    pub fn from_dto(dto: &RiseReleaseSlideDTO) -> Self {
        Self {
            id: dto.id,
            position: dto.position,
            media_url: dto.media_url.clone(),
            media_kind: if dto.media_kind.is_empty() { "image".to_owned() } else { dto.media_kind.clone() },
            title: dto.title.clone(),
            body: dto.body.clone(),
            action_title: dto.action_title.clone(),
            action_url: dto.action_url.clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: i64,
        position: i64,
        media_url: String,
        media_kind: String,
        title: String,
        body: String,
        action_title: String,
        action_url: String,
    ) -> Self {
        Self { id, position, media_url, media_kind, title, body, action_title, action_url }
    }

    pub fn has_action(&self) -> bool {
        !trimming_whitespaces_and_newlines(&self.action_title).is_empty()
            && !trimming_whitespaces_and_newlines(&self.action_url).is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseReleaseAckOutcome {
    Shown,
    Dismissed,
    Action,
}

impl RiseReleaseAckOutcome {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::Dismissed => "dismissed",
            Self::Action => "action",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        [Self::Shown, Self::Dismissed, Self::Action].into_iter().find(|outcome| outcome.raw_value() == raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseReleaseNotesError {
    #[error("missingActiveSession")]
    MissingActiveSession,
    #[error("staleAccount")]
    StaleAccount,
}
