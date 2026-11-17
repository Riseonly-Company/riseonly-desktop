use std::sync::OnceLock;

use futures::FutureExt;
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use futures::future::{BoxFuture, Either, LocalBoxFuture};
use gpui::{App, AsyncApp};
use rise_engine::{RiseEngineObservability, RiseWireError};

type RiseEngineMainJob = Box<dyn FnOnce(AsyncApp) -> LocalBoxFuture<'static, ()> + Send>;

#[derive(Clone)]
pub struct RiseEngineMainHop {
    jobs: mpsc::UnboundedSender<RiseEngineMainJob>,
}

static SHARED: OnceLock<RiseEngineMainHop> = OnceLock::new();

impl RiseEngineMainHop {
    /// process wide hop, the first call starts its foreground loop
    pub fn shared(cx: &mut App) -> RiseEngineMainHop {
        SHARED.get_or_init(|| Self::start(cx)).clone()
    }

    /// the hop once something on the main thread started it
    pub fn installed() -> Option<RiseEngineMainHop> {
        SHARED.get().cloned()
    }

    fn start(cx: &mut App) -> RiseEngineMainHop {
        let (jobs, mut receiver) = mpsc::unbounded::<RiseEngineMainJob>();
        cx.spawn(async move |cx| {
            while let Some(job) = receiver.next().await {
                let job_cx = cx.clone();
                cx.spawn(async move |_| job(job_cx).await).detach();
            }
        })
        .detach();
        RiseEngineMainHop { jobs }
    }

    /// runs work on the main thread, fire and forget
    pub fn post(&self, work: impl FnOnce(&mut App) + Send + 'static) {
        let job: RiseEngineMainJob = Box::new(move |cx: AsyncApp| {
            async move { cx.update(work) }.boxed_local()
        });
        let _ = self.jobs.unbounded_send(job);
    }

    /// awaits work on the main thread from engine code
    /// task locals like call context and request id ride along, main side reads them
    /// dropping the future abandons the main work
    pub fn run<R, W>(&self, work: W) -> BoxFuture<'static, anyhow::Result<R>>
    where
        R: Send + 'static,
        W: FnOnce(AsyncApp) -> LocalBoxFuture<'static, anyhow::Result<R>> + Send + 'static,
    {
        let call_context = RiseEngineObservability::call_context();
        let request_id = RiseEngineObservability::request_id();
        let (result_sender, result_receiver) = oneshot::channel::<anyhow::Result<R>>();
        let (cancel_sender, cancel_receiver) = oneshot::channel::<()>();
        let job: RiseEngineMainJob = Box::new(move |cx: AsyncApp| {
            async move {
                let mut scoped = work(cx);
                if let Some(request_id) = request_id {
                    scoped = RiseEngineObservability::with_request_id(request_id, scoped)
                        .boxed_local();
                }
                if let Some(call_context) = call_context {
                    scoped = RiseEngineObservability::with_call_context(call_context, scoped)
                        .boxed_local();
                }
                if let Either::Left((result, _)) =
                    futures::future::select(scoped, cancel_receiver).await
                {
                    let _ = result_sender.send(result);
                }
            }
            .boxed_local()
        });
        let queued = self.jobs.unbounded_send(job).is_ok();
        async move {
            let _cancel = cancel_sender;
            if !queued {
                return Err(anyhow::Error::new(RiseWireError::NotConnected));
            }
            match result_receiver.await {
                Ok(result) => result,
                Err(oneshot::Canceled) => Err(anyhow::Error::new(RiseWireError::NotConnected)),
            }
        }
        .boxed()
    }
}
