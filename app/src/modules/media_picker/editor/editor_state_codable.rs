use gpui::{Hsla, Rgba};
use serde::{Deserialize, Serialize};

use crate::modules::media_picker::editor::editor_state::{
    EditorImageState, EditorTextEntityStyleMode, EditorTextEntityTextAlignment, EditorVideoState,
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RGBAColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl RGBAColor {
    pub fn new(color: Hsla) -> Self {
        let rgba = color.to_rgb();
        Self { r: rgba.r, g: rgba.g, b: rgba.b, a: rgba.a }
    }

    pub fn ui_color(&self) -> Hsla {
        Hsla::from(Rgba { r: self.r, g: self.g, b: self.b, a: self.a })
    }
}

pub mod rgba_color {
    use gpui::Hsla;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::RGBAColor;

    pub fn serialize<S: Serializer>(color: &Hsla, serializer: S) -> Result<S::Ok, S::Error> {
        RGBAColor::new(*color).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Hsla, D::Error> {
        RGBAColor::deserialize(deserializer).map(|color| color.ui_color())
    }
}

impl From<EditorTextEntityStyleMode> for i64 {
    fn from(value: EditorTextEntityStyleMode) -> i64 {
        match value {
            EditorTextEntityStyleMode::Background => 0,
            EditorTextEntityStyleMode::TranslucentBg => 1,
            EditorTextEntityStyleMode::TextOnly => 2,
        }
    }
}

impl TryFrom<i64> for EditorTextEntityStyleMode {
    type Error = String;

    fn try_from(raw_value: i64) -> Result<Self, Self::Error> {
        match raw_value {
            0 => Ok(Self::Background),
            1 => Ok(Self::TranslucentBg),
            2 => Ok(Self::TextOnly),
            other => Err(format!("unknown EditorTextEntity.StyleMode raw value {other}")),
        }
    }
}

impl From<EditorTextEntityTextAlignment> for i64 {
    fn from(value: EditorTextEntityTextAlignment) -> i64 {
        match value {
            EditorTextEntityTextAlignment::Leading => 0,
            EditorTextEntityTextAlignment::Center => 1,
            EditorTextEntityTextAlignment::Trailing => 2,
        }
    }
}

impl TryFrom<i64> for EditorTextEntityTextAlignment {
    type Error = String;

    fn try_from(raw_value: i64) -> Result<Self, Self::Error> {
        match raw_value {
            0 => Ok(Self::Leading),
            1 => Ok(Self::Center),
            2 => Ok(Self::Trailing),
            other => Err(format!("unknown EditorTextEntity.TextAlignment raw value {other}")),
        }
    }
}

pub enum EditorStateCodec {}

#[derive(Serialize, Deserialize)]
struct Envelope<Payload> {
    version: i64,
    kind: String,
    payload: Payload,
}

impl EditorStateCodec {
    const CURRENT_VERSION: i64 = 1;
    const IMAGE_KIND: &'static str = "image";
    const VIDEO_KIND: &'static str = "video";

    pub fn encode_image(state: &EditorImageState) -> Option<Vec<u8>> {
        serde_json::to_vec(&Envelope {
            version: Self::CURRENT_VERSION,
            kind: Self::IMAGE_KIND.to_string(),
            payload: state,
        })
        .ok()
    }

    pub fn decode_image(data: &[u8]) -> Option<EditorImageState> {
        if let Ok(envelope) = serde_json::from_slice::<Envelope<EditorImageState>>(data) {
            if envelope.kind != Self::IMAGE_KIND || envelope.version > Self::CURRENT_VERSION {
                return None;
            }
            return Some(envelope.payload);
        }
        serde_json::from_slice::<EditorImageState>(data).ok()
    }

    pub fn encode_video(state: &EditorVideoState) -> Option<Vec<u8>> {
        serde_json::to_vec(&Envelope {
            version: Self::CURRENT_VERSION,
            kind: Self::VIDEO_KIND.to_string(),
            payload: state,
        })
        .ok()
    }

    pub fn decode_video(data: &[u8]) -> Option<EditorVideoState> {
        if let Ok(envelope) = serde_json::from_slice::<Envelope<EditorVideoState>>(data) {
            if envelope.kind != Self::VIDEO_KIND || envelope.version > Self::CURRENT_VERSION {
                return None;
            }
            return Some(envelope.payload);
        }
        serde_json::from_slice::<EditorVideoState>(data).ok()
    }
}
