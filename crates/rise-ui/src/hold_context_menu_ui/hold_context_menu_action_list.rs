use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, ElementId, Entity, FocusHandle, FontWeight, Global, Hsla, InteractiveElement, IntoElement, KeyBinding,
    ObjectFit, ParentElement, Pixels, RenderOnce, StatefulInteractiveElement, Styled, StyledImage, Window, actions,
    canvas, div, fill, hsla, img, px, rgb, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::selection_changed;

use crate::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use crate::hold_context_menu_ui::hold_context_menu_frame_keys::{
    HoldContextMenuActionFramesKey, HoldContextMenuHitTarget, HoldContextMenuHitTargets, HoldContextMenuHitTargetsKey,
    HoldContextMenuRowLocalFramesKey,
};
use crate::hold_context_menu_ui::hold_context_menu_glass::{HoldContextMenuGlass, HoldContextMenuShadow};
use crate::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use crate::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use crate::IconUi;
use crate::swiftui::animated::Animated;
use crate::swiftui::environment::{EnvKey, env, reduce_motion};
use crate::swiftui::font::{Font, FontDesign, SwiftUiFont};
use crate::swiftui::measure::measure;
use crate::swiftui::motion::{MotionSpec, Spring, ease_out};
use crate::swiftui::shapes::{Shape, centred_stroke};

const LEADING_IMAGE_SIDE: f32 = 28.0;
const SUBTITLE_FONT_SIZE: f32 = 13.0;

/// ink for row titles, icons separators and the highlight white over the glass panel
/// a flat panel takes the theme text color
pub struct HoldContextMenuInkKey;

impl EnvKey for HoldContextMenuInkKey {
    type Value = Hsla;

    fn default_value() -> Self::Value {
        white()
    }
}

pub fn hold_context_menu_ink(cx: &App) -> Hsla {
    env::<HoldContextMenuInkKey>(cx)
}

pub const KEY_CONTEXT: &str = "HoldContextMenu";

actions!(
    hold_context_menu,
    [
        ActivateHighlighted,
        DismissMenu,
        HighlightFirst,
        HighlightLast,
        HighlightNext,
        HighlightPrevious,
    ]
);

struct KeyBindingsInstalled;

impl Global for KeyBindingsInstalled {}

/// keyboard for an open menu, arrows Home and End move the highlight
/// enter lifts the highlighted row and Escape closes, safe to call twice
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);

    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("down", HighlightNext, context),
        KeyBinding::new("up", HighlightPrevious, context),
        KeyBinding::new("home", HighlightFirst, context),
        KeyBinding::new("end", HighlightLast, context),
        KeyBinding::new("enter", ActivateHighlighted, context),
        KeyBinding::new("escape", DismissMenu, context),
    ]);
}

fn destructive_ink() -> Hsla {
    rgb(0xff5959).into()
}

fn with_opacity(color: Hsla, opacity: f32) -> Hsla {
    hsla(color.h, color.s, color.l, color.a * opacity)
}

/// the travelling highlight, one capsule for the whole list in the list own space
struct HoldContextMenuActionListState {
    row_frames: HashMap<String, Bounds<Pixels>>,
    window_row_frames: HashMap<String, Bounds<Pixels>>,
    list_frame: Option<Bounds<Pixels>>,
    window_action_frames: HashMap<String, Bounds<Pixels>>,
    highlight_frame: Animated<Bounds<Pixels>>,
    highlight_shown: bool,
    highlight_opacity: Animated<f32>,
    applied_highlight_id: Option<String>,
    appeared: bool,
}

impl HoldContextMenuActionListState {
    fn new() -> Self {
        Self {
            row_frames: HoldContextMenuRowLocalFramesKey::default_value(),
            window_row_frames: HashMap::new(),
            list_frame: None,
            window_action_frames: HoldContextMenuActionFramesKey::default_value(),
            highlight_frame: Animated::new(Bounds::default()),
            highlight_shown: false,
            highlight_opacity: Animated::new(0.0),
            applied_highlight_id: None,
            appeared: false,
        }
    }

