use rise_canon::RiseLimits;
use rise_canon::RiseIdentityCanon;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::map_view::LatLng;

use super::map_tracks_types::{MapPlace, MapPlaceKind, MapPlacePolicy};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;

const OPTIMISTIC_TEMPORARY_PREFIX: &str = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MapPlaceSheetSessionPage {
    Actions,
    Details,
    Edit,
}

#[derive(Clone, PartialEq, Debug)]
pub struct MapPlaceSheetSession {
    pub id: Uuid,
    pub account_id: Option<String>,
    pub account_epoch: Option<String>,
    pub saved: Option<MapPlace>,
    pub draft: MapPlace,
    pub page: MapPlaceSheetSessionPage,
    pub is_creating: bool,
    pub is_saving: bool,
    pub error_text: Option<String>,
}

impl MapPlaceSheetSession {
    pub fn new(place: MapPlace) -> Self {
        Self {
            id: Uuid::new_v4(),
            account_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            saved: Some(place.clone()),
            draft: place,
            page: MapPlaceSheetSessionPage::Details,
            is_creating: false,
            is_saving: false,
            error_text: None,
        }
    }

    pub fn with_coordinate(coordinate: LatLng) -> Self {
        let mut draft = MapPlace::new(
            format!("{OPTIMISTIC_TEMPORARY_PREFIX}{}", Uuid::new_v4().uuid_string()),
            MapPlaceKind::Custom,
            false,
            String::new(),
            coordinate.latitude,
            coordinate.longitude,
            RiseLimits::service_value("map_home_radius_m"),
        );
        draft.emoji = "\u{1F333}".to_owned();
        Self {
            id: Uuid::new_v4(),
            account_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            saved: None,
            draft,
            page: MapPlaceSheetSessionPage::Actions,
            is_creating: true,
            is_saving: false,
            error_text: None,
        }
    }

    pub fn is_current(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(self.account_id.as_deref(), self.account_epoch.as_deref())
    }

    pub fn is_valid(&self) -> bool {
        MapPlacePolicy::name_rule().error_text(&self.draft.name).is_none()
            && MapPlacePolicy::description_rule().error_text(&self.draft.description).is_none()
    }
}
