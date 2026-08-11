use std::future::Future;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// Runs work once a burst of edits has stopped, and never once per edit.
///
/// Trailing: the work runs after `window` of quiet, not on the first edit.
/// [`Debouncer::with_max_wait`] adds a ceiling so a steadily typing user still
/// gets an answer. A new edit cancels the pending run rather than adding a second
/// one, and a dropped `Debouncer` leaves nothing running. Answers are NOT
/// deduplicated — order a late reply against current input with your own
/// generation check.
///
/// ```ignore
/// // In the actor, on every keystroke:
/// self.tag_debounce.send(&self.commands, AuthCommand::CheckTagNow { tag });
/// ```
pub struct Debouncer {
    window: Duration,
    max_wait: Option<Duration>,
    pending: Option<Pending>,
    burst_started: Option<Instant>,
}

struct Pending {
    task: JoinHandle<()>,
    // Held, not sent: dropping this closes the channel and the task takes its cancel arm.
    fire_now: oneshot::Sender<()>,
}

impl Debouncer {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            max_wait: None,
            pending: None,
            burst_started: None,
        }
    }

    /// Fire regardless once a burst has been going this long.
    ///
    /// Silently ignored when shorter than the window: that would be a throttle.
    pub fn with_max_wait(mut self, max_wait: Duration) -> Self {
        if max_wait >= self.window {
            self.max_wait = Some(max_wait);
        }
        self
    }

    pub fn window(&self) -> Duration {
        self.window
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Delivers `message` into `channel` once the edits stop.
    pub fn send<M>(&mut self, channel: &mpsc::Sender<M>, message: M)
    where
        M: Send + 'static,
    {
        let channel = channel.clone();
        self.run(async move {
            let _ = channel.send(message).await;
        });
    }

    /// The general form, for work that is not a message.
    pub fn run<F>(&mut self, work: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let now = Instant::now();
        let burst_started = *self.burst_started.get_or_insert(now);

        self.cancel_pending();

        let deadline = match self.max_wait {
            Some(max_wait) => (now + self.window).min(burst_started + max_wait),
            None => now + self.window,
        };

        let (fire_now, fired) = oneshot::channel();
        let task = tokio::spawn(async move {
            let run = tokio::select! {
                _ = tokio::time::sleep_until(deadline) => true,
                signal = fired => signal.is_ok(),
            };
            if run {
                work.await;
            }
        });

        self.pending = Some(Pending { task, fire_now });
    }

    /// Runs whatever is pending immediately.
    pub fn flush(&mut self) {
        self.burst_started = None;
        if let Some(pending) = self.pending.take() {
            let _ = pending.fire_now.send(());
        }
    }

    /// Throws away whatever is pending; the next edit starts a fresh `max_wait`.
    pub fn cancel(&mut self) {
        self.burst_started = None;
        self.cancel_pending();
    }

    fn cancel_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.task.abort();
        }
    }
}

impl Drop for Debouncer {
    fn drop(&mut self) {
        self.cancel_pending();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counter() -> (Arc<AtomicUsize>, impl Fn() -> usize) {
        let count = Arc::new(AtomicUsize::new(0));
        let read = {
            let count = Arc::clone(&count);
            move || count.load(Ordering::SeqCst)
        };
        (count, read)
    }

    fn bump(count: &Arc<AtomicUsize>) -> impl Future<Output = ()> + Send + 'static {
        let count = Arc::clone(count);
        async move {
            count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_edits_produces_exactly_one_run() {
        let (count, read) = counter();
        let mut debouncer = Debouncer::new(Duration::from_millis(300));

        for _ in 0..10 {
            debouncer.run(bump(&count));
            tokio::time::advance(Duration::from_millis(50)).await;
        }
        assert_eq!(read(), 0, "nothing may run while the user is still typing");

        tokio::time::advance(Duration::from_millis(400)).await;
        tokio::task::yield_now().await;
        assert_eq!(read(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_work_runs_after_the_window_of_quiet() {
        let (count, read) = counter();
        let mut debouncer = Debouncer::new(Duration::from_millis(300));

        debouncer.run(bump(&count));
        tokio::time::advance(Duration::from_millis(299)).await;
        tokio::task::yield_now().await;
        assert_eq!(read(), 0);

        tokio::time::advance(Duration::from_millis(2)).await;
        tokio::task::yield_now().await;
        assert_eq!(read(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_ceiling_fires_during_an_endless_burst() {
        let (count, read) = counter();
        let mut debouncer =
            Debouncer::new(Duration::from_millis(300)).with_max_wait(Duration::from_millis(800));

        for _ in 0..20 {
            debouncer.run(bump(&count));
            tokio::time::advance(Duration::from_millis(100)).await;
            tokio::task::yield_now().await;
        }

        assert!(
            read() >= 1,
            "a two-second burst with an 800ms ceiling must have fired"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_ceiling_shorter_than_the_window_is_refused_rather_than_obeyed() {
        let debouncer =
            Debouncer::new(Duration::from_millis(300)).with_max_wait(Duration::from_millis(100));
        assert!(
            debouncer.max_wait.is_none(),
            "a ceiling under the window would make this a throttle"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn flush_runs_the_pending_work_at_once() {
        let (count, read) = counter();
        let mut debouncer = Debouncer::new(Duration::from_secs(30));

        debouncer.run(bump(&count));
        debouncer.flush();
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        assert_eq!(read(), 1);
        assert!(!debouncer.is_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn flushing_nothing_is_not_an_error_and_runs_nothing() {
        let mut debouncer = Debouncer::new(Duration::from_millis(300));
        debouncer.flush();
        assert!(!debouncer.is_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_means_the_work_never_runs() {
        let (count, read) = counter();
        let mut debouncer = Debouncer::new(Duration::from_millis(300));

        debouncer.run(bump(&count));
        debouncer.cancel();
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;

        assert_eq!(read(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_debouncer_cancels_what_it_owed() {
        let (count, read) = counter();
        {
            let mut debouncer = Debouncer::new(Duration::from_millis(300));
            debouncer.run(bump(&count));
        }
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;

        assert_eq!(read(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn send_delivers_the_last_message_into_the_channel() {
        let (tx, mut rx) = mpsc::channel::<&'static str>(8);
        let mut debouncer = Debouncer::new(Duration::from_millis(200));

        debouncer.send(&tx, "first");
        tokio::time::advance(Duration::from_millis(50)).await;
        debouncer.send(&tx, "second");

        tokio::time::advance(Duration::from_millis(300)).await;
        tokio::task::yield_now().await;

        assert_eq!(rx.try_recv().ok(), Some("second"));
        assert!(
            rx.try_recv().is_err(),
            "the superseded message must never arrive"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_burst_after_quiet_gets_a_fresh_ceiling() {
        let (count, read) = counter();
        let mut debouncer =
            Debouncer::new(Duration::from_millis(200)).with_max_wait(Duration::from_millis(500));

        debouncer.run(bump(&count));
        tokio::time::advance(Duration::from_millis(300)).await;
        tokio::task::yield_now().await;
        assert_eq!(read(), 1);

        debouncer.cancel();
        debouncer.run(bump(&count));
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
        assert_eq!(read(), 1, "the fresh burst is still inside its own window");

        tokio::time::advance(Duration::from_millis(200)).await;
        tokio::task::yield_now().await;
        assert_eq!(read(), 2);
    }
}
