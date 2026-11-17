use rise_core::foundation::foundation_date::Date;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MessageLocationInfo {
    pub latitude: f64,
    pub longitude: f64,
    pub title: Option<String>,
    pub address: Option<String>,
    pub live_period: Option<f64>,
    pub live_until: Option<Date>,
}

impl MessageLocationInfo {
    pub fn live(&self) -> bool {
        self.live_until.is_some()
    }

    pub fn live_remaining(&self, now: Date) -> Option<f64> {
        let live_until = self.live_until?;
        let left = live_until.time_interval_since(&now);
        (left > 0.0).then_some(left)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessagePollDraft {
    pub question: String,
    pub options: Vec<String>,
    pub is_anonymous: bool,
    pub allows_multiple: bool,
    pub quiz_correct_index: Option<usize>,
}

impl Default for MessagePollDraft {
    fn default() -> Self {
        Self {
            question: String::new(),
            options: Vec::new(),
            is_anonymous: true,
            allows_multiple: false,
            quiz_correct_index: None,
        }
    }
}
