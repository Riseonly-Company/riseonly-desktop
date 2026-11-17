use std::collections::HashSet;
use std::sync::Arc;

use rise_core::foundation::foundation_date::Date;

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_core::rise_identifiers::RiseRequestIDAllocator;

#[tokio::test]
async fn request_ids_stay_unique_when_clock_moves_backward_and_across_concurrent_callers() {
    let allocator = Arc::new(RiseRequestIDAllocator::new());
    let fixed = Date::from_time_interval_since_1970(1_700_000_000.0);
    let first = allocator.next(fixed);
    let backwards = allocator.next(fixed.adding_time_interval(-60.0));
    assert!(backwards.raw_value() > first.raw_value());

    let runtime = RiseAccountRuntime::current();
    let group: Vec<_> = (0..16)
        .map(|_| {
            let allocator = allocator.clone();
            runtime.spawn(async move {
                (0..1_000).map(|_| allocator.next(fixed).raw_value()).collect::<Vec<u64>>()
            })
        })
        .collect();
    let mut values: Vec<u64> = Vec::new();
    for chunk in group {
        values.extend(chunk.await.expect("allocator task finished"));
    }
    assert_eq!(values.iter().collect::<HashSet<_>>().len(), values.len());
}