    /// single writer of the highlight geometry
    /// a hidden capsule is placed on the target row first and faded in so it never flies in
    /// a disabled row counts as no row
    fn move_highlight(
        &mut self,
        to_id: Option<&str>,
        animated: bool,
        actions: &[HoldContextMenuAction],
        now: std::time::Duration,
        reduce_motion: bool,
    ) {
        let frame = to_id
            .and_then(|id| actions.iter().find(|action| action.id == id))
            .filter(|action| !action.disabled)
            .and_then(|action| self.row_frames.get(&action.id).copied())
            .filter(|frame| frame.size.height > Pixels::ZERO);
        let fade = ease_out(HoldContextMenuMetrics::HIGHLIGHT_FADE_DURATION);

        let Some(frame) = frame else {
            if !self.highlight_shown {
                return;
            }
            self.highlight_shown = false;
            self.highlight_opacity.set(0.0, fade, now, false);
            return;
        };

        if !self.highlight_shown {
            self.place_highlight(frame);
            self.highlight_shown = true;
            self.highlight_opacity.set(1.0, fade, now, false);
            return;
        }

        if *self.highlight_frame.target() == frame {
            return;
        }
        if !animated || reduce_motion {
            self.place_highlight(frame);
            return;
        }
        let travel = MotionSpec::spring(Spring::new(
            HoldContextMenuMetrics::HIGHLIGHT_TRAVEL_RESPONSE,
            HoldContextMenuMetrics::HIGHLIGHT_TRAVEL_DAMPING,
        ));
        self.highlight_frame.set(frame, travel, now, false);
    }

    /// rows moved (submenu swap or panel resize) so the highlight follows without animating
    /// not a pointer change
    fn apply_measured_frames(
        &mut self,
        actions: &[HoldContextMenuAction],
        now: std::time::Duration,
        reduce_motion: bool,
    ) -> bool {
        let Some(list_frame) = self.list_frame else {
            return false;
        };
        let mut frames = HoldContextMenuRowLocalFramesKey::default_value();
        for action in actions {
            if let Some(row) = self.window_row_frames.get(&action.id) {
                let local = Bounds { origin: row.origin - list_frame.origin, size: row.size };
                HoldContextMenuRowLocalFramesKey::reduce(&mut frames, HashMap::from([(action.id.clone(), local)]));
            }
        }
        if self.row_frames == frames {
            return false;
        }
        self.row_frames = frames;
        if self.appeared {
            let current = self.applied_highlight_id.clone();
            self.move_highlight(current.as_deref(), false, actions, now, reduce_motion);
        }
        true
    }

    /// a bare write never inherits a running transition like panel entrance or submenu swap
    fn place_highlight(&mut self, frame: Bounds<Pixels>) {
        self.highlight_frame.jump(frame);
    }
}

#[derive(IntoElement)]
pub struct HoldContextMenuActionList {
    id: ElementId,
    pub actions: Vec<HoldContextMenuAction>,
    pub publishes_interaction_frames: bool,
    pub hit_targets: Option<HoldContextMenuHitTargets>,
    pub chrome: bool,
    pub max_width: f32,
    pub row_line_limit: usize,
    pub highlighted_action_id: Option<String>,
    pub on_select: Rc<dyn Fn(String, &mut Window, &mut App)>,
    pub on_highlight: Option<Rc<dyn Fn(Option<String>, &mut Window, &mut App)>>,
    pub on_action_frames: Option<Rc<dyn Fn(HashMap<String, Bounds<Pixels>>, &mut Window, &mut App)>>,
    pub on_dismiss: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    pub focus_handle: Option<FocusHandle>,
}

impl HoldContextMenuActionList {
    /// id keys the highlight state, which lives as long as the list is drawn
    pub fn new(
        id: impl Into<ElementId>,
        actions: Vec<HoldContextMenuAction>,
        highlighted_action_id: Option<String>,
        on_select: Rc<dyn Fn(String, &mut Window, &mut App)>,
    ) -> Self {
        Self {
            id: id.into(),
            actions,
            publishes_interaction_frames: true,
            hit_targets: None,
            chrome: true,
            max_width: HoldContextMenuMetrics::MENU_MAX_WIDTH,
            row_line_limit: 1,
            highlighted_action_id,
            on_select,
            on_highlight: None,
            on_action_frames: None,
            on_dismiss: None,
            focus_handle: None,
        }
    }

    pub fn publishes_interaction_frames(mut self, publishes_interaction_frames: bool) -> Self {
        self.publishes_interaction_frames = publishes_interaction_frames;
        self
    }

    pub fn hit_targets(mut self, hit_targets: Option<HoldContextMenuHitTargets>) -> Self {
        self.hit_targets = hit_targets;
        self
    }

    pub fn chrome(mut self, chrome: bool) -> Self {
        self.chrome = chrome;
        self
    }

