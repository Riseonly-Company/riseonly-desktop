use gpui::App;
use rise_media::map_view::LatLng;
use rise_platform::gpui_shim::PlatformSupport;
use rise_platform::location;

pub enum MapDirections {}

impl MapDirections {
    pub fn open(to: LatLng, name: &str, cx: &mut App) {
        if location::open_directions(to.latitude, to.longitude, name, cx) == PlatformSupport::Unsupported {
            tracing::debug!(target: "riseonly", "no navigator took the directions link");
        }
    }
}
