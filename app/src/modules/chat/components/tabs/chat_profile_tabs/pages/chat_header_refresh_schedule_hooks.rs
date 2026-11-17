use rise_core::foundation::foundation_date::TimeInterval;

pub enum ChatHeaderRefreshSchedule {}

impl ChatHeaderRefreshSchedule {
    const MINUTE: TimeInterval = 60.0;
    const HOUR: TimeInterval = 3_600.0;
    const DAY: TimeInterval = 86_400.0;
    const QUARTER_HOUR_STEP: TimeInterval = 15.0 * 60.0;
    const DAY_STEP: TimeInterval = 24.0 * 60.0 * 60.0;
    const BOUNDARY_SLACK: TimeInterval = 0.05;
    const MILLISECONDS_PER_SECOND: TimeInterval = 1_000.0;

    /// next time the header should refresh from typing times, online state and last seen
    pub fn next_refresh_timestamp(
        typing_timestamps: &[TimeInterval],
        typing_ttl: TimeInterval,
        is_online: bool,
        last_seen_raw: Option<i64>,
        now: TimeInterval,
        presence_now: Option<TimeInterval>,
    ) -> Option<TimeInterval> {
        let mut candidates: Vec<TimeInterval> = typing_timestamps
            .iter()
            .map(|timestamp| timestamp + typing_ttl)
            .filter(|expiry| *expiry > now)
            .collect();

        if !is_online
            && let Some(last_seen) = Self::normalized_timestamp(last_seen_raw)
            && last_seen > 0.0
        {
            let delta = (presence_now.unwrap_or(now) - last_seen).max(0.0);
            let step = match delta {
                delta if delta < Self::MINUTE => Self::MINUTE,
                delta if delta < Self::HOUR => Self::MINUTE,
                delta if delta < Self::DAY => Self::QUARTER_HOUR_STEP,
                _ => Self::DAY_STEP,
            };
            let until_boundary = ((delta / step).floor() + 1.0) * step - delta + Self::BOUNDARY_SLACK;
            candidates.push(now + until_boundary.max(Self::BOUNDARY_SLACK));
        }

        candidates.into_iter().reduce(TimeInterval::min)
    }

    /// last_seen arrives as epoch milliseconds and the schedule works in seconds
    pub fn normalized_timestamp(raw: Option<i64>) -> Option<TimeInterval> {
        raw.map(|raw| raw as TimeInterval / Self::MILLISECONDS_PER_SECOND)
    }
}
