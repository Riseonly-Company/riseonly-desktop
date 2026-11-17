use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::host_os::HostOs;

use super::process_activity::{
    ProcessActivitySource, ProcessActivityToken, ProcessActivityUnavailable,
    begin_process_activity_with,
};

struct EndCounter(Arc<AtomicUsize>);

impl Drop for EndCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Granting(Arc<AtomicUsize>);

impl ProcessActivitySource for Granting {
    fn begin(&self, _reason: &str) -> Result<ProcessActivityToken, ProcessActivityUnavailable> {
        Ok(Box::new(EndCounter(self.0.clone())))
    }
}

struct Unwritten;

impl ProcessActivitySource for Unwritten {}

struct Refusing;

impl ProcessActivitySource for Refusing {
    fn begin(&self, _reason: &str) -> Result<ProcessActivityToken, ProcessActivityUnavailable> {
        Err(ProcessActivityUnavailable::NotThrottled { host: HostOs::current() })
    }
}

#[test]
fn the_activity_lasts_exactly_as_long_as_the_lease() {
    let ended = Arc::new(AtomicUsize::new(0));
    let lease = begin_process_activity_with(&Granting(ended.clone()), "test");
    assert!(lease.is_held());
    assert_eq!(ended.load(Ordering::SeqCst), 0);
    drop(lease);
    assert_eq!(ended.load(Ordering::SeqCst), 1);
}

#[test]
fn a_branch_that_is_not_written_names_itself_and_the_lease_holds_nothing() {
    let answer = Unwritten.begin("test").err();
    assert_eq!(
        answer,
        Some(ProcessActivityUnavailable::BranchNotWritten { host: HostOs::current() })
    );
    assert!(!begin_process_activity_with(&Unwritten, "test").is_held());
}

#[test]
fn an_os_that_does_not_throttle_gives_a_lease_that_holds_nothing() {
    assert!(!begin_process_activity_with(&Refusing, "test").is_held());
}