    pub fn max_width(mut self, max_width: f32) -> Self {
        self.max_width = max_width;
        self
    }

    pub fn row_line_limit(mut self, row_line_limit: usize) -> Self {
        self.row_line_limit = row_line_limit;
        self
    }

    pub fn on_highlight(mut self, on_highlight: Rc<dyn Fn(Option<String>, &mut Window, &mut App)>) -> Self {
        self.on_highlight = Some(on_highlight);
        self
    }

    pub fn on_action_frames(
        mut self,
        on_action_frames: Rc<dyn Fn(HashMap<String, Bounds<Pixels>>, &mut Window, &mut App)>,
    ) -> Self {
        self.on_action_frames = Some(on_action_frames);
        self
    }

    pub fn on_dismiss(mut self, on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>) -> Self {
        self.on_dismiss = Some(on_dismiss);
        self
    }

    pub fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    /// enabled row step rows away from the highlighted one, clamped at the ends
    /// with no highlight the first step lands on the first or last enabled row
    fn neighbour(actions: &[HoldContextMenuAction], highlighted: Option<&str>, step: isize) -> Option<String> {
        let enabled: Vec<&HoldContextMenuAction> = actions.iter().filter(|action| !action.disabled).collect();
        let last = enabled.len().checked_sub(1)?;
        let current = highlighted.and_then(|id| enabled.iter().position(|action| action.id == id));
        let index = match current {
            Some(index) => index.saturating_add_signed(step).min(last),
            None if step >= 0 => 0,
            None => last,
        };
        enabled.get(index).map(|action| action.id.clone())
    }

    fn action_row(
        &self,
        action: &HoldContextMenuAction,
        ink: Hsla,
        state: &Entity<HoldContextMenuActionListState>,
        actions: &Rc<Vec<HoldContextMenuAction>>,
        reduce_motion: bool,
    ) -> impl IntoElement {
        let text_color = if action.destructive { destructive_ink() } else { ink };
        let icon_color = if action.destructive { destructive_ink() } else { with_opacity(ink, 0.94) };
        let content_opacity = if action.disabled { 0.42 } else { 1.0 };
        let icon_side = px(HoldContextMenuMetrics::MENU_ROW_ICON_SIZE);

        let mut label = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .px(px(HoldContextMenuMetrics::MENU_ROW_HORIZONTAL_PADDING))
            .min_h(px(HoldContextMenuMetrics::MENU_ROW_HEIGHT))
            .relative();
        if let Some(picture) = action.leading_image.clone() {
            let picture = img(picture).object_fit(ObjectFit::Cover).size(px(LEADING_IMAGE_SIDE));
            let picture = match action.leading_image_corner_radius {
                Some(radius) => div().rounded(px(radius)),
                None => div().rounded_full(),
            }
            .size(px(LEADING_IMAGE_SIDE))
            .overflow_hidden()
            .child(picture);
            label = label.child(
                div()
                    .flex()
                    .justify_center()
                    .w(px(HoldContextMenuMetrics::MENU_ROW_ICON_SLOT_WIDTH))
                    .opacity(content_opacity)
                    .child(picture),
            );
        } else if let Some(symbol) = action.system_image.as_deref() {
            label = label.child(
                div()
                    .flex()
                    .justify_center()
                    .w(px(HoldContextMenuMetrics::MENU_ROW_ICON_SLOT_WIDTH))
                    .opacity(content_opacity)
                    .children(IconUi::sized(symbol, icon_side, icon_color, false)),
            );
        }
        label = label.child(
            div()
                .flex_auto()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(1.0))
                .opacity(content_opacity)
                .child(
                    div()
                        .w_full()
                        .swiftui_font(Font::system(
                            HoldContextMenuMetrics::MENU_ROW_FONT_SIZE,
                            FontWeight::NORMAL,
                            FontDesign::Default,
                        ))
                        .text_color(text_color)
                        .line_clamp(self.row_line_limit)
                        .text_ellipsis()
                        .child(action.title.clone()),
                )
                .children(action.subtitle.clone().map(|subtitle| {
                    div()
                        .w_full()
                        .swiftui_font(Font::system(SUBTITLE_FONT_SIZE, FontWeight::NORMAL, FontDesign::Default))
                        .text_color(with_opacity(ink, 0.55))
                        .line_clamp(1)
                        .text_ellipsis()
                        .child(subtitle)
                })),
        );
        if action.checked {
            label = label.child(
                div()
                    .flex_none()
                    .children(IconUi::sized("checkmark", icon_side, icon_color, false)),
            );
        }
        if self.publishes_interaction_frames {
            let state = state.clone();
            let actions = actions.clone();
            let id = action.id.clone();
            label = label.child(measure(
                ElementId::Name(format!("hold-menu-row-local-{}", action.id).into()),
                move |bounds, _, cx| {
                    state.update(cx, |state, cx| {
                        state.window_row_frames.retain(|key, _| actions.iter().any(|action| &action.id == key));
                        state.window_row_frames.insert(id, bounds);
                        if state.apply_measured_frames(&actions, RiseClock::monotonic(), reduce_motion) {
                            cx.notify();
                        }
                    });
                },
            ));
        }
        let guide_anchor = if self.publishes_interaction_frames { action.guide_anchor } else { None };

