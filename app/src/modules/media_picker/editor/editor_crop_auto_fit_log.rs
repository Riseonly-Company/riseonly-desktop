use std::fmt::Debug;

pub enum EditorCropAutoFitLog {}

impl EditorCropAutoFitLog {
    pub fn is_enabled() -> bool {
        cfg!(debug_assertions)
    }

    /// fields are printed as key=value
    /// sorted
    /// after the event name
    pub fn log(event: &str, fields: &[(&str, &dyn Debug)]) {
        if !Self::is_enabled() {
            return;
        }
        let mut pairs: Vec<String> = fields.iter().map(|(key, value)| format!("{key}={value:?}")).collect();
        pairs.sort();
        let tail = pairs.join(" ");
        if tail.is_empty() {
            tracing::debug!(target: "CropAutoFit", "[CropAutoFit] {event}");
        } else {
            tracing::debug!(target: "CropAutoFit", "[CropAutoFit] {event} | {tail}");
        }
    }
}
