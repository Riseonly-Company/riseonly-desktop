use super::NativeMemoryPressure;
use crate::memory_pressure::*;

#[test]
fn the_dispatch_source_starts_and_cancels_cleanly() {
    let watch = NativeMemoryPressure
        .start(Box::new(|_level| {}))
        .expect("libdispatch creates a memory pressure source on every supported macOS");
    drop(watch);
}

#[test]
fn two_watches_live_side_by_side() {
    let first = NativeMemoryPressure.start(Box::new(|_level| {})).expect("first source");
    let second = NativeMemoryPressure.start(Box::new(|_level| {})).expect("second source");
    drop(first);
    drop(second);
}
