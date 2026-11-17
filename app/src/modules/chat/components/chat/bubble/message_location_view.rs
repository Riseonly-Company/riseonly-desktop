use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{App, ElementId, FontWeight, IntoElement, ObjectFit, RenderImage, RenderOnce, SharedString, Window, img};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::ns_cache::NSCache;
use rise_i18n::{FormatArg, I18n, format_printf, tr};
use rise_media::map_view::{Camera, LatLng, SnapshotRequest, snapshot};
use rise_theme::color;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::UnitPoint;

use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::media_viewer::location_fullscreen::{
    ChatLocationPin, LOCATION_SPAN_DEGREES, location_app_paths, location_map_image, location_map_style,
    location_presentation, region_zoom,
};
use crate::modules::chat::stores::message::message_actions::message_types::MessageLocationInfo;

#[derive(Default)]
struct MessageLocationViewState {
    snapshot: Option<Arc<RenderImage>>,
    loaded_snapshot_identity: Option<String>,
}

#[derive(IntoElement)]
pub struct MessageLocationView {
    id: ElementId,
    pub location: MessageLocationInfo,
    pub is_own: bool,
}

impl MessageLocationView {
    const MAP_WIDTH: f32 = 280.0;
    const MAP_HEIGHT: f32 = 140.0;

    pub fn new(id: impl Into<ElementId>, location: MessageLocationInfo) -> Self {
        Self {
            id: id.into(),
            location,
            is_own: false,
        }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }

    fn format_remaining(s: TimeInterval) -> String {
        let mins = (s / 60.0).ceil() as i64;
        if mins < 60 {
            return format_printf(&tr("duration_minutes_abbr"), &[FormatArg::Int(mins)]);
        }
        let hours = mins / 60;
        let rest_mins = mins % 60;
        if rest_mins == 0 {
            format_printf(&tr("duration_hours_abbr"), &[FormatArg::Int(hours)])
        } else {
            format_printf(&tr("duration_hours_minutes_abbr"), &[FormatArg::Int(hours), FormatArg::Int(rest_mins)])
        }
    }

    fn snapshot_identity(&self, scale: f32, style_key: &str) -> String {
        MessageLocationSnapshotCache::key(
            self.location.latitude,
            self.location.longitude,
            Self::MAP_WIDTH,
            Self::MAP_HEIGHT,
            scale,
            style_key,
        )
    }
}

