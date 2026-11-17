use std::fmt;
use std::sync::LazyLock;

use parking_lot::Mutex;
use rise_canon::rise_rpc::{REQUEST_ID_MINIMUM, RequestIdAllocator, timestamp_seconds_of};
use rise_core::foundation::foundation_date::Date;
use serde::{Deserialize, Serialize};

/// number that identifies one request, the server dedups retries by it
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct RiseRequestID {
    raw_value: u64,
}

/// a decoded number below the minimum request id, refused instead of crashing
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
#[error("request id {0} is below the canon minimum")]
pub struct RiseRequestIDOutOfRange(pub u64);

impl RiseRequestID {
    /// makes a request id, none if the number is below the minimum
    /// so a value from disk or the wire never crashes the app
    pub fn new(raw_value: u64) -> Option<Self> {
        Self::try_from(raw_value).ok()
    }

    pub const fn raw_value(&self) -> u64 {
        self.raw_value
    }

    pub fn description(&self) -> String {
        self.to_string()
    }
}

impl TryFrom<u64> for RiseRequestID {
    type Error = RiseRequestIDOutOfRange;

    fn try_from(raw_value: u64) -> Result<Self, Self::Error> {
        if raw_value < REQUEST_ID_MINIMUM {
            return Err(RiseRequestIDOutOfRange(raw_value));
        }
        Ok(Self { raw_value })
    }
}

impl From<RiseRequestID> for u64 {
    fn from(id: RiseRequestID) -> Self {
        id.raw_value
    }
}

impl fmt::Display for RiseRequestID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw_value)
    }
}

static SHARED: LazyLock<RiseRequestIDAllocator> = LazyLock::new(RiseRequestIDAllocator::new);

/// issues request ids with the bit layout of rise-rpc, shared by every platform and the server
#[derive(Default)]
pub struct RiseRequestIDAllocator {
    allocator: Mutex<RequestIdAllocator>,
}

impl RiseRequestIDAllocator {
    pub fn shared() -> &'static RiseRequestIDAllocator {
        &SHARED
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// next request id, callers pass Date.now()
    /// a clock before 1970 counts as zero ms
    pub fn next(&self, now: Date) -> RiseRequestID {
        let requested_milliseconds = u64::try_from(now.ms()).unwrap_or(0);
        let raw_value = self.allocator.lock().next(requested_milliseconds);
        RiseRequestID { raw_value }
    }

    pub fn timestamp_seconds(request_id: RiseRequestID) -> i64 {
        i64::try_from(timestamp_seconds_of(request_id.raw_value())).unwrap_or(i64::MAX)
    }
}
