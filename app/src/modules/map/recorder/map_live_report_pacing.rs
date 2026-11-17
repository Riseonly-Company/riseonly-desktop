#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MapLiveReportPacingDecision {
    Send,
    Wait { until_ms: i64 },
    Busy,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MapLiveReportPacing {
    pub watched_interval_ms: i64,
    pub unwatched_interval_ms: i64,
    pub minimum_interval_ms: i64,
    pub deadline_ms: i64,

    next_report_at_ms: i64,
    last_sent_at_ms: Option<i64>,
    reported_moving: Option<bool>,
    failures: i64,
    in_flight_since_ms: Option<i64>,
    retries_at_once: bool,
}

impl MapLiveReportPacing {
    pub fn new(watched_interval_ms: i64, unwatched_interval_ms: i64, minimum_interval_ms: i64, deadline_ms: i64) -> Self {
        Self {
            watched_interval_ms,
            unwatched_interval_ms,
            minimum_interval_ms,
            deadline_ms,
            next_report_at_ms: 0,
            last_sent_at_ms: None,
            reported_moving: None,
            failures: 0,
            in_flight_since_ms: None,
            retries_at_once: false,
        }
    }

    pub fn next_report_at_ms(&self) -> i64 {
        self.next_report_at_ms
    }

    pub fn last_sent_at_ms(&self) -> Option<i64> {
        self.last_sent_at_ms
    }

    pub fn reported_moving(&self) -> Option<bool> {
        self.reported_moving
    }

    pub fn failures(&self) -> i64 {
        self.failures
    }

    pub fn in_flight_since_ms(&self) -> Option<i64> {
        self.in_flight_since_ms
    }

    pub fn decision(&self, moving: bool, now_ms: i64) -> MapLiveReportPacingDecision {
        if let Some(since) = self.in_flight_since_ms
            && now_ms - since <= self.deadline_ms
        {
            return MapLiveReportPacingDecision::Busy;
        }
        let mut due_at_ms = self.next_report_at_ms;
        if let (Some(reported_moving), Some(last_sent_at_ms)) = (self.reported_moving, self.last_sent_at_ms)
            && reported_moving != moving
        {
            due_at_ms = due_at_ms.min(last_sent_at_ms + self.minimum_interval_ms);
        }
        if now_ms >= due_at_ms {
            MapLiveReportPacingDecision::Send
        } else {
            MapLiveReportPacingDecision::Wait { until_ms: due_at_ms }
        }
    }

    pub fn sent(&mut self, now_ms: i64) {
        if self.in_flight_since_ms.is_some() {
            self.failures += 1;
        }
        self.in_flight_since_ms = Some(now_ms);
        self.last_sent_at_ms = Some(now_ms);
        self.retries_at_once = false;
    }

    /// server_interval_ms is the answer's cadence, none when the report failed
    /// demanded says a viewer is waiting on this owner which caps any wait at the watched interval
    pub fn finished(&mut self, moving: bool, server_interval_ms: Option<i64>, demanded: bool, now_ms: i64) {
        self.in_flight_since_ms = None;
        let mut wait_ms = match server_interval_ms {
            Some(server_interval_ms) => {
                self.failures = 0;
                self.reported_moving = Some(moving);
                server_interval_ms.max(0)
            }
            None => {
                self.failures += 1;
                if self.retries_at_once {
                    0
                } else {
                    let step = (self.failures - 1).clamp(0, 4);
                    self.unwatched_interval_ms.min(self.watched_interval_ms << step)
                }
            }
        };
        if demanded {
            wait_ms = wait_ms.min(self.watched_interval_ms);
        }
        self.retries_at_once = false;
        self.next_report_at_ms = now_ms + wait_ms;
    }

    /// a viewer's demand or the foreground
    /// next fix goes at once and a report still in flight is retried as soon as it fails
    pub fn resumed(&mut self, now_ms: i64) {
        self.next_report_at_ms = now_ms;
        self.retries_at_once = self.in_flight_since_ms.is_some();
    }
}
