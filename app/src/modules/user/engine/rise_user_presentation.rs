use std::collections::BTreeMap;

use rise_widgets::PageStatus;

use super::rise_user_engine_models::{ProfileKey, ProfileModel};

#[derive(Clone, PartialEq, Debug, Default)]
pub struct ProfileEntry {
    pub profile: ProfileModel,
    pub status: PageStatus,
    pub error_key: Option<&'static str>,
    pub did_load: bool,
    pub follow_in_flight: bool,
}

impl ProfileEntry {
    pub fn has_content(&self) -> bool {
        !self.profile.is_empty()
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct UserSnapshot {
    pub revision: u64,
    pub viewer_id: Option<String>,
    pub profiles: BTreeMap<ProfileKey, ProfileEntry>,
}

impl UserSnapshot {
    pub fn entry(&self, key: &ProfileKey) -> Option<&ProfileEntry> {
        self.profiles.get(key)
    }

    pub fn profile(&self, key: &ProfileKey) -> Option<&ProfileModel> {
        self.profiles
            .get(key)
            .map(|entry| &entry.profile)
            .filter(|profile| !profile.is_empty())
    }

    /// True when the key names the signed-in account, whether the page asked for
    /// it as `Own` or reached it by id or tag from somewhere else in the app.
    pub fn is_viewer(&self, key: &ProfileKey) -> bool {
        if key.is_own() {
            return true;
        }

        let Some(viewer) = self.viewer_id.as_deref().filter(|id| !id.is_empty()) else {
            return false;
        };

        match key {
            ProfileKey::Own => true,
            ProfileKey::Id(id) => id == viewer,
            ProfileKey::Tag(_) => self
                .profile(key)
                .map(|profile| profile.id == viewer)
                .unwrap_or(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with(key: ProfileKey, profile: ProfileModel) -> UserSnapshot {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            key,
            ProfileEntry {
                profile,
                status: PageStatus::Loaded,
                did_load: true,
                ..ProfileEntry::default()
            },
        );

        UserSnapshot {
            revision: 1,
            viewer_id: Some("u1".into()),
            profiles,
        }
    }

    #[test]
    fn a_blank_placeholder_is_not_content() {
        let snapshot = snapshot_with(
            ProfileKey::Own,
            ProfileModel::placeholder(String::new(), String::new()),
        );

        assert!(snapshot.profile(&ProfileKey::Own).is_none());
        assert!(!snapshot.entry(&ProfileKey::Own).unwrap().has_content());
    }

    #[test]
    fn reaching_your_own_profile_by_id_still_reads_as_your_own() {
        let snapshot = snapshot_with(
            ProfileKey::Id("u1".into()),
            ProfileModel::placeholder("u1".into(), "aianov".into()),
        );

        assert!(snapshot.is_viewer(&ProfileKey::Id("u1".into())));
        assert!(!snapshot.is_viewer(&ProfileKey::Id("u2".into())));
    }

    #[test]
    fn reaching_it_by_tag_needs_the_payload_before_it_can_tell() {
        let mut snapshot = snapshot_with(
            ProfileKey::Tag("aianov".into()),
            ProfileModel::placeholder("u1".into(), "aianov".into()),
        );
        assert!(snapshot.is_viewer(&ProfileKey::Tag("aianov".into())));

        snapshot.profiles.clear();
        assert!(
            !snapshot.is_viewer(&ProfileKey::Tag("aianov".into())),
            "a tag alone says nothing about whose account it is"
        );
    }

    #[test]
    fn a_signed_out_snapshot_owns_nothing_it_did_not_ask_for_as_its_own() {
        let mut snapshot = snapshot_with(
            ProfileKey::Id("u1".into()),
            ProfileModel::placeholder("u1".into(), "aianov".into()),
        );
        snapshot.viewer_id = None;

        assert!(!snapshot.is_viewer(&ProfileKey::Id("u1".into())));
        assert!(snapshot.is_viewer(&ProfileKey::Own));
    }
}
