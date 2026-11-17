use gpui::{Bounds, Pixels, Point, Size};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementSurface {
    PostList { session: u64 },
    ProfileGrid { scope_key: String },
    ShortViewer { session: u64 },
    StoriesGrid { scope_key: String },
    StoryPlayer { request_id: String },
    ProfileHighlights { scope_key: String },
}

impl SharedElementSurface {
    /// token string, prefixes differ before the first dot so tokens of different cases never collide
    pub fn token(&self) -> String {
        match self {
            Self::PostList { session } => format!("list.{session}"),
            Self::ProfileGrid { scope_key } => format!("grid.{scope_key}"),
            Self::ShortViewer { session } => format!("viewer.{session}"),
            Self::StoriesGrid { scope_key } => format!("stories.{scope_key}"),
            Self::StoryPlayer { request_id } => format!("story.{request_id}"),
            Self::ProfileHighlights { scope_key } => format!("highlights.{scope_key}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementPart {
    Media,
    Text,
}

impl SharedElementPart {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Text => "text",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SharedElementKey {
    pub surface: SharedElementSurface,
    pub item_id: String,
    pub part: SharedElementPart,
}

impl SharedElementKey {
    pub fn new(surface: SharedElementSurface, item_id: impl Into<String>, part: SharedElementPart) -> Self {
        Self { surface, item_id: item_id.into(), part }
    }

    /// own namespace, never the post-image keys of the
    /// media viewer since those are shared with feed tabs
    pub fn frame_registry_key(&self) -> String {
        format!("se|{}|{}|{}", self.surface.token(), self.item_id, self.part.raw_value())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementDirection {
    Open,
    Exit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SharedElementContentMode {
    Fill,
    Fit { backdrop: bool },
    AnchoredWidthFit,
}

/// screen space geometry of one end of a flight
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementEndpoint {
    pub clip: Bounds<Pixels>,
    pub content: Bounds<Pixels>,
    pub corner_radius: f32,
    pub mode: SharedElementContentMode,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementPopPanMode {
    Stock,
    StockArmed,
    Flight,
    Crossfade,
    Ignore,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementExitStyle {
    Flight,
    Crossfade,
    StockWithFade,
    Stock,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementOpenStyle {
    Flight,
    Crossfade,
    Stock,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementReleaseOutcome {
    Finish,
    Cancel,
}

pub enum SharedElementGeometry {}

/// cubic bezier timing curve from zero to one
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementCubicTiming {
    pub c1: Point<f32>,
    pub c2: Point<f32>,
}

pub enum SharedElementCoverSchedule {}

pub enum SharedElementGesturePolicy {}

/// back swipe from the side or pull down from the top of a list
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SharedElementGesturePolicyAxis {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementGesturePolicyInput {
    pub translation: Point<Pixels>,
    pub velocity: Point<Pixels>,
    pub touch_start: Point<Pixels>,
    pub source: Bounds<Pixels>,
    pub viewport: Size<Pixels>,
    pub layout_sign: f32,
    pub axis: SharedElementGesturePolicyAxis,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementGesturePolicyFrame {
    pub clip: Bounds<Pixels>,
    pub scale: f32,
    pub chrome_alpha: f32,
    pub progress: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementGesturePolicyDecision {
    Finish,
    Cancel,
}

pub enum SharedElementReturnScrollMath {}

/// viewport local band the returning cell is centered in
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementReturnScrollMathBand {
    pub top: f32,
    pub bottom_inset: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementReturnScrollMathClamp {
    None,
    Top,
    Bottom,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementReturnScrollMathPosition {
    pub master_offset: f32,
    pub collapse_progress: f32,
    pub inner_offset: f32,
    pub clamp: SharedElementReturnScrollMathClamp,
}

pub enum SharedElementVisibilityPolicy {}

#[derive(Clone, PartialEq, Debug)]
pub struct SharedElementVisibilityPolicyCandidate {
    pub item_id: String,
    pub media_frame: Option<Bounds<Pixels>>,
    pub text_frame: Option<Bounds<Pixels>>,
}

pub enum SharedElementContentPolicy {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementContentPolicyVideoSource {
    StopFrame,
    Poster,
    GridCover,
    Replicant,
    FadeOnly,
}

pub enum SharedElementSessionPolicy {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementSessionPolicyPhase {
    Idle,
    Armed,
    Running,
    Settling,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementSessionPolicyLandingAction {
    Land,
    CrossfadeReveal,
}

pub enum ProfilePostListWindowPolicy {}
