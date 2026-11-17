use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, BoxShadow, Bounds, ElementId, FocusHandle, Hsla, InteractiveElement, IntoElement, ParentElement, PathBuilder,
    Pixels, Point, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, black, canvas, div, fill,
    point, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::IconUi;
use rise_ui::keyboard_dismiss::LayerFocus;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::motion::linear;
use rise_ui::swiftui::transition::Transition;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};

use crate::modules::chat::components::chat::video_note::video_note_player_layer_view::VideoNotePlayerLayerView;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;

pub type ChatVideoNoteExpandedPlayerDismiss = Rc<dyn Fn(&mut Window, &mut App)>;

const BACKDROP_OPACITY: f32 = 0.55;
const PROGRESS_LINE_WIDTH: f32 = 4.0;
const PROGRESS_MOTION_DURATION: f64 = 0.05;
const PLAY_BADGE_SIZE: f32 = 64.0;
const PLAY_BADGE_OPACITY: f32 = 0.4;
const PLAY_ICON_SIZE: f32 = 24.0;
const PLAY_ICON_OFFSET_X: f32 = 2.0;
const SHADOW_OPACITY: f32 = 0.35;
const SHADOW_RADIUS: f32 = 20.0;
const SHADOW_OFFSET_Y: f32 = 8.0;
const TRANSITION_SCALE: f32 = 0.3;

struct ChatVideoNoteExpandedPlayerState {
    focus_handle: FocusHandle,
    progress: Animated<f32>,
}

#[derive(IntoElement)]
pub struct ChatVideoNoteExpandedPlayer {
    id: ElementId,
    on_dismiss: Option<ChatVideoNoteExpandedPlayerDismiss>,
}

impl ChatVideoNoteExpandedPlayer {
    pub const MAXIMUM_EXPANDED_SIZE: f32 = 320.0;
    pub const EXPANDED_SIDE_INSET: f32 = 32.0;

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            on_dismiss: None,
        }
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(on_dismiss));
        self
    }

    /// circle size, fits the window width like it fits the screen
    pub fn expanded_size(viewport_width: f32) -> f32 {
        (viewport_width - Self::EXPANDED_SIDE_INSET).min(Self::MAXIMUM_EXPANDED_SIZE)
    }

    /// transition for the presenter presence
    pub fn transition() -> Transition {
        Transition::asymmetric(
            Transition::scale(TRANSITION_SCALE).combined(Transition::opacity()),
            Transition::scale(TRANSITION_SCALE).combined(Transition::opacity()),
        )
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(name))
    }
}

impl RenderOnce for ChatVideoNoteExpandedPlayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store = VideoNotePlayerStore::shared(cx);
        let (player, display_epoch, progress, is_playing) = {
            let store = store.read(cx);
            (store.player(), store.display_epoch(), store.progress(), store.is_playing())
        };
        let size = Self::expanded_size(f32::from(window.viewport_size().width));

        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| ChatVideoNoteExpandedPlayerState {
            focus_handle: cx.focus_handle(),
            progress: Animated::new(progress),
        });
        let now = RiseClock::monotonic();
        let shown_progress = state.update(cx, |state, cx| {
            state.progress.set_in(progress, linear(PROGRESS_MOTION_DURATION), now, cx);
            state.progress.frame_value(now, window)
        });
        let focus_handle = state.read(cx).focus_handle.clone();

        let backdrop_store = store.clone();
        let badge_store = store.clone();
        let on_dismiss = self.on_dismiss.clone();
        let layer_focus = LayerFocus::keyed(self.child_id("focus-return"), window, cx);
        let (appearing, leaving) = (layer_focus.clone(), layer_focus);
        let appear_focus = focus_handle.clone();
        let disappear_focus = focus_handle.clone();
        let progress_color = theme.primary._100;

        let circle = div()
            .relative()
            .size(px(size))
            .flex_none()
            .shadow(vec![BoxShadow {
                color: black().opacity(SHADOW_OPACITY),
                offset: point(Pixels::ZERO, px(SHADOW_OFFSET_Y)),
                blur_radius: px(SHADOW_RADIUS),
                spread_radius: Pixels::ZERO,
                inset: false,
            }])
            .rounded_full()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_full()
                    .overflow_hidden()
                    .child(VideoNotePlayerLayerView::new(player, display_epoch)),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        paint_progress_trim(bounds, shown_progress, PROGRESS_LINE_WIDTH, progress_color, window);
                    },
                )
                .absolute()
                .inset_0(),
            )
            .when(!is_playing, |circle| {
                circle.child(
                    div().absolute().inset_0().flex().items_center().justify_center().child(
                        div()
                            .id(self.child_id("play"))
                            .size(px(PLAY_BADGE_SIZE))
                            .rounded_full()
                            .bg(black().opacity(PLAY_BADGE_OPACITY))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                badge_store.update(cx, |store, cx| store.pause_or_resume(cx));
                            })
                            .child(
                                div()
                                    .relative()
                                    .left(px(PLAY_ICON_OFFSET_X))
                                    .children(IconUi::sized("play.fill", px(PLAY_ICON_SIZE), white(), true)),
                            ),
                    ),
                )
            });

        div()
            .id(self.child_id("root"))
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&focus_handle)
            .when_some(on_dismiss, |player, on_dismiss| {
                player.layer_keys(LayerCancel::dismiss(move |window, cx| on_dismiss(window, cx)))
            })
            .child(
                div()
                    .id(self.child_id("backdrop"))
                    .absolute()
                    .inset_0()
                    .bg(black().opacity(BACKDROP_OPACITY))
                    .on_click(move |_, _, cx| {
                        backdrop_store.update(cx, |store, cx| store.pause_or_resume(cx));
                    }),
            )
            .child(circle)
            .on_appear(self.child_id("appear"), move |window, cx| {
                LayerFocus::did_appear(&appearing, &appear_focus, window, cx);
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    LayerFocus::will_disappear(&leaving, &disappear_focus, window, cx);
                }
            })
    }
}

/// arc trimmed to progress with round caps, starts at twelve oclock and runs clockwise
fn paint_progress_trim(bounds: Bounds<Pixels>, progress: f32, line_width: f32, color: Hsla, window: &mut Window) {
    let fraction = progress.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return;
    }
    let center = bounds.center();
    let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0;
    let sweep = 2.0 * PI * fraction;
    let start = -PI / 2.0;
    let steps = ((fraction * 96.0).ceil() as usize).max(2);
    let points: Vec<Point<Pixels>> = (0..=steps)
        .map(|step| {
            let angle = start + sweep * step as f32 / steps as f32;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        })
        .collect();
    let mut builder = PathBuilder::stroke(px(line_width));
    builder.add_polygon(&points, false);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
    let half = line_width / 2.0;
    for end in [points.first(), points.last()].into_iter().flatten() {
        let cap = Bounds::new(point(end.x - px(half), end.y - px(half)), size(px(line_width), px(line_width)));
        window.paint_quad(fill(cap, color).corner_radii(px(half)));
    }
}
