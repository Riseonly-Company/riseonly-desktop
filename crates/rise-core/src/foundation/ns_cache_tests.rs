use std::sync::{Arc, Mutex, MutexGuard};

use super::ns_cache::{NSCache, ns_cache_did_receive_memory_warning};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn stores_and_returns_an_object() {
    let _serial = serial();
    let cache: NSCache<String, Arc<str>> = NSCache::new();
    cache.set_object(Arc::from("frame"), "a".to_string());

    assert_eq!(cache.object(&"a".to_string()).as_deref(), Some("frame"));
    assert_eq!(cache.object(&"missing".to_string()), None);
}

#[test]
fn replacing_a_key_replaces_its_cost() {
    let _serial = serial();
    let cache: NSCache<&'static str, u32> = NSCache::new();
    cache.set_object_cost(1, "a", 40);
    cache.set_object_cost(2, "a", 10);

    assert_eq!(cache.object(&"a"), Some(2));
    assert_eq!(cache.count(), 1);
    assert_eq!(cache.total_cost(), 10);
}

#[test]
fn count_limit_evicts_the_least_recently_used() {
    let _serial = serial();
    let cache: NSCache<&'static str, u32> = NSCache::new();
    cache.set_count_limit(2);
    cache.set_object(1, "a");
    cache.set_object(2, "b");
    assert_eq!(cache.object(&"a"), Some(1));
    cache.set_object(3, "c");

    assert_eq!(cache.count(), 2);
    assert_eq!(cache.object(&"b"), None);
    assert_eq!(cache.object(&"a"), Some(1));
    assert_eq!(cache.object(&"c"), Some(3));
}

#[test]
fn total_cost_limit_evicts_until_the_cost_fits() {
    let _serial = serial();
    let cache: NSCache<&'static str, u32> = NSCache::new();
    cache.set_total_cost_limit(100);
    cache.set_object_cost(1, "a", 40);
    cache.set_object_cost(2, "b", 40);
    cache.set_object_cost(3, "c", 40);

    assert!(cache.total_cost() <= 100);
    assert_eq!(cache.object(&"a"), None);
    assert_eq!(cache.object(&"b"), Some(2));
    assert_eq!(cache.object(&"c"), Some(3));
    assert_eq!(cache.total_cost(), 80);
}

#[test]
fn an_object_costlier_than_the_whole_limit_is_not_kept() {
    let _serial = serial();
    let cache: NSCache<&'static str, u32> = NSCache::new();
    cache.set_total_cost_limit(50);
    cache.set_object_cost(1, "huge", 80);

    assert_eq!(cache.object(&"huge"), None);
    assert_eq!(cache.total_cost(), 0);
}

#[test]
fn zero_limits_mean_unbounded() {
    let _serial = serial();
    let cache: NSCache<u32, u32> = NSCache::new();
    for key in 0..500 {
        cache.set_object_cost(key, key, 1_000_000);
    }

    assert_eq!(cache.count(), 500);
}

#[test]
fn lowering_a_limit_evicts_at_once() {
    let _serial = serial();
    let cache: NSCache<u32, u32> = NSCache::new();
    for key in 0..10 {
        cache.set_object_cost(key, key, 10);
    }
    cache.set_count_limit(4);
    assert_eq!(cache.count(), 4);
    cache.set_total_cost_limit(20);
    assert_eq!(cache.count(), 2);
    assert_eq!(cache.object(&9), Some(9));
}

#[test]
fn remove_object_and_remove_all_objects() {
    let _serial = serial();
    let cache: NSCache<&'static str, u32> = NSCache::new();
    cache.set_object_cost(1, "a", 5);
    cache.set_object_cost(2, "b", 7);
    cache.remove_object(&"a");

    assert_eq!(cache.object(&"a"), None);
    assert_eq!(cache.total_cost(), 7);

    cache.remove_all_objects();
    assert_eq!(cache.count(), 0);
    assert_eq!(cache.total_cost(), 0);
}

#[test]
fn memory_warning_empties_every_live_cache() {
    let _serial = serial();
    let first: NSCache<&'static str, u32> = NSCache::new();
    let second: NSCache<u32, String> = NSCache::new();
    first.set_object(1, "a");
    second.set_object("x".to_string(), 7);
    let dropped: NSCache<u32, u32> = NSCache::new();
    dropped.set_object(1, 1);
    drop(dropped);

    ns_cache_did_receive_memory_warning();

    assert_eq!(first.count(), 0);
    assert_eq!(second.count(), 0);
    first.set_object(2, "b");
    assert_eq!(first.object(&"b"), Some(2));
}

#[test]
fn name_round_trips() {
    let _serial = serial();
    let cache: NSCache<u32, u32> = NSCache::new();
    assert_eq!(cache.name(), "");
    cache.set_name("riseonly.lottie.posters");
    assert_eq!(cache.name(), "riseonly.lottie.posters");
}
