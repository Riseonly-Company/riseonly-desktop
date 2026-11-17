use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use futures::future::{Either, select};
use gpui::{App, Global, Task};
use parking_lot::Mutex;
use rise_canon::RiseTimingCanon;
use rise_platform::HostOs;
use rise_platform::gpui_shim::hides_other_apps_on;
use rise_platform::quit_request::on_quit_request;

use crate::core::widgets::notifier::notifier_store::Notifier;

use super::rise_app_lifecycle::{LifecycleSignal, RiseAppLifecycle};

#[derive(Default)]
struct QuitLedger {
    active: Vec<(u64, &'static str)>,
    next_id: u64,
    released: Option<oneshot::Sender<()>>,
}

impl QuitLedger {
    fn begin(&mut self, name: &'static str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.active.push((id, name));
        id
    }

    fn end(&mut self, id: u64) -> Option<oneshot::Sender<()>> {
        self.active.retain(|(active_id, _)| *active_id != id);
        if self.active.is_empty() { self.released.take() } else { None }
    }
}

/// holds the exit back until dropped or until the drain limit passes
#[must_use = "the exit stops waiting for the work when the lease is dropped"]
pub struct QuitLease {
    ledger: Arc<Mutex<QuitLedger>>,
    id: u64,
}

impl Drop for QuitLease {
    fn drop(&mut self) {
        let released = self.ledger.lock().end(self.id);
        if let Some(released) = released {
            let _ = released.send(());
        }
    }
}

struct QuitActions {
    hide_windows: Rc<dyn Fn(&mut App)>,
    exit: Rc<dyn Fn(&mut App)>,
}

impl Default for QuitActions {
    fn default() -> Self {
        Self {
            hide_windows: Rc::new(hide_windows),
            exit: Rc::new(|cx: &mut App| cx.quit()),
        }
    }
}

/// the active leases and the one exit in progress
#[derive(Default)]
pub struct QuitDrain {
    ledger: Arc<Mutex<QuitLedger>>,
    requested: bool,
    actions: QuitActions,
    wait: Option<Task<()>>,
}

impl Global for QuitDrain {}

impl QuitDrain {
    /// keep the lease while the work runs, name only labels the log at the limit
    pub fn lease(name: &'static str, cx: &mut App) -> QuitLease {
        let ledger = Arc::clone(&cx.default_global::<Self>().ledger);
        let id = ledger.lock().begin(name);
        QuitLease { ledger, id }
    }
}

/// the one way the app exits by itself, a repeated request joins the exit in progress
pub fn request_quit(cx: &mut App) {
    cx.defer(run_request);
}

/// sends the quits the os shell asks for through request_quit, call after launch
pub fn install_system_requests(cx: &mut App) {
    let (sender, requests) = mpsc::unbounded();
    let support = on_quit_request(move || {
        let _ = sender.unbounded_send(());
    });
    if !support.is_supported() {
        tracing::debug!(target: "riseonly", "this host's shell has no quit request");
        return;
    }
    forward_requests(requests, cx);
}

fn forward_requests(mut requests: mpsc::UnboundedReceiver<()>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while requests.next().await.is_some() {
            cx.update(request_quit);
        }
    })
    .detach();
}

fn run_request(cx: &mut App) {
    let drain = cx.default_global::<QuitDrain>();
    if drain.requested {
        return;
    }
    drain.requested = true;
    let ledger = Arc::clone(&drain.ledger);

    RiseAppLifecycle::apply(LifecycleSignal::Quit, cx);
    Notifier::commit_pending_undo_now(cx);

    if ledger.lock().active.is_empty() {
        exit(cx);
        return;
    }
    let hide = Rc::clone(&cx.global::<QuitDrain>().actions.hide_windows);
    hide(cx);
    let limit = cx.background_executor().timer(drain_limit());
    let wait = cx.spawn(async move |cx| {
        wait_for_release(ledger, limit).await;
        cx.update(exit);
    });
    cx.global_mut::<QuitDrain>().wait = Some(wait);
}

async fn wait_for_release(ledger: Arc<Mutex<QuitLedger>>, limit: impl Future<Output = ()>) {
    let mut limit = Box::pin(limit);
    loop {
        let released = {
            let mut ledger = ledger.lock();
            if ledger.active.is_empty() {
                return;
            }
            let (sender, receiver) = oneshot::channel();
            ledger.released = Some(sender);
            receiver
        };
        match select(released, limit).await {
            Either::Left((_, remaining)) => limit = remaining,
            Either::Right(_) => {
                let names: Vec<&'static str> =
                    ledger.lock().active.iter().map(|(_, name)| *name).collect();
                tracing::warn!(target: "riseonly", ?names, "quit drain limit passed");
                return;
            }
        }
    }
}

fn exit(cx: &mut App) {
    let exit = Rc::clone(&cx.global::<QuitDrain>().actions.exit);
    exit(cx);
}

fn drain_limit() -> Duration {
    Duration::from_millis(RiseTimingCanon::DESKTOP_QUIT_DRAIN_MS as u64)
}

fn hide_windows(cx: &mut App) {
    if hides_other_apps_on(HostOs::current()) {
        cx.hide();
        return;
    }
    for handle in cx.windows() {
        handle.update(cx, |_, window, _| window.minimize_window()).ok();
    }
}

#[cfg(test)]
#[path = "rise_app_quit_drain_tests.rs"]
mod tests;
