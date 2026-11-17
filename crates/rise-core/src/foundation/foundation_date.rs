use std::cell::RefCell;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const TIME_INTERVAL_SINCE_1970_AT_REFERENCE_DATE: f64 = 978_307_200.0;

const MILLISECONDS_PER_SECOND: f64 = 1_000.0;

/// seconds as a double
pub type TimeInterval = f64;

/// point in time as ms since 1970 utc
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Date {
    ms: i64,
}

impl Date {
    /// current time from the app clock
    pub fn now() -> Self {
        RiseClock::now()
    }

    /// date from ms since 1970, the wire unit
    pub const fn from_ms(ms: i64) -> Self {
        Self { ms }
    }

    /// ms since 1970
    pub const fn ms(&self) -> i64 {
        self.ms
    }

    /// date from seconds since 1970, rounded to the ms
    pub fn from_time_interval_since_1970(seconds: TimeInterval) -> Self {
        Self { ms: seconds_to_ms(seconds) }
    }

    pub fn from_time_interval_since_reference_date(seconds: TimeInterval) -> Self {
        Self::from_time_interval_since_1970(seconds + TIME_INTERVAL_SINCE_1970_AT_REFERENCE_DATE)
    }

    pub fn from_time_interval_since_now(seconds: TimeInterval) -> Self {
        RiseClock::now().adding_time_interval(seconds)
    }

    /// the far past, year 1
    pub const fn distant_past() -> Self {
        Self { ms: -62_135_769_600_000 }
    }

    pub const fn distant_future() -> Self {
        Self { ms: 64_092_211_200_000 }
    }

    /// seconds since 1970
    pub fn time_interval_since_1970(&self) -> TimeInterval {
        self.ms as f64 / MILLISECONDS_PER_SECOND
    }

    pub fn time_interval_since_reference_date(&self) -> TimeInterval {
        self.time_interval_since_1970() - TIME_INTERVAL_SINCE_1970_AT_REFERENCE_DATE
    }

    /// seconds from other to self, positive when self is later
    pub fn time_interval_since(&self, other: &Date) -> TimeInterval {
        self.ms.saturating_sub(other.ms) as f64 / MILLISECONDS_PER_SECOND
    }

    /// seconds from now, negative for past dates
    pub fn time_interval_since_now(&self) -> TimeInterval {
        self.time_interval_since(&RiseClock::now())
    }

    /// date plus some seconds
    pub fn adding_time_interval(&self, seconds: TimeInterval) -> Self {
        Self { ms: self.ms.saturating_add(seconds_to_ms(seconds)) }
    }

    /// date plus an exact duration, for timers in whole units
    pub fn adding_duration(&self, duration: Duration) -> Self {
        let delta = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
        Self { ms: self.ms.saturating_add(delta) }
    }
}

fn seconds_to_ms(seconds: TimeInterval) -> i64 {
    (seconds * MILLISECONDS_PER_SECOND).round() as i64
}

/// app clock, wall time for dates and monotonic time for animation and timeouts
pub struct RiseClock;

static MONOTONIC_ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);

thread_local! {
    static TEST_CLOCK: RefCell<Option<TestClock>> = const { RefCell::new(None) };
}

impl RiseClock {
    /// wall clock time, or the test clock if one is installed on this thread
    pub fn now() -> Date {
        if let Some(clock) = Self::installed() {
            return clock.now();
        }
        let ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since) => i64::try_from(since.as_millis()).unwrap_or(i64::MAX),
            Err(before) => i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
        };
        Date::from_ms(ms)
    }

    /// monotonic time since some process start, never goes back when the wall clock changes
    pub fn monotonic() -> Duration {
        if let Some(clock) = Self::installed() {
            return clock.monotonic();
        }
        MONOTONIC_ORIGIN.elapsed()
    }

    /// swaps both clocks on this thread until the guard drops, tests only
    /// gpui test executors run on the test thread so stores see this clock
    pub fn install_test(clock: TestClock) -> TestClockGuard {
        let previous = TEST_CLOCK.with(|slot| slot.replace(Some(clock)));
        TestClockGuard { previous }
    }

    fn installed() -> Option<TestClock> {
        TEST_CLOCK.with(|slot| slot.borrow().clone())
    }
}

/// puts the old clock back on drop
#[must_use = "the test clock is removed as soon as the guard drops"]
pub struct TestClockGuard {
    previous: Option<TestClock>,
}

impl Drop for TestClockGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        TEST_CLOCK.with(|slot| *slot.borrow_mut() = previous);
    }
}

#[derive(Debug)]
struct TestClockState {
    now: Date,
    monotonic: Duration,
}

/// clock you drive by hand in tests, clones share one time
#[derive(Clone, Debug)]
pub struct TestClock {
    state: Arc<Mutex<TestClockState>>,
}

impl TestClock {
    /// clock at now with monotonic time zero
    pub fn new(now: Date) -> Self {
        Self { state: Arc::new(Mutex::new(TestClockState { now, monotonic: Duration::ZERO })) }
    }

    /// current wall time of this clock
    pub fn now(&self) -> Date {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).now
    }

    /// current monotonic time of this clock
    pub fn monotonic(&self) -> Duration {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).monotonic
    }

    /// moves wall and monotonic time forward together
    pub fn advance(&self, by: Duration) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.now = state.now.adding_duration(by);
        state.monotonic = state.monotonic.saturating_add(by);
    }

    pub fn set_now(&self, now: Date) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).now = now;
    }
}
