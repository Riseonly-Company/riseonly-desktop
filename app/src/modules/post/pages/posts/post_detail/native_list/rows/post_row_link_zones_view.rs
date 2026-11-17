use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, ElementId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Window, canvas,
    point,
};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::media_link_areas_overlay::MediaLinkZone;
use crate::modules::post::components::post::post_music_plate_overlay::MediaFrameRect;

/// zones of the shown page and where the page sits
#[derive(Clone, Debug, PartialEq)]
pub struct PostRowLinkZonesViewTarget {
    pub zones: Vec<MediaLinkZone>,
    pub frame: Bounds<Pixels>,
    pub media_aspect: Option<f32>,
    pub media_fills: bool,
}

pub type PostRowLinkZonesViewTargetProvider = Rc<dyn Fn(&App) -> Option<PostRowLinkZonesViewTarget>>;
pub type PostRowLinkZonesViewOnOpen = Rc<dyn Fn(&MediaLinkZone, &mut Window, &mut App)>;

/// placed links over the shown page of a native row, same zones
/// hits and touch ownership as the feed media link areas overlay
/// only zones take touches and everything else falls through
#[derive(IntoElement)]
pub struct PostRowLinkZonesView {
    id: ElementId,
    target: PostRowLinkZonesViewTargetProvider,
    on_open: PostRowLinkZonesViewOnOpen,
}

#[derive(Default)]
struct PostRowLinkZonesViewState {
    origin: Cell<Point<Pixels>>,
    pressed: Cell<Option<i64>>,
}

impl PostRowLinkZonesView {
    const MINIMUM_SIDE: f32 = 32.0;

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), target: Rc::new(|_: &App| None), on_open: Rc::new(|_: &MediaLinkZone, _: &mut Window, _: &mut App| {}) }
    }

    pub fn target(mut self, target: PostRowLinkZonesViewTargetProvider) -> Self {
        self.target = target;
        self
    }

    pub fn on_open(mut self, on_open: PostRowLinkZonesViewOnOpen) -> Self {
        self.on_open = on_open;
        self
    }

    /// zone under at (this views coords), topmost first
    pub fn zone(target: Option<PostRowLinkZonesViewTarget>, at: Point<Pixels>) -> Option<MediaLinkZone> {
        let target = target?;
        if target.zones.is_empty() {
            return None;
        }
        let media = MediaFrameRect::rect(target.frame.size, target.media_aspect, target.media_fills);
        let media_x = f32::from(media.origin.x);
        let media_y = f32::from(media.origin.y);
        let media_width = f32::from(media.size.width);
        let media_height = f32::from(media.size.height);
        let local = point(at.x - target.frame.origin.x, at.y - target.frame.origin.y);
        for zone in target.zones.iter().rev() {
            let width = f32::max(Self::MINIMUM_SIDE, zone.width as f32 * media_width);
            let height = f32::max(Self::MINIMUM_SIDE, zone.height as f32 * media_height);
            let dx = f32::from(local.x) - (media_x + zone.center_x as f32 * media_width);
            let dy = f32::from(local.y) - (media_y + zone.center_y as f32 * media_height);
            let angle = -(zone.rotation as f32);
            let local_x = dx * angle.cos() - dy * angle.sin();
            let local_y = dx * angle.sin() + dy * angle.cos();
            if local_x.abs() <= width / 2.0 && local_y.abs() <= height / 2.0 {
                return Some(zone.clone());
            }
        }
        None
    }
}

impl RenderOnce for PostRowLinkZonesView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| PostRowLinkZonesViewState::default());

        let measured = state.clone();
        let probe = canvas(
            move |bounds, _, cx| {
                measured.read(cx).origin.set(bounds.origin);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();

        let press_state = state.clone();
        let press_target = self.target.clone();
        let drag_state = state.clone();
        let release_state = state;
        let release_target = self.target;
        let on_open = self.on_open;
        div()
            .id(self.id)
            .absolute()
            .inset_0()
            .child(probe)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                let origin = press_state.read(cx).origin.get();
                let zone = Self::zone(press_target(cx), event.position - origin);
                press_state.read(cx).pressed.set(zone.as_ref().map(|zone| zone.id));
                if zone.is_some() {
                    cx.stop_propagation();
                }
            })
            .on_mouse_move(move |_: &MouseMoveEvent, _: &mut Window, cx: &mut App| {
                if drag_state.read(cx).pressed.get().is_some() {
                    cx.stop_propagation();
                }
            })
            .on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, window: &mut Window, cx: &mut App| {
                let (pressed, origin) = {
                    let state = release_state.read(cx);
                    (state.pressed.replace(None), state.origin.get())
                };
                let Some(pressed) = pressed else {
                    return;
                };
                cx.stop_propagation();
                let Some(zone) = Self::zone(release_target(cx), event.position - origin) else {
                    return;
                };
                if zone.id != pressed {
                    return;
                }
                on_open(&zone, window, cx);
            })
    }
}