        let mut row = div()
            .id(ElementId::Name(format!("hold-menu-row-{}", action.id).into()))
            .relative()
            .px(px(8.0))
            .child(label.guide_anchor(guide_anchor));
        if self.publishes_interaction_frames {
            if let Some(hit_targets) = self.hit_targets.clone() {
                row = row.child(HoldContextMenuHitTarget::new(
                    hit_targets,
                    HoldContextMenuHitTargetsKey::Action(action.id.clone()),
                ));
            } else if let Some(on_action_frames) = self.on_action_frames.clone() {
                let state = state.clone();
                let actions = actions.clone();
                let id = action.id.clone();
                row = row.child(measure(
                    ElementId::Name(format!("hold-menu-row-global-{}", action.id).into()),
                    move |bounds, window, cx| {
                        let frames = state.update(cx, |state, _| {
                            state.window_action_frames.retain(|key, _| actions.iter().any(|action| &action.id == key));
                            HoldContextMenuActionFramesKey::reduce(
                                &mut state.window_action_frames,
                                HashMap::from([(id, bounds)]),
                            );
                            state.window_action_frames.clone()
                        });
                        on_action_frames(frames, window, cx);
                    },
                ));
            }
        }
        if action.disabled {
            return row;
        }

        let selected_id = action.id.clone();
        let on_select = self.on_select.clone();
        row = row.cursor_pointer().on_click(move |_, window, cx| {
            let _ = selection_changed();
            on_select(selected_id.clone(), window, cx);
        });
        if let Some(on_highlight) = self.on_highlight.clone() {
            let hovered_id = action.id.clone();
            let highlighted = self.highlighted_action_id.clone();
            row = row.on_hover(move |hovered, window, cx| {
                if *hovered {
                    on_highlight(Some(hovered_id.clone()), window, cx);
                } else if highlighted.as_deref() == Some(hovered_id.as_str()) {
                    on_highlight(None, window, cx);
                }
            });
        }
        row
    }

    fn rows(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ink = hold_context_menu_ink(cx);
        let now = RiseClock::monotonic();
        let reduce_motion = reduce_motion(cx);
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| HoldContextMenuActionListState::new());
        let actions = Rc::new(self.actions.clone());

        let mut list = div()
            .relative()
            .flex()
            .flex_col()
            .py(px(5.0))
            .min_w(px(HoldContextMenuMetrics::MENU_MIN_WIDTH))
            .max_w(px(self.max_width + HoldContextMenuMetrics::MENU_DESKTOP_WIDTH_SLACK))
            .key_context(KEY_CONTEXT);
        if let Some(focus_handle) = self.focus_handle.as_ref() {
            list = list.track_focus(focus_handle);
        }

        if self.publishes_interaction_frames {
            let state = state.clone();
            let actions = actions.clone();
            list = list.child(measure(
                ElementId::NamedChild(Arc::new(self.id.clone()), "row-space".into()),
                move |bounds, _, cx| {
                    state.update(cx, |state, cx| {
                        state.list_frame = Some(bounds);
                        if state.apply_measured_frames(&actions, RiseClock::monotonic(), reduce_motion) {
                            cx.notify();
                        }
                    });
                },
            ));
        }

        {
            let state = state.clone();
            let actions = actions.clone();
            let highlighted = self.highlighted_action_id.clone();
            let highlight_color = with_opacity(ink, HoldContextMenuMetrics::MENU_ROW_HIGHLIGHT_OPACITY as f32);
            list = list.child(
                canvas(
                    |_, _, _| {},
                    move |list_bounds, _, window, cx| {
                        let (frame, opacity) = state.update(cx, |state, _| {
                            if !state.appeared {
                                state.appeared = true;
                                state.applied_highlight_id = highlighted.clone();
                                state.move_highlight(highlighted.as_deref(), false, &actions, now, reduce_motion);
                            } else if state.applied_highlight_id != highlighted {
                                state.applied_highlight_id = highlighted.clone();
                                state.move_highlight(highlighted.as_deref(), true, &actions, now, reduce_motion);
                            }
                            (state.highlight_frame.frame_value(now, window), state.highlight_opacity.frame_value(now, window))
                        });
                        if opacity <= 0.0 || frame.size.height <= Pixels::ZERO {
                            return;
                        }
                        let bounds = Bounds { origin: list_bounds.origin + frame.origin, size: frame.size };
                        let radius = if frame.size.height < frame.size.width { frame.size.height } else { frame.size.width } / 2.0;
                        window.paint_quad(
                            fill(bounds, with_opacity(highlight_color, opacity)).corner_radii(radius),
                        );
                    },
                )
                .absolute()
                .inset_0(),
            );
        }

        for (idx, action) in self.actions.iter().enumerate() {
            if idx > 0 && action.section_break {
                list = list.child(
                    div()
                        .px(px(HoldContextMenuMetrics::MENU_SECTION_SEPARATOR_INSET))
                        .py(px(HoldContextMenuMetrics::MENU_SECTION_SEPARATOR_GAP))
                        .child(
                            div()
                                .h(px(0.66))
                                .bg(with_opacity(ink, HoldContextMenuMetrics::MENU_SECTION_SEPARATOR_OPACITY as f32)),
                        ),
                );
            }
            list = list.child(self.action_row(action, ink, &state, &actions, reduce_motion));
        }

        let highlighted = self.highlighted_action_id.clone();
        if let Some(on_highlight) = self.on_highlight.clone() {
            let key_actions = actions.clone();
            let keyboard_highlight = move |step: isize, from: Option<&str>, window: &mut Window, cx: &mut App| {
                on_highlight(Self::neighbour(&key_actions, from, step), window, cx);
            };
            let keyboard_highlight = Rc::new(keyboard_highlight);
            let (next, previous, first, last) = (
                keyboard_highlight.clone(),
                keyboard_highlight.clone(),
                keyboard_highlight.clone(),
                keyboard_highlight,
            );
            let (next_from, previous_from) = (highlighted.clone(), highlighted.clone());
            list = list
                .on_action(move |_: &HighlightNext, window, cx| next(1, next_from.as_deref(), window, cx))
                .on_action(move |_: &HighlightPrevious, window, cx| previous(-1, previous_from.as_deref(), window, cx))
                .on_action(move |_: &HighlightFirst, window, cx| first(1, None, window, cx))
                .on_action(move |_: &HighlightLast, window, cx| last(-1, None, window, cx));
        }
        {
            let on_select = self.on_select.clone();
            let actions = actions.clone();
            let highlighted = highlighted.clone();
            list = list.on_action(move |_: &ActivateHighlighted, window, cx| {
                let Some(id) = highlighted.as_deref() else { return };
                if actions.iter().any(|action| action.id == id && !action.disabled) {
                    let _ = selection_changed();
                    on_select(id.to_string(), window, cx);
                }
            });
        }
        if let Some(on_dismiss) = self.on_dismiss.clone() {
            list = list.on_action(move |_: &DismissMenu, window, cx| on_dismiss(window, cx));
        }
        list
    }
}

impl RenderOnce for HoldContextMenuActionList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.chrome {
            return self.rows(window, cx).into_any_element();
        }
        let corner_radius = HoldContextMenuMetrics::MENU_CORNER_RADIUS;
        let glass_id = ElementId::NamedChild(Arc::new(self.id.clone()), "glass".into());
        let stroke = with_opacity(white(), HoldContextMenuMetrics::PANEL_STROKE_OPACITY as f32);
        div()
            .relative()
            .child(HoldContextMenuShadow::new(corner_radius))
            .child(
                div()
                    .relative()
                    .rounded(px(corner_radius))
                    .overflow_hidden()
                    .child(HoldContextMenuGlass::new(glass_id).corner_radius(corner_radius).opaque(true))
                    .child(self.rows(window, cx)),
            )
            .child(centred_stroke(stroke, 1.0, Shape::continuous(corner_radius)))
            .into_any_element()
    }
}
