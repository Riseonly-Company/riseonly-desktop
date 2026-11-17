use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, AsyncApp, BackgroundExecutor, Context, Task};

pub const MINIMUM_TIME_INTERVAL: Duration = Duration::from_micros(100);

/// scheduled timer, keep it in a field since dropping it invalidates the timer
pub struct RepeatingTimer {
    task: Option<Task<()>>,
    valid: Rc<Cell<bool>>,
    time_interval: Duration,
    tolerance: Duration,
}

impl RepeatingTimer {
    /// timer inside a store or view, block gets the entity and the timer stops once it is released
    pub fn scheduled_timer<T: 'static>(
        time_interval: Duration,
        repeats: bool,
        cx: &mut Context<T>,
        mut block: impl FnMut(&mut T, &mut Context<T>) + 'static,
    ) -> Self {
        let interval = effective_interval(time_interval);
        let valid = Rc::new(Cell::new(true));
        let alive = valid.clone();
        let executor = cx.background_executor().clone();
        let first_fire = executor.now() + interval;
        let task = cx.spawn(async move |this, cx| {
            run_fires(&executor, first_fire, interval, repeats, &alive, || {
                this.update(cx, |entity, cx| block(entity, cx)).is_ok()
            })
            .await;
        });
        Self::from_task(task, valid, time_interval)
    }

    /// same timer for an owner that isnt an entity, like a shared ticker
    pub fn scheduled_timer_in_app(
        time_interval: Duration,
        repeats: bool,
        cx: &mut App,
        mut block: impl FnMut(&mut App) + 'static,
    ) -> Self {
        let interval = effective_interval(time_interval);
        let valid = Rc::new(Cell::new(true));
        let alive = valid.clone();
        let executor = cx.background_executor().clone();
        let first_fire = executor.now() + interval;
        let task = cx.spawn(async move |cx: &mut AsyncApp| {
            run_fires(&executor, first_fire, interval, repeats, &alive, || {
                cx.update(|cx| block(cx));
                true
            })
            .await;
        });
        Self::from_task(task, valid, time_interval)
    }

    fn from_task(task: Task<()>, valid: Rc<Cell<bool>>, time_interval: Duration) -> Self {
        Self {
            task: Some(task),
            valid,
            time_interval,
            tolerance: Duration::ZERO,
        }
    }

    /// block never runs again, safe to call from inside the block
    pub fn invalidate(&mut self) {
        self.valid.set(false);
        self.task = None;
    }

    /// false after invalidate, after a one shot timer fired and after the owner entity was released
    pub fn is_valid(&self) -> bool {
        self.task.is_some() && self.valid.get()
    }

    pub fn time_interval(&self) -> Duration {
        self.time_interval
    }

    pub fn tolerance(&self) -> Duration {
        self.tolerance
    }

    /// sets tolerance, only for parity since gpui timers dont coalesce so exact fire is always inside it
    pub fn set_tolerance(&mut self, tolerance: Duration) {
        self.tolerance = tolerance;
    }
}

impl Drop for RepeatingTimer {
    fn drop(&mut self) {
        self.invalidate();
    }
}

async fn run_fires(
    executor: &BackgroundExecutor,
    first_fire: Instant,
    interval: Duration,
    repeats: bool,
    valid: &Cell<bool>,
    mut fire: impl FnMut() -> bool,
) {
    let mut fire_date = first_fire;
    loop {
        let now = executor.now();
        if fire_date > now {
            executor.timer(fire_date - now).await;
        }
        if !valid.get() {
            return;
        }
        let owner_alive = fire();
        if !repeats || !owner_alive {
            valid.set(false);
            return;
        }
        fire_date = next_fire_date(fire_date, interval, executor.now());
    }
}

/// first grid point after previous that isnt in the past at now
pub(crate) fn next_fire_date(previous: Instant, interval: Duration, now: Instant) -> Instant {
    let candidate = previous + interval;
    if candidate >= now {
        return candidate;
    }
    let elapsed = now.saturating_duration_since(previous).as_nanos();
    let step = interval.as_nanos().max(1);
    let steps = elapsed.div_ceil(step);
    let offset = steps.saturating_mul(step);
    previous + Duration::from_nanos(u64::try_from(offset).unwrap_or(u64::MAX))
}

pub(crate) fn effective_interval(time_interval: Duration) -> Duration {
    time_interval.max(MINIMUM_TIME_INTERVAL)
}
