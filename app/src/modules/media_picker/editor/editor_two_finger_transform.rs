use gpui::{
    AnyElement, App, Context, IntoElement, ParentElement, Render, RenderOnce, Styled, Window, div,
};
use rise_core::foundation::foundation_uuid::Uuid;

/// entity the canvas scales and turns with two fingers, eg photo or text
/// one finger moves it by its own pan
/// canvas owns pinch and rotation so fingers land anywhere
pub trait EditorTwoFingerTransformable: Render {
    fn entity_id(&self) -> Uuid;
    fn begin_two_finger_transform(&mut self, cx: &mut Context<Self>);
    fn apply_two_finger_transform(&mut self, scale: f32, rotation: f32, cx: &mut Context<Self>);
    fn end_two_finger_transform(&mut self, cx: &mut Context<Self>);
}

/// recognizer kinds for the two finger gesture
/// desktop pinch is the trackpad event, gpui has no trackpad rotation
/// so Rotation only comes from a canvas faking it with modifier plus wheel
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorTwoFingerRecognizer {
    Pinch,
    Rotation,
    Other,
}

pub enum EditorTwoFinger {}

impl EditorTwoFinger {
    pub fn is_two_finger_recognizer(recognizer: EditorTwoFingerRecognizer, on_overlay_canvas: bool) -> bool {
        matches!(recognizer, EditorTwoFingerRecognizer::Pinch | EditorTwoFingerRecognizer::Rotation)
            && on_overlay_canvas
    }
}

/// layer container that takes no pointer itself
/// a press that misses its children reaches whatever is under it
/// why: a gpui div without handlers has no hitbox so only its children get hit
#[derive(IntoElement, Default)]
pub struct EditorPassThroughView {
    children: Vec<AnyElement>,
}

impl EditorPassThroughView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ParentElement for EditorPassThroughView {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for EditorPassThroughView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().absolute().top_0().left_0().size_full().children(self.children)
    }
}
