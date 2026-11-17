use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Context, Edges, ElementId, Entity, IntoElement, Pixels, Render, RenderOnce, Subscription,
    Window, black, div, point, prelude::*, px, size,
};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::{ScenePhase, scene_phase};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes};

use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::modules::camera::capture::rise_camera_presenter::{
    RiseCameraPresenter, RiseCameraPresenterStage, RiseCameraStoryComposition,
};
use crate::modules::camera::components::rise_camera_screen::RiseCameraScreen;
use crate::modules::media_picker::export::media_picker_export_progress_overlay::MediaPickerExportProgressOverlay;
use crate::modules::story::components::story_create_sheet::StoryCreateSheet;

const PICKER_DIM_OPACITY: f32 = 0.55;
const TILE_CORNER_RADIUS: f32 = 12.0;

pub struct RiseCameraHost {
    windowed: bool,
    presenter: Entity<RiseCameraPresenter>,
    screen: Option<Entity<RiseCameraScreen>>,
    scene_phase: Option<ScenePhase>,
    entrance_reported: bool,
    container: Rc<Cell<Bounds<Pixels>>>,
    _presenter_observation: Subscription,
}

impl RiseCameraHost {
    pub fn new(windowed: bool, cx: &mut Context<Self>) -> Self {
        let presenter = RiseCameraPresenter::shared(cx);
        let presenter_observation = cx.observe(&presenter, |_, _, cx| cx.notify());
        Self {
            windowed,
            presenter,
            screen: None,
            scene_phase: None,
            entrance_reported: false,
            container: Rc::new(Cell::new(Bounds::default())),
            _presenter_observation: presenter_observation,
        }
    }

    fn scene_phase_changed(&mut self, phase: ScenePhase, cx: &mut Context<Self>) {
        let previous = self.scene_phase.replace(phase);
        if previous.is_none() || previous == Some(phase) {
            return;
        }
        let presenter = self.presenter.clone();
        cx.defer(move |cx| {
            let (on_screen, at_camera, capture) = {
                let presenter = presenter.read(cx);
                (
                    presenter.is_on_screen(),
                    matches!(presenter.stage(), RiseCameraPresenterStage::Camera),
                    presenter.capture.clone(),
                )
            };
            if !on_screen {
                return;
            }
            match phase {
                ScenePhase::Active => {
                    if at_camera {
                        capture.update(cx, |capture, cx| capture.resume(cx));
                    }
                }
                ScenePhase::Background => capture.update(cx, |capture, cx| capture.suspend(cx)),
                ScenePhase::Inactive => {}
            }
        });
    }

    fn stage_layer(&self, cx: &mut App) -> Option<AnyElement> {
        let (export_progress, composition) = {
            let presenter = self.presenter.read(cx);
            match presenter.stage() {
                RiseCameraPresenterStage::Camera => return None,
                RiseCameraPresenterStage::Exporting(_) => (Some(presenter.export_progress.clone()), None),
                RiseCameraPresenterStage::Story(composition) => (None, Some(composition.clone())),
            }
        };
        if let Some(state) = export_progress {
            let theme_store = ThemeStore::shared(cx);
            return Some(
                div()
                    .absolute()
                    .inset_0()
                    .bg(black())
                    .child(MediaPickerExportProgressOverlay::new(state, theme_store))
                    .into_any_element(),
            );
        }
        let composition = composition?;
        let presenter = self.presenter.clone();
        Some(
            RiseCameraStorySheet {
                composition,
                on_closed: Rc::new(move |published, cx| {
                    presenter.update(cx, |presenter, cx| presenter.story_sheet_closed(published, cx));
                }),
            }
            .into_any_element(),
        )
    }
}

impl Render for RiseCameraHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.scene_phase_changed(scene_phase(cx), cx);
        let container_cell = self.container.clone();
        let host = div().absolute().inset_0().child(measure("rise-camera-host-size", move |bounds, window, _| {
            if container_cell.get() != bounds {
                container_cell.set(bounds);
                window.refresh();
            }
        }));
        let (shows, progress, is_open, is_from_picker, origin, window_safe_area, settling_closed) = {
            let presenter = self.presenter.read(cx);
            (
                presenter.is_on_screen() && presenter.is_windowed() == self.windowed,
                presenter.presented_progress(window),
                presenter.is_open(),
                presenter.is_from_picker(),
                presenter.origin_rect(),
                presenter.window_safe_area(),
                presenter.is_settling_closed(),
            )
        };
        if !shows {
            self.screen = None;
            self.entrance_reported = false;
            return host.into_any_element();
        }
        let container = self.container.get();
        let width = f32::from(container.size.width).max(1.0);
        let frame_padding = if self.windowed { window_safe_area } else { Edges::default() };
        let presenter = self.presenter.clone();
        let screen = self
            .screen
            .get_or_insert_with(|| cx.new(|cx| RiseCameraScreen::new(presenter, width, cx)))
            .clone();
        screen.update(cx, |screen, cx| screen.set_screen_width(width, cx));
        if !self.entrance_reported {
            self.entrance_reported = true;
            let presenter = self.presenter.clone();
            window.on_next_frame(move |_, cx| {
                presenter.update(cx, |presenter, cx| presenter.entrance_did_mount(cx));
            });
        }

        let dim_opacity = if is_from_picker && origin.is_none() { PICKER_DIM_OPACITY * progress } else { 0.0 };
        let dim = div().absolute().inset_0().bg(black().opacity(dim_opacity));

        let content = div()
            .size_full()
            .bg(black())
            .pt(frame_padding.top)
            .pb(frame_padding.bottom)
            .pl(frame_padding.left)
            .pr(frame_padding.right)
            .child(screen);
        let entrance = RiseCameraEntrance {
            progress,
            width,
            origin,
            container,
        }
        .body(content.into_any_element());

        host.child(dim)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(entrance)
                    .children((!is_open || settling_closed).then(|| {
                        div().id("rise-camera-host-moving").absolute().inset_0().occlude()
                    }))
                    .occludes_bottom_chrome(),
            )
            .children(self.stage_layer(cx))
            .into_any_element()
    }
}

