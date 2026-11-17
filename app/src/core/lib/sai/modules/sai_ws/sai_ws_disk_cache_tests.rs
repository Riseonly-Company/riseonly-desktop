use super::sai_ws_disk_cache::{SaiWsDiskCache, SaiWsDiskCacheLRURecord, SaiWsDiskCacheLimits};

fn record(digest_hex: &str, byte_count: i64, last_accessed_at: f64) -> SaiWsDiskCacheLRURecord {
    SaiWsDiskCacheLRURecord {
        digest_hex: digest_hex.to_owned(),
        byte_count,
        last_accessed_at,
    }
}

#[test]
fn owner_scope_separates_accounts_and_legacy_keys() {
    assert_eq!(
        SaiWsDiskCache::owner_scope_identifier("account:user-a:typed-search:hello"),
        "account:user-a:"
    );
    assert_eq!(
        SaiWsDiskCache::owner_scope_identifier("account:user-b:typed-search:hello"),
        "account:user-b:"
    );
    assert_eq!(
        SaiWsDiskCache::owner_scope_identifier("legacy-unscoped-key"),
        "__unscoped__"
    );
}

#[test]
fn eviction_uses_least_recently_used_order_for_entry_budget() {
    let records = [
        record("oldest", 10, 1.0),
        record("middle", 10, 2.0),
        record("newest", 10, 3.0),
    ];
    let limits = SaiWsDiskCacheLimits {
        maximum_entry_count_per_account: 2,
        maximum_total_bytes_per_account: 1_000,
    };

    assert_eq!(
        SaiWsDiskCache::eviction_digests(&records, &limits),
        vec!["oldest".to_owned()]
    );
}

#[test]
fn eviction_honors_byte_budget_and_can_drop_oversized_newest_entry() {
    let records = [record("old", 20, 1.0), record("new-but-too-large", 100, 2.0)];
    let limits = SaiWsDiskCacheLimits {
        maximum_entry_count_per_account: 10,
        maximum_total_bytes_per_account: 50,
    };

    assert_eq!(
        SaiWsDiskCache::eviction_digests(&records, &limits),
        vec!["old".to_owned(), "new-but-too-large".to_owned()]
    );
}
