use std::collections::HashMap;
use std::sync::LazyLock;

use parking_lot::Mutex;
use rise_canon::rise_service_text::file_service;
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_core::foundation::foundation_url::{Url, UrlComponents};

use crate::core::media::remote_media_disk_cache::RemoteMediaHttpResponse;

/// range fan-out advice for a private media download
/// file service sets connections and segment size on the redirect by tier
/// only advice, storage serves any range
pub enum RiseMediaRangePolicy {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RiseMediaRangePolicyAdvice {
    pub max_parallel_ranges: i64,
    pub chunk_bytes: i64,
}

static BY_KEY: LazyLock<Mutex<HashMap<String, RiseMediaRangePolicyAdvice>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl RiseMediaRangePolicy {
    pub const PARALLEL_HEADER: &'static str = file_service::SECURE_HEADER_PARALLEL_RANGES;
    pub const CHUNK_HEADER: &'static str = file_service::SECURE_HEADER_RANGE_CHUNK_BYTES;

    pub const FALLBACK: RiseMediaRangePolicyAdvice = RiseMediaRangePolicyAdvice {
        max_parallel_ranges: 1,
        chunk_bytes: 0,
    };

    const CAPACITY: usize = 512;
    const MAXIMUM_PARALLEL_RANGES: i64 = 16;
    const MINIMUM_CHUNK_BYTES: i64 = 65_536;

    pub fn record(response: &RemoteMediaHttpResponse, url: Option<&Url>) {
        let Some(url) = url else {
            return;
        };
        let Some(parallel) = Self::integer(response.value(Self::PARALLEL_HEADER)) else {
            return;
        };
        let Some(chunk) = Self::integer(response.value(Self::CHUNK_HEADER)) else {
            return;
        };
        if !(1..=Self::MAXIMUM_PARALLEL_RANGES).contains(&parallel)
            || chunk < Self::MINIMUM_CHUNK_BYTES
        {
            return;
        }
        let advice = RiseMediaRangePolicyAdvice {
            max_parallel_ranges: parallel,
            chunk_bytes: chunk,
        };
        let mut by_key = BY_KEY.lock();
        if by_key.len() >= Self::CAPACITY {
            by_key.clear();
        }
        by_key.insert(Self::key(url), advice);
    }

    pub fn advice(url: &Url) -> RiseMediaRangePolicyAdvice {
        BY_KEY
            .lock()
            .get(&Self::key(url))
            .copied()
            .unwrap_or(Self::FALLBACK)
    }

    pub fn reset_for_account_change() {
        BY_KEY.lock().clear();
    }

    /// scheme host and path only, the query is a signature
    /// or cache buster and the advice is about the object
    fn key(url: &Url) -> String {
        let Some(mut components) = UrlComponents::from_url(url, false) else {
            return url.absolute_string().to_owned();
        };
        components.set_query(None);
        components.set_fragment(None);
        components
            .string()
            .unwrap_or_else(|| url.absolute_string().to_owned())
    }

    fn integer(raw: Option<&str>) -> Option<i64> {
        let raw = trimming_whitespaces(raw?);
        if raw.is_empty() {
            return None;
        }
        int_from_string(raw)
    }
}
