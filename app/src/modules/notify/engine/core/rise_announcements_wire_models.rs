use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde::{Deserialize, Serialize};

pub type RiseAnnouncementDTO = rise_canon::rise_proto::notify::ProtoAnnouncement;
pub type RiseAnnouncementSlideDTO = rise_canon::rise_proto::notify::ProtoAnnouncementSlide;

/// engine model separate from the wire dto since it is written to the encrypted store
/// a later wire rename must not invalidate cached announcements
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseAnnouncement {
    pub id: i64,
    pub locale: String,
    pub title: String,
    pub body: String,
    pub sent_at: i64,
    pub slides: Vec<RiseAnnouncementSlide>,
}

impl RiseAnnouncement {
    pub fn from_dto(dto: &RiseAnnouncementDTO) -> Self {
        let mut ordered: Vec<&RiseAnnouncementSlideDTO> = dto.slides.iter().collect();
        ordered.sort_by_key(|slide| (slide.position, slide.id));
        Self {
            id: dto.id,
            locale: dto.locale.clone(),
            title: dto.title.clone(),
            body: dto.body.clone(),
            sent_at: dto.sent_at,
            slides: ordered.into_iter().map(RiseAnnouncementSlide::from_dto).collect(),
        }
    }

    pub fn new(
        id: i64,
        locale: String,
        title: String,
        body: String,
        sent_at: i64,
        slides: Vec<RiseAnnouncementSlide>,
    ) -> Self {
        Self { id, locale, title, body, sent_at, slides }
    }

    /// an announcement with no slides is still showable, the modal builds one slide from title and body
    /// neither present means nothing to show
    pub fn is_presentable(&self) -> bool {
        if self.id <= 0 {
            return false;
        }
        if !self.slides.is_empty() {
            return true;
        }
        !trimming_whitespaces_and_newlines(&self.title).is_empty()
            || !trimming_whitespaces_and_newlines(&self.body).is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseAnnouncementSlide {
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

impl RiseAnnouncementSlide {
    pub fn from_dto(dto: &RiseAnnouncementSlideDTO) -> Self {
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
pub enum RiseAnnouncementAckOutcome {
    Shown,
    Dismissed,
    Action,
}

impl RiseAnnouncementAckOutcome {
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
