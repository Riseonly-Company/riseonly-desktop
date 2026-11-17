use std::time::Duration;

use gpui::TestAppContext;

use super::{DEFAULT_REQUEST_TIMEOUT, REQUEST_TIMED_OUT, SaiFetchHttpEngine};

#[test]
fn a_request_without_a_timeout_gets_the_url_request_default() {
    assert_eq!(SaiFetchHttpEngine::request_limit(None), DEFAULT_REQUEST_TIMEOUT);
    assert_eq!(SaiFetchHttpEngine::request_limit(Some(5.0)), Duration::from_secs(5));
    assert_eq!(SaiFetchHttpEngine::request_limit(Some(-1.0)), DEFAULT_REQUEST_TIMEOUT);
}

#[gpui::test]
async fn a_server_that_never_answers_ends_in_an_error(cx: &mut TestAppContext) {
    let limit = Duration::from_secs(1);
    let waiting = cx.update(|cx| {
        cx.spawn(async move |cx| {
            SaiFetchHttpEngine::limited(std::future::pending::<anyhow::Result<()>>(), limit, cx).await
        })
    });
    cx.run_until_parked();
    cx.executor().advance_clock(limit);
    cx.run_until_parked();
    let error = waiting.await.expect_err("the limit ends the wait");
    assert_eq!(error.to_string(), REQUEST_TIMED_OUT);
}
