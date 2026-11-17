use rise_core::foundation::foundation_date::TimeInterval;

/// smooths upload byte counts between slab acks
/// socket takes whole slabs so raw counts jump in steps
/// follows throughput, never passes socket bytes
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseUploadByteEstimator {
    acknowledged_bytes: i64,
    acknowledged_at: TimeInterval,
    bytes_per_second: f64,
    shown_bytes: i64,
}

impl RiseUploadByteEstimator {
    const RATE_WEIGHT: f64 = 0.3;

    pub fn new(started_at: TimeInterval) -> Self {
        Self {
            acknowledged_bytes: 0,
            acknowledged_at: started_at,
            bytes_per_second: 0.0,
            shown_bytes: 0,
        }
    }

    pub fn acknowledge(&mut self, total_acknowledged: i64, at: TimeInterval) {
        if total_acknowledged <= self.acknowledged_bytes {
            return;
        }
        let elapsed = at - self.acknowledged_at;
        if elapsed > 0.0 {
            let sample = (total_acknowledged - self.acknowledged_bytes) as f64 / elapsed;
            self.bytes_per_second = if self.bytes_per_second > 0.0 {
                self.bytes_per_second + (sample - self.bytes_per_second) * Self::RATE_WEIGHT
            } else {
                sample
            };
        }
        self.acknowledged_bytes = total_acknowledged;
        self.acknowledged_at = at;
    }

    /// sent_bytes is what the socket took, before the first ack there is no rate
    /// so the raw count is shown
    pub fn estimate(&mut self, sent_bytes: i64, at: TimeInterval) -> i64 {
        let ceiling = self.acknowledged_bytes.max(sent_bytes);
        let projected = if self.bytes_per_second > 0.0 {
            self.acknowledged_bytes
                + (self.bytes_per_second * (at - self.acknowledged_at).max(0.0)) as i64
        } else {
            ceiling
        };
        self.shown_bytes = self.shown_bytes.max(ceiling.min(projected));
        self.shown_bytes
    }
}
