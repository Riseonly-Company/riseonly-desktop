use rise_canon::rise_request::story::create_story::TrackPlate;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::UrlComponents;

use crate::core::stores::http::http_service::HttpService;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_picker_result::MediaPickerTrackPlate;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;

/// music plate of create_story built by the rules the service checks
/// service refuses the whole story if the plate fails
/// a refused cover is left out instead
pub enum StoryTrackPlateWire {}

impl StoryTrackPlateWire {
    const HTTPS_SCHEME: &'static str = "https";
    const PATH_SEPARATOR: &'static str = "/";
    const NETWORK_PATH_PREFIX: &'static str = "//";

    /// none when the plate cant travel, the service refuses a plate without a track id
    pub fn plate(
        track_id: Option<&str>,
        plate: Option<&MediaPickerTrackPlate>,
        track: &RiseStoredMusicTrack,
    ) -> Option<TrackPlate> {
        let plate = plate?;
        let track_id = track_id?;
        if trimming_whitespaces_and_newlines(track_id).is_empty() {
            return None;
        }
        Some(TrackPlate {
            artist: Some(
                track
                    .artist
                    .as_ref()
                    .map(|artist| artist.stage_name.clone())
                    .unwrap_or_default(),
            ),
            center_x: Some(plate.center_x),
            center_y: Some(plate.center_y),
            cover_url: Some(Self::cover_url(track.cover_url.as_deref())),
            is_dark: Some(plate.is_dark),
            rotation: Some(plate.rotation),
            scale: Some(plate.scale),
            title: Some(track.title.clone().unwrap_or_default()),
        })
    }

    /// cover as the service stores it, a file route path or https address on its media hosts
    /// anything else is empty, also local files and api urls
    pub fn cover_url(raw: Option<&str>) -> String {
        let Some(normalized) = RemoteMediaURL::optional_normalized(raw) else {
            return String::new();
        };
        if normalized.starts_with(Self::PATH_SEPARATOR) {
            return if normalized.starts_with(Self::NETWORK_PATH_PREFIX) {
                String::new()
            } else {
                normalized
            };
        }
        let Some(components) = UrlComponents::from_string(&normalized) else {
            return String::new();
        };
        let Some(host) = components.host() else {
            return String::new();
        };
        if let Some(api_host) = HttpService::resolved_api_base_url().host()
            && host.to_lowercase() == api_host.to_lowercase()
        {
            let path = components.percent_encoded_path();
            let is_plain_path = path.starts_with(Self::PATH_SEPARATOR)
                && !path.starts_with(Self::NETWORK_PATH_PREFIX)
                && components.percent_encoded_query().is_none()
                && components.percent_encoded_fragment().is_none();
            return if is_plain_path { path.to_owned() } else { String::new() };
        }
        if components
            .scheme()
            .is_some_and(|scheme| scheme.to_lowercase() == Self::HTTPS_SCHEME)
        {
            normalized
        } else {
            String::new()
        }
    }
}
