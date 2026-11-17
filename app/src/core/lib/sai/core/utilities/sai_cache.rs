use std::fmt::Display;

use super::sai_account_cache_scope::SaiAccountCacheScope;

const SEGMENT_SEPARATOR: &str = "-";

/// cache key scoped to the active account
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SaiCache {
    pub key: String,
}

impl SaiCache {
    /// every segment is printed with its display text, which for strings is the string itself
    pub fn new<Segment: Display>(segments: impl IntoIterator<Item = Segment>) -> Self {
        let logical_key = segments
            .into_iter()
            .map(|segment| segment.to_string())
            .collect::<Vec<_>>()
            .join(SEGMENT_SEPARATOR);
        Self {
            key: Self::make_key(&logical_key),
        }
    }

    fn make_key(logical_key: &str) -> String {
        SaiAccountCacheScope::scoped_key(logical_key)
    }
}
