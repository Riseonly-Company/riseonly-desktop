use std::future::Future;
use std::sync::OnceLock;

/// runs a d-bus future on the calling thread with tokio timers at hand
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("rise-dbus-runtime")
                .enable_time()
                .build()
                .expect("the d-bus tokio runtime starts")
        })
        .block_on(future)
}
