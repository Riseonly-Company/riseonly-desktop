use rise_catalog::StoryAudience as RiseStoryAudience;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_limits::limit::{STORY_IMAGE_DIMENSION, STORY_IMAGE_DURATION, STORY_PREFETCH, STORY_REWIND_THRESHOLD};

use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementSurface;
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryGroupDTO;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoryPlayerPresentationStyle {
    Standard,
    FeedCube,
}

pub enum StoryPlaybackPolicy {}

impl StoryPlaybackPolicy {
    pub const IMAGE_DURATION: f64 = STORY_IMAGE_DURATION as f64 / 1000.0;
    pub const REWIND_THRESHOLD: f64 = STORY_REWIND_THRESHOLD as f64 / 1000.0;
    pub const PREFETCH_COUNT: i64 = rise_limits::signed(STORY_PREFETCH);
    pub const IMAGE_MAXIMUM_PIXEL_DIMENSION: i64 = rise_limits::signed(STORY_IMAGE_DIMENSION);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoryPlayerEntrance {
    Slide,
    SharedElement { epoch: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoryPlayerSharedElementOrigin {
    pub surface: SharedElementSurface,
    pub item_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoryPlayerRequest {
    pub id: Uuid,
    pub groups: Vec<StoryGroupDTO>,
    pub start_group_index: usize,
    pub start_story_id: Option<i64>,
    pub presentation_style: StoryPlayerPresentationStyle,
    pub owns_explicit_stories: bool,
    pub entrance: StoryPlayerEntrance,
    pub shared_element_origin: Option<StoryPlayerSharedElementOrigin>,
    pub layout: Vec<StoryPlayerGroupLayout>,
}

impl StoryPlayerRequest {
    pub fn new(groups: Vec<StoryGroupDTO>, start_group_index: usize) -> Self {
        let layout = groups.iter().map(StoryPlayerGroupLayout::new).collect();
        Self {
            id: Uuid::new_v4(),
            groups,
            start_group_index,
            start_story_id: None,
            presentation_style: StoryPlayerPresentationStyle::Standard,
            owns_explicit_stories: false,
            entrance: StoryPlayerEntrance::Slide,
            shared_element_origin: None,
            layout,
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn with_start_story_id(mut self, start_story_id: Option<i64>) -> Self {
        self.start_story_id = start_story_id;
        self
    }

    pub fn with_presentation_style(mut self, presentation_style: StoryPlayerPresentationStyle) -> Self {
        self.presentation_style = presentation_style;
        self
    }

    pub fn with_owns_explicit_stories(mut self, owns_explicit_stories: bool) -> Self {
        self.owns_explicit_stories = owns_explicit_stories;
        self
    }

    pub fn with_entrance(mut self, entrance: StoryPlayerEntrance) -> Self {
        self.entrance = entrance;
        self
    }

    pub fn with_shared_element_origin(mut self, shared_element_origin: Option<StoryPlayerSharedElementOrigin>) -> Self {
        self.shared_element_origin = shared_element_origin;
        self
    }

    pub fn request_id(&self) -> String {
        self.id.uuid_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoryPlayerGroupLayout {
    pub author_id: String,
    pub story_ids: Vec<i64>,
}

impl StoryPlayerGroupLayout {
    pub fn new(group: &StoryGroupDTO) -> Self {
        let stable = group
            .stories
            .first()
            .map(|story| trimming_whitespaces_and_newlines(&story.author_id).to_owned())
            .unwrap_or_default();
        let nested = group
            .author
            .as_ref()
            .map(|author| trimming_whitespaces_and_newlines(&author.id).to_owned())
            .unwrap_or_default();
        Self {
            author_id: if stable.is_empty() { nested } else { stable },
            story_ids: group.stories.iter().map(|story| story.id).collect(),
        }
    }
}

pub type StoryAudience = RiseStoryAudience;

pub trait StoryAudienceExt {
    fn icon(&self) -> &'static str;
}

impl StoryAudienceExt for StoryAudience {
    fn icon(&self) -> &'static str {
        match self {
            StoryAudience::Everyone => "globe",
            StoryAudience::Contacts => "person.2.fill",
            StoryAudience::CloseFriends => "star.fill",
            StoryAudience::Selected => "checkmark.circle.fill",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoryPendingPublishPhase {
    Preparing,
    Uploading,
    Creating,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoryPendingPublish {
    pub story_id: i64,
    pub author_id: String,
    pub media_source: String,
    pub is_video: bool,
    pub phase: StoryPendingPublishPhase,
}

impl StoryPendingPublish {
    pub fn is_active(&self) -> bool {
        self.phase != StoryPendingPublishPhase::Failed
    }
}
