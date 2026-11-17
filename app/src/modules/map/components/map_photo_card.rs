use std::f32::consts::PI;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, ElementId, Entity, Hsla, IntoElement, ParentElement, PathBuilder, Pixels, Point, Render,
    RenderOnce, Styled, Subscription, Window, canvas, div, fill, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;

use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItemKind, MediaPickerResult};
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;

type PressAction = Rc<dyn Fn(&mut Window, &mut App)>;

const BUTTON_SIDE: f32 = 48.0;

/// glass button over the routes panel that pins a photo
#[derive(IntoElement)]
pub struct MapAddPhotoButton {
    progress: Option<f64>,
    action: PressAction,
}

impl MapAddPhotoButton {
    pub fn new(progress: Option<f64>, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { progress, action: Rc::new(action) }
    }
}

impl RenderOnce for MapAddPhotoButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let action = self.action;
        let content = match self.progress {
            Some(progress) => {
                let target = progress.clamp(0.04, 1.0) as f32;
                let now = RiseClock::monotonic();
                let reduce_motion = reduce_motion(cx);
                let trim = window.use_keyed_state(ElementId::Name("map-add-photo-progress".into()), cx, |_, _| {
                    Animated::new(target)
                });
                trim.update(cx, |trim, _| trim.set(target, ease_out(0.2), now, reduce_motion));
                let stroke_end = trim.read(cx).frame_value(now, window);
                let track = theme.text.secondary.opacity(0.22);
                let tint = theme.primary._100;
                canvas(|_, _, _| (), move |bounds, (), window, _| paint_progress(bounds, stroke_end, track, tint, window))
                    .size_full()
                    .into_any_element()
            }
            None => div()
                .children(IconUi::sized("camera.fill", px(17.), theme.text.primary, true))
                .into_any_element(),
        };
        plain(ElementId::Name("map-add-photo-button".into()))
            .disabled(self.progress.is_some())
            .on_press(move |window, cx| action(window, cx))
            .child(
                div()
                    .id(ElementId::Name("map-add-photo-face".into()))
                    .tooltip_label(tr("map_photo_add"))
                    .size(px(BUTTON_SIDE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(content)
                    .liquid_chrome_capsule(),
            )
    }
}

fn paint_progress(bounds: Bounds<Pixels>, stroke_end: f32, track: Hsla, tint: Hsla, window: &mut Window) {
    const INSET: f32 = 11.0;
    const LINE_WIDTH: f32 = 3.0;
    let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0 - INSET;
    if radius <= 0.0 {
        return;
    }
    let centre = bounds.center();
    let arc = |start: f32, sweep: f32| -> Vec<Point<Pixels>> {
        let steps = ((sweep.abs() / (2.0 * PI)) * 96.0).ceil().max(2.0) as usize;
        (0..=steps)
            .map(|step| {
                let angle = start + sweep * step as f32 / steps as f32;
                point(centre.x + px(radius * angle.cos()), centre.y + px(radius * angle.sin()))
            })
            .collect()
    };
    let stroke = |points: &[Point<Pixels>], closed: bool, color: Hsla, window: &mut Window| {
        let mut builder = PathBuilder::stroke(px(LINE_WIDTH));
        builder.add_polygon(points, closed);
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    };
    stroke(&arc(0.0, 2.0 * PI), true, track, window);
    let sweep = 2.0 * PI * stroke_end.clamp(0.0, 1.0);
    let progress = arc(-PI / 2.0, sweep);
    stroke(&progress, false, tint, window);
    let half = LINE_WIDTH / 2.0;
    for end in [progress.first(), progress.last()].into_iter().flatten() {
        let dot = Bounds::new(point(end.x - px(half), end.y - px(half)), size(px(LINE_WIDTH), px(LINE_WIDTH)));
        window.paint_quad(fill(dot, tint).corner_radii(px(half)));
    }
}

/// moves the camera to the reader's own pin
/// sits where the system location button would since the pin is the reader's avatar
#[derive(IntoElement)]
pub struct MapLocateButton {
    action: PressAction,
}

impl MapLocateButton {
    pub fn new(action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { action: Rc::new(action) }
    }
}

impl RenderOnce for MapLocateButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = rise_ui::theme(cx).text.primary;
        let action = self.action;
        plain(ElementId::Name("map-locate-button".into())).on_press(move |window, cx| action(window, cx)).child(
            div()
                .id(ElementId::Name("map-locate-face".into()))
                .tooltip_label(tr("map_locate_me"))
                .size(px(BUTTON_SIDE))
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("location.fill", px(17.), color, true))
                .liquid_chrome_capsule(),
        )
    }
}

/// mounted at the root beside the other full screen sheets
/// inside the map page it would be clipped and sit under the tab bar
pub struct MapPhotoPickerHost {
    interactions: Entity<MapInteractionsStore>,
    theme_store: Entity<ThemeStore>,
    _interactions_observation: Subscription,
}

impl MapPhotoPickerHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let interactions = MapInteractionsStore::shared(cx);
        let interactions_observation = cx.observe(&interactions, |_, _, cx| cx.notify());
        Self { interactions, theme_store: ThemeStore::shared(cx), _interactions_observation: interactions_observation }
    }
}

impl Render for MapPhotoPickerHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.interactions.read(cx).photo_picker_visible {
            return div().into_any_element();
        }
        let visible = Binding::entity(
            self.interactions.clone(),
            |interactions| interactions.photo_picker_visible,
            |interactions, value, cx| interactions.set_photo_picker_visible(value, cx),
        );
        let caption = Binding::entity(
            self.interactions.clone(),
            |interactions| interactions.photo_caption.clone(),
            |interactions, value, _| interactions.photo_caption = value,
        );
        let complete = self.interactions.clone();
        div()
            .size_full()
            .child(
                MediaPickerSheet::new(
                    "map-photo-picker",
                    visible,
                    MediaPickerConfiguration::map_grid(),
                    self.theme_store.clone(),
                    move |result: MediaPickerResult, _: &mut Window, cx: &mut App| {
                        complete.update(cx, |interactions, cx| interactions.set_photo_picker_visible(false, cx));
                        let items: Vec<_> = result
                            .items
                            .into_iter()
                            .filter(|item| item.kind != MediaPickerItemKind::File && !item.is_file)
                            .collect();
                        if result.was_cancelled || items.is_empty() {
                            return;
                        }
                        let store = complete.clone();
                        cx.defer(move |cx| store.update(cx, |interactions, cx| interactions.add_photos(items, cx)));
                    },
                )
                .caption(Some(caption))
                .experimental_bottom_sheet(true),
            )
            .into_any_element()
    }
}
