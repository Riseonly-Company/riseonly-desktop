use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, BoxShadow, ElementId, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, Rgba,
    SharedString, Size, Window, black, canvas, point,
};
use rise_canon::rise_proto;
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;
use rise_ui::{IconUi, theme};

use crate::modules::media_picker::editor::editor_link_entity::MediaLinkAddress;
use crate::modules::post::components::post::post_music_plate_overlay::MediaFrameRect;

/// one tap zone of a link baked into a photo or video
/// unrotated box and its turn, in fractions of the published frame
#[derive(Clone, Debug, PartialEq)]
pub struct MediaLinkZone {
    pub id: i64,
    pub url: String,
    pub center_x: f64,
    pub center_y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
}

impl MediaLinkZone {
    pub fn host(&self) -> String {
        MediaLinkAddress::host(&self.url)
    }

    pub fn zones_story(story: &[rise_proto::stories::StoryLinkArea]) -> Vec<MediaLinkZone> {
        story
            .iter()
            .enumerate()
            .filter_map(|(index, area)| {
                if area.url.is_empty() || area.width <= 0.0 || area.height <= 0.0 {
                    return None;
                }
                Some(MediaLinkZone {
                    id: index as i64,
                    url: area.url.clone(),
                    center_x: area.center_x,
                    center_y: area.center_y,
                    width: area.width,
                    height: area.height,
                    rotation: area.rotation,
                })
            })
            .collect()
    }

    /// always empty, links live only in stories
    /// feed and post detail and riffs ask here, so old posts with link_areas show no link zone or hint
    pub fn zones_post(_post: &[rise_proto::post::PostLinkArea], _slide: i64) -> Vec<MediaLinkZone> {
        Vec::new()
    }
}

pub type MediaLinkAreasOverlayOnTap = Rc<dyn Fn(&MediaLinkZone, Point<Pixels>, &mut Window, &mut App)>;

#[derive(Default)]
struct MediaLinkAreasOverlayState {
    origin: Cell<Point<Pixels>>,
    container: Cell<Size<Pixels>>,
    pressed: Cell<Option<i64>>,
}

/// transparent tap targets over the media mapped into the media as drawn, fit or fill
/// only the zones take touches and everything else falls through
#[derive(IntoElement)]
pub struct MediaLinkAreasOverlay {
    id: ElementId,
    zones: Vec<MediaLinkZone>,
    media_aspect: Option<f32>,
    media_fills: bool,
    on_tap: MediaLinkAreasOverlayOnTap,
}

impl MediaLinkAreasOverlay {
    const MINIMUM_SIDE: f32 = 32.0;

    pub fn new(id: impl Into<ElementId>, zones: Vec<MediaLinkZone>, on_tap: MediaLinkAreasOverlayOnTap) -> Self {
        Self { id: id.into(), zones, media_aspect: None, media_fills: true, on_tap }
    }

    pub fn media_aspect(mut self, media_aspect: Option<f32>) -> Self {
        self.media_aspect = media_aspect;
        self
    }

    pub fn media_fills(mut self, media_fills: bool) -> Self {
        self.media_fills = media_fills;
        self
    }

    /// true if a point of the overlay lands on the zone tap box, same mapping
    /// min side and turn as the touch targets
    /// a layer drawn over the targets asks this
    pub fn hit(
        zone: &MediaLinkZone,
        at: Point<Pixels>,
        container: Size<Pixels>,
        media_aspect: Option<f32>,
        media_fills: bool,
    ) -> bool {
        let frame = MediaFrameRect::rect(container, media_aspect, media_fills);
        let frame_x = f32::from(frame.origin.x);
        let frame_y = f32::from(frame.origin.y);
        let frame_width = f32::from(frame.size.width);
        let frame_height = f32::from(frame.size.height);
        let dx = f32::from(at.x) - (frame_x + zone.center_x as f32 * frame_width);
        let dy = f32::from(at.y) - (frame_y + zone.center_y as f32 * frame_height);
        let turn = -(zone.rotation as f32);
        let local_x = dx * turn.cos() - dy * turn.sin();
        let local_y = dx * turn.sin() + dy * turn.cos();
        local_x.abs() <= f32::max(Self::MINIMUM_SIDE, zone.width as f32 * frame_width) / 2.0
            && local_y.abs() <= f32::max(Self::MINIMUM_SIDE, zone.height as f32 * frame_height) / 2.0
    }