/// moves the app screen with the feed camera, they stand side by side
/// feed leaves to the right as the camera comes in from the left, only this wrapper reads progress
#[derive(IntoElement)]
pub struct RiseCameraFeedShift {
    content: AnyElement,
}

impl RenderOnce for RiseCameraFeedShift {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let width = window
            .use_keyed_state("rise-camera-feed-shift-width", cx, |_, _| Rc::new(Cell::new(0.0_f32)))
            .read(cx)
            .clone();
        let presenter = RiseCameraPresenter::shared(cx);
        let presenter = presenter.read(cx);
        let progress = if presenter.is_on_screen() && !presenter.is_from_picker() && !presenter.is_windowed() {
            presenter.presented_progress(window)
        } else {
            0.0
        };
        let measured = width.clone();
        div()
            .relative()
            .size_full()
            .left(px(progress * width.get()))
            .child(self.content)
            .child(measure("rise-camera-feed-shift-size", move |bounds, window, _| {
                let width = f32::from(bounds.size.width);
                if measured.get() != width {
                    measured.set(width);
                    window.refresh();
                }
            }))
    }
}

pub trait RiseCameraFeedShiftModifiers: IntoElement + Sized {
    fn rise_camera_feed_shift(self) -> RiseCameraFeedShift {
        RiseCameraFeedShift { content: self.into_any_element() }
    }
}

impl<E: IntoElement> RiseCameraFeedShiftModifiers for E {}

struct RiseCameraEntrance {
    progress: f32,
    width: f32,
    origin: Option<Bounds<Pixels>>,
    container: Bounds<Pixels>,
}

impl RiseCameraEntrance {
    fn body(self, content: AnyElement) -> AnyElement {
        let Some(origin) = self.origin else {
            return div()
                .absolute()
                .inset_0()
                .left(px((self.progress - 1.0) * self.width))
                .child(content)
                .into_any_element();
        };
        let p = self.progress.clamp(0.0, 1.0);
        let rect = Self::window_rect(origin, self.container, p);
        div()
            .absolute()
            .left(rect.origin.x)
            .top(rect.origin.y)
            .w(rect.size.width)
            .h(rect.size.height)
            .clip_shape(Shape::continuous(TILE_CORNER_RADIUS * (1.0 - p)))
            .child(content)
            .into_any_element()
    }

    /// local rect of the window at p, from the tile (0) to the whole host (1)
    /// every value is linear in p so one spring drives them all
    fn window_rect(origin: Bounds<Pixels>, container: Bounds<Pixels>, p: f32) -> Bounds<Pixels> {
        let start_x = f32::from(origin.origin.x - container.origin.x);
        let start_y = f32::from(origin.origin.y - container.origin.y);
        let start_w = f32::from(origin.size.width);
        let start_h = f32::from(origin.size.height);
        let end_w = f32::from(container.size.width);
        let end_h = f32::from(container.size.height);
        Bounds {
            origin: point(px(start_x - start_x * p), px(start_y - start_y * p)),
            size: size(px(start_w + (end_w - start_w) * p), px(start_h + (end_h - start_h) * p)),
        }
    }
}

#[derive(IntoElement)]
struct RiseCameraStorySheet {
    composition: RiseCameraStoryComposition,
    on_closed: Rc<dyn Fn(bool, &mut App)>,
}

#[derive(Clone, Copy)]
struct RiseCameraStorySheetState {
    is_presented: bool,
    did_publish: bool,
}

impl RenderOnce for RiseCameraStorySheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.composition.id.uuid_string();
        let state = window.use_keyed_state(
            ElementId::Name(format!("rise-camera-story-sheet-{id}").into()),
            cx,
            |_, _| RiseCameraStorySheetState { is_presented: true, did_publish: false },
        );
        let read_state = state.clone();
        let write_state = state.clone();
        let on_closed = self.on_closed.clone();
        let is_presented = Binding::new(
            move |cx: &App| read_state.read(cx).is_presented,
            move |presented: bool, _: &mut Window, cx: &mut App| {
                let was_presented = write_state.read(cx).is_presented;
                write_state.update(cx, |state, cx| {
                    state.is_presented = presented;
                    cx.notify();
                });
                if !was_presented || presented {
                    return;
                }
                let state = write_state.clone();
                let on_closed = on_closed.clone();
                cx.defer(move |cx| {
                    let published = state.read(cx).did_publish;
                    on_closed(published, cx);
                });
            },
        );
        let published_state = state.clone();
        StoryCreateSheet::new(
            ElementId::Name(format!("rise-camera-story-create-{id}").into()),
            self.composition.item.clone(),
            "",
            is_presented,
            move |_, cx| {
                published_state.update(cx, |state, cx| {
                    state.did_publish = true;
                    cx.notify();
                });
            },
        )
        .initial_track(None)
    }
}
