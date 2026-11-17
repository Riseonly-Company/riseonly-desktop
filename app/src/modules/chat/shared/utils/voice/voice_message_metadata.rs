#[derive(Clone, Debug, PartialEq)]
pub struct VoiceMessageMetadata {
    pub waveform: Vec<f32>,
    pub duration_seconds: i64,
    pub duration_milliseconds: i64,
}