impl RenderOnce for MessageLocationView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let live = self.location.live();
        use_task(self.child_id("live-tick"), live, window, cx, async move |cx| {
            if !live {
                return;
            }
            loop {
                cx.background_executor().timer(Duration::from_secs(30)).await;
                if cx.update(|window, _| window.refresh()).is_err() {
                    break;
                }
            }
        });
        let now = Date::now();
        let remaining = self.location.live_remaining(now);

        let store = theme_store(cx);
        let theme = store.read(cx).current_theme().clone();
        let scale = window.scale_factor().max(1.0).min(3.0);
        let style = location_map_style(cx);
        let style_key = format!("{:?}|{}", store.read(cx).resolved_appearance(), I18n::shared().code());
        let identity = self.snapshot_identity(scale, &style_key);

        let state = window.use_keyed_state(self.child_id("snapshot"), cx, |_, _| MessageLocationViewState::default());
        let is_layout_measurement = is_message_layout_measurement(cx);
        {
            let state = state.clone();
            let task_identity = identity.clone();
            let latitude = self.location.latitude;
            let longitude = self.location.longitude;
            use_task(self.child_id("snapshot-task"), identity.clone(), window, cx, async move |cx| {
                let _ = cx.update(|_, cx| {
                    state.update(cx, |state, cx| {
                        state.snapshot = None;
                        state.loaded_snapshot_identity = None;
                        cx.notify();
                    })
                });
                if is_layout_measurement {
                    return;
                }
                let image = MessageLocationSnapshotCache::image(
                    latitude,
                    longitude,
                    (MessageLocationView::MAP_WIDTH, MessageLocationView::MAP_HEIGHT),
                    scale,
                    style,
                    task_identity.clone(),
                    cx,
                )
                .await;
                let _ = cx.update(|_, cx| {
                    state.update(cx, |state, cx| {
                        state.snapshot = image;
                        state.loaded_snapshot_identity = Some(task_identity);
                        cx.notify();
                    })
                });
            });
        }

        let shown = {
            let state = state.read(cx);
            match (&state.snapshot, &state.loaded_snapshot_identity) {
                (Some(snapshot), Some(loaded)) if *loaded == identity => Some(snapshot.clone()),
                _ => None,
            }
        };
        let backdrop = match shown {
            Some(snapshot) => img(snapshot).size_full().object_fit(ObjectFit::Cover).into_any_element(),
            None => LinearGradient::colors(
                [color("#A7CDDB"), color("#D7E5C5")],
                UnitPoint::TOP_LEADING,
                UnitPoint::BOTTOM_TRAILING,
            )
            .view()
            .size_full()
            .into_any_element(),
        };

        let presenter = location_presentation(cx);
        let presented = self.location.clone();
        let map = zstack(Alignment::Center)
            .frame_size(Self::MAP_WIDTH, Self::MAP_HEIGHT)
            .continuous_corners(12.0)
            .overflow_hidden()
            .child(backdrop)
            .child(
                ChatLocationPin::new(self.child_id("pin"), remaining.is_some())
                    .pulse_diameter(56.0)
                    .resting_pulse_diameter(22.0),
            )
            .bubble_tap_target(self.child_id("tap"), move |_window: &mut Window, cx: &mut App| {
                if let Some(presenter) = &presenter {
                    let location = presented.clone();
                    presenter.update(cx, |presenter, cx| presenter.present(location, cx));
                }
            });

        let title = self.location.title.clone().unwrap_or_else(|| {
            if remaining.is_some() { tr("chat_msg_live_location") } else { tr("chat_msg_location") }
        });
        let mut text = vstack(2.0).align(HorizontalAlignment::Leading).child(
            MainText::new(title)
                .px(14.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(if self.is_own { theme.content_on(theme.primary._100) } else { theme.text.primary })
                .number_of_lines(1),
        );
        let secondary = if self.is_own { theme.content_on(theme.primary._100).opacity(0.78) } else { theme.text.secondary };
        if let Some(remaining) = remaining {
            text = text.child(
                MainText::new(format!("{} {}", tr("chat_msg_live_for"), Self::format_remaining(remaining)))
                    .px(12.0)
                    .color(if self.is_own { theme.content_on(theme.primary._100).opacity(0.85) } else { theme.semantic.error_100 })
                    .number_of_lines(1),
            );
        } else if self.location.live() {
            text = text.child(
                MainText::new(tr("chat_location_live_ended")).px(12.0).color(secondary).number_of_lines(1),
            );
        } else if let Some(addr) = self.location.address.clone() {
            text = text.child(MainText::new(addr).px(12.0).color(secondary).number_of_lines(2));
        }

        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .max_w(px(280.0))
            .child(map)
            .child(text)
    }
}

static CACHE: LazyLock<NSCache<String, Arc<RenderImage>>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(80);
    cache.set_total_cost_limit(32 * 1_024 * 1_024);
    cache
});

enum MessageLocationSnapshotCache {}

impl MessageLocationSnapshotCache {
    fn key(latitude: f64, longitude: f64, width: f32, height: f32, scale: f32, style_key: &str) -> String {
        format!(
            "{:.5}|{:.5}|{}x{}|{:.1}|{}",
            latitude, longitude, width as i64, height as i64, scale, style_key
        )
    }

    async fn image(
        latitude: f64,
        longitude: f64,
        size: (f32, f32),
        scale: f32,
        style_json: String,
        key: String,
        cx: &mut gpui::AsyncWindowContext,
    ) -> Option<Arc<RenderImage>> {
        let (width, height) = size;
        if !latitude.is_finite()
            || !longitude.is_finite()
            || !(-90.0..=90.0).contains(&latitude)
            || !(-180.0..=180.0).contains(&longitude)
            || width <= 1.0
            || height <= 1.0
        {
            return None;
        }
        if let Some(cached) = CACHE.object(&key) {
            return Some(cached);
        }
        let paths = location_app_paths()?;
        let center = LatLng::new(latitude, longitude);
        let zoom = region_zoom(
            center,
            LOCATION_SPAN_DEGREES,
            LOCATION_SPAN_DEGREES,
            f64::from(width),
            f64::from(height),
        );
        let request = SnapshotRequest {
            style_json,
            width: width as u32,
            height: height as u32,
            scale_factor: scale,
            camera: Camera::new(center, zoom),
        };
        let frame = cx
            .background_executor()
            .spawn(async move { snapshot(&request, paths) })
            .await;
        let frame = match frame {
            Ok(frame) => frame,
            Err(reason) => {
                tracing::debug!(%reason, "location snapshot failed");
                return None;
            }
        };
        let cost = frame.bgra.len();
        let image = location_map_image(frame)?;
        CACHE.set_object_cost(image.clone(), key, cost);
        Some(image)
    }
}
