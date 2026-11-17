use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, TestAppContext};

use super::super::sai_fetch::SaiFetch;
use super::super::sai_fetch_options::SaiFetchOptions;
use super::{SaiFetchHandle, SaiFetchOperation};

#[gpui::test]
async fn a_caller_that_lets_go_of_the_handle_still_gets_its_callback(cx: &mut TestAppContext) {
    let delivered = Rc::new(Cell::new(None));
    let sink = Rc::clone(&delivered);
    cx.update(|cx| {
        let operation: SaiFetchOperation<i32> = Rc::new(|cx: &mut App| {
            let timer = cx.background_executor().timer(Duration::from_millis(10));
            cx.background_spawn(async move {
                timer.await;
                Ok::<i32, anyhow::Error>(7)
            })
        });
        let handle = cx.new(|_| SaiFetchHandle::new(operation, SaiFetchOptions::default()));
        SaiFetch::handler(&handle, move |value, _| sink.set(Some(value)), |_, _| {}, cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(20));
    cx.run_until_parked();
    assert_eq!(delivered.get(), Some(7));
}
