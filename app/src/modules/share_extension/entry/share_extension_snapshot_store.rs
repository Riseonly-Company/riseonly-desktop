use std::cmp::Ordering;

use futures::future::{BoxFuture, FutureExt};
use rise_core::foundation::foundation_compare::{CompareOptions, compare_case_insensitive, folding};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;

use crate::modules::share_extension::entry::share_extension_dependencies::ShareExtensionSnapshotProviding;
use crate::modules::share_extension::entry::share_extension_models::{
    ShareAccountSnapshot, ShareDestination, ShareDestinationKind, ShareDestinationTitle,
};
use crate::modules::share_extension::entry::share_extension_strings::ShareStrings;
use crate::modules::share_extension::shared::rise_share_models::{RiseShareSnapshot, RiseShareTargetKind};

#[derive(Clone, Debug)]
pub struct ShareExtensionSnapshotStore {
    snapshot: RiseShareSnapshot,
    destinations: Vec<ShareDestination>,
}

impl ShareExtensionSnapshotStore {
    pub fn new(snapshot: RiseShareSnapshot) -> Self {
        let destinations = snapshot
            .targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| {
                if !target.can_send {
                    return None;
                }
                let kind = Self::destination_kind(target.kind);
                Some(ShareDestination::new(
                    target.id.clone(),
                    kind,
                    ShareDestinationTitle::resolve(kind, &target.title),
                    Self::safe_remote_url(target.avatar_url.as_deref()),
                    target.avatar_color_key.clone(),
                    target.topics_enabled,
                    target.can_send,
                    target.last_activity_ms,
                    index as i64,
                ))
            })
            .collect();
        Self { snapshot, destinations }
    }

    fn destination_kind(kind: RiseShareTargetKind) -> ShareDestinationKind {
        match kind {
            RiseShareTargetKind::Favourites => ShareDestinationKind::Saved,
            RiseShareTargetKind::PrivateChat => ShareDestinationKind::Direct,
            RiseShareTargetKind::Group => ShareDestinationKind::Group,
            RiseShareTargetKind::Channel => ShareDestinationKind::Channel,
        }
    }

    fn safe_remote_url(raw: Option<&str>) -> Option<Url> {
        let url = Url::from_string(raw?)?;
        let scheme = url.scheme()?.to_lowercase();
        if scheme == "https" || scheme == "http" {
            Some(url)
        } else {
            None
        }
    }

    fn destination_sort(lhs: &ShareDestination, rhs: &ShareDestination) -> Ordering {
        if lhs.last_activity_at_milliseconds != rhs.last_activity_at_milliseconds {
            return rhs.last_activity_at_milliseconds.cmp(&lhs.last_activity_at_milliseconds);
        }
        if lhs.sort_index != rhs.sort_index {
            return lhs.sort_index.cmp(&rhs.sort_index);
        }
        compare_case_insensitive(&lhs.title, &rhs.title)
    }

    fn normalized(value: &str) -> String {
        let folded = folding(
            value,
            CompareOptions::CASE_INSENSITIVE.union(CompareOptions::DIACRITIC_INSENSITIVE),
        );
        trimming_whitespaces_and_newlines(&folded).to_owned()
    }

    fn sorted_prefix(mut destinations: Vec<ShareDestination>, limit: i64) -> Vec<ShareDestination> {
        destinations.sort_by(Self::destination_sort);
        destinations.truncate(limit.max(0) as usize);
        destinations
    }
}

impl ShareExtensionSnapshotProviding for ShareExtensionSnapshotStore {
    fn account(&self) -> BoxFuture<'_, anyhow::Result<ShareAccountSnapshot>> {
        let account = &self.snapshot.account;
        let snapshot = ShareAccountSnapshot {
            id: account.id.clone(),
            display_name: if account.name.is_empty() {
                ShareStrings::account()
            } else {
                account.name.clone()
            },
            tag: account.tag.clone(),
            avatar_url: Self::safe_remote_url(account.avatar_url.as_deref()),
        };
        async move { Ok(snapshot) }.boxed()
    }

    fn recent_destinations(&self, limit: i64) -> BoxFuture<'_, anyhow::Result<Vec<ShareDestination>>> {
        let destinations = Self::sorted_prefix(self.destinations.clone(), limit);
        async move { Ok(destinations) }.boxed()
    }

    fn search_destinations(
        &self,
        query: String,
        limit: i64,
    ) -> BoxFuture<'_, anyhow::Result<Vec<ShareDestination>>> {
        let normalized_query = Self::normalized(&query);
        let destinations = if normalized_query.is_empty() {
            Vec::new()
        } else {
            let matches = self
                .destinations
                .iter()
                .filter(|destination| Self::normalized(&destination.title).contains(&normalized_query))
                .cloned()
                .collect();
            Self::sorted_prefix(matches, limit)
        };
        async move { Ok(destinations) }.boxed()
    }
}