    fn zone_at(
        zones: &[MediaLinkZone],
        at: Point<Pixels>,
        container: Size<Pixels>,
        media_aspect: Option<f32>,
        media_fills: bool,
    ) -> Option<MediaLinkZone> {
        zones
            .iter()
            .rev()
            .find(|zone| Self::hit(zone, at, container, media_aspect, media_fills))
            .cloned()
    }

    fn center(zone: &MediaLinkZone, container: Size<Pixels>, media_aspect: Option<f32>, media_fills: bool) -> Point<Pixels> {
        let frame = MediaFrameRect::rect(container, media_aspect, media_fills);
        point(
            frame.origin.x + frame.size.width * zone.center_x as f32,
            frame.origin.y + frame.size.height * zone.center_y as f32,
        )
    }
}

impl RenderOnce for MediaLinkAreasOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| MediaLinkAreasOverlayState::default());
        let zones = Rc::new(self.zones);
        let media_aspect = self.media_aspect;
        let media_fills = self.media_fills;
        let on_tap = self.on_tap;

        let measured = state.clone();
        let probe = canvas(
            move |bounds, _, cx| {
                let state = measured.read(cx);
                state.origin.set(bounds.origin);
                state.container.set(bounds.size);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();

        let press_state = state.clone();
        let press_zones = zones.clone();
        let release_state = state;
        let release_zones = zones;
        div()
            .id(self.id)
            .absolute()
            .inset_0()
            .child(probe)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                let state = press_state.read(cx);
                let local = event.position - state.origin.get();
                let zone = Self::zone_at(&press_zones, local, state.container.get(), media_aspect, media_fills);
                state.pressed.set(zone.as_ref().map(|zone| zone.id));
                if zone.is_some() {
                    cx.stop_propagation();
                }
            })
            .on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, window: &mut Window, cx: &mut App| {
                let (pressed, local, container) = {
                    let state = release_state.read(cx);
                    (state.pressed.replace(None), event.position - state.origin.get(), state.container.get())
                };
                let Some(pressed) = pressed else {
                    return;
                };
                cx.stop_propagation();
                let Some(zone) = Self::zone_at(&release_zones, local, container, media_aspect, media_fills) else {
                    return;
                };
                if zone.id != pressed {
                    return;
                }
                let center = Self::center(&zone, container, media_aspect, media_fills);
                on_tap(&zone, center, window, cx);
            })
    }
}

/// first tap on a link in a story pauses it and this hint names the site
/// a tap on the hint or a second tap on the link opens the app link sheet, same as telegram
#[derive(IntoElement)]
pub struct MediaLinkOpenHint {
    id: ElementId,
    zone: MediaLinkZone,
    on_open: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl MediaLinkOpenHint {
    pub fn new(id: impl Into<ElementId>, zone: MediaLinkZone, on_open: Rc<dyn Fn(&mut Window, &mut App)>) -> Self {
        Self { id: id.into(), zone, on_open }
    }
}

impl RenderOnce for MediaLinkOpenHint {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let plate: Hsla = Rgba { r: 0.11, g: 0.11, b: 0.12, a: 1.0 }.into();
        let ink = theme(cx).content_on(plate);
        let host = self.zone.host();
        let on_open = self.on_open;
        let label = hstack(8.0)
            .text_color(ink)
            .child(div().flex_none().children(IconUi::sized("link", px(14.0), ink, false)))
            .child(
                vstack(1.0)
                    .items_start()
                    .min_w_0()
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(15.0, FontWeight::SEMIBOLD))
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(SharedString::from(tr("ctx_open_link"))),
                    )
                    .child(
                        div()
                            .swiftui_font(Font::system_size(12.0))
                            .text_color(ink.opacity(0.75))
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(SharedString::from(host)),
                    ),
            )
            .px(px(14.0))
            .py(px(9.0))
            .bg(plate)
            .rounded_full()
            .shadow(vec![BoxShadow {
                color: black().opacity(0.25),
                offset: point(px(0.0), px(2.0)),
                blur_radius: px(8.0),
                spread_radius: px(0.0),
                inset: false,
            }]);
        plain(self.id).on_press(move |window, cx| on_open(window, cx)).child(label)
    }
}
