use std::rc::Rc;

use gpui::{App, ElementId, Hsla, Rgba, SharedString, Window, white};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::ui_color_system::UIColor;
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::swiftui::angular_gradient::{AngularGradient, angular_gradient_circle_stroke};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::theme;

use crate::modules::media_picker::editor::editor_bottom_bar::editor_panel_escape;
use crate::modules::media_picker::editor::editor_state::EditorDrawStrokeTool;

/// callback for a picked color
pub type EditorBrushColorAction = Rc<dyn Fn(Hsla, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct EditorBrushPanel {
    pub tool: Binding<EditorDrawStrokeTool>,
    pub color: Hsla,
    pub width_fraction: Binding<f32>,
    pub on_pick_color: EditorBrushColorAction,
    pub on_close: ButtonAction,
    pub on_confirm: ButtonAction,
    pub on_select_text_mode: ButtonAction,
    pub on_select_stickers: ButtonAction,
    pub on_present_color_picker: ButtonAction,
    pub on_select_link: Option<ButtonAction>,
    pub allows_blur: bool,
}

impl EditorBrushPanel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tool: Binding<EditorDrawStrokeTool>,
        color: Hsla,
        width_fraction: Binding<f32>,
        on_pick_color: impl Fn(Hsla, &mut Window, &mut App) + 'static,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        on_confirm: impl Fn(&mut Window, &mut App) + 'static,
        on_select_text_mode: impl Fn(&mut Window, &mut App) + 'static,
        on_select_stickers: impl Fn(&mut Window, &mut App) + 'static,
        on_present_color_picker: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            tool,
            color,
            width_fraction,
            on_pick_color: Rc::new(on_pick_color),
            on_close: Rc::new(on_close),
            on_confirm: Rc::new(on_confirm),
            on_select_text_mode: Rc::new(on_select_text_mode),
            on_select_stickers: Rc::new(on_select_stickers),
            on_present_color_picker: Rc::new(on_present_color_picker),
            on_select_link: None,
            allows_blur: true,
        }
    }

    pub fn on_select_link(mut self, on_select_link: Option<ButtonAction>) -> Self {
        self.on_select_link = on_select_link;
        self
    }

    pub fn allows_blur(mut self, allows_blur: bool) -> Self {
        self.allows_blur = allows_blur;
        self
    }

    /// true for tools that draw with the selected color, eraser and blur ignore it
    pub fn paints_with_color(tool: EditorDrawStrokeTool) -> bool {
        tool != EditorDrawStrokeTool::Eraser && tool != EditorDrawStrokeTool::Blur
    }

    pub fn palette() -> [Hsla; 8] {
        let color = |r, g, b| Hsla::from(Rgba { r, g, b, a: 1. });
        [
            color(1.0, 0.27, 0.27),
            color(1.0, 0.55, 0.18),
            color(1.0, 0.85, 0.18),
            color(0.30, 0.85, 0.45),
            color(0.18, 0.85, 0.85),
            color(0.32, 0.6, 1.0),
            color(0.75, 0.45, 1.0),
            white(),
        ]
    }

    fn brushes_row(&self, selected_tool: EditorDrawStrokeTool) -> impl IntoElement {
        let color = self.color;
        let definitions = if self.allows_blur { EditorBrushDef::ALL.to_vec() } else { EditorBrushDef::without_blur() };
        let tool = self.tool.clone();
        hstack(0.).w_full().children(definitions.into_iter().enumerate().map(move |(index, definition)| {
            let is_selected = selected_tool == definition.tool;
            let tint = if !is_selected {
                white().opacity(0.62)
            } else if Self::paints_with_color(definition.tool) {
                color
            } else {
                white()
            };
            let tool = tool.clone();
            plain(("editor-brush", index))
                .on_press(move |window, cx| tool.set(definition.tool, window, cx))
                .flex_1()
                .min_h(px(50.))
                .flex()
                .items_center()
                .justify_center()
                .child(BrushIcon { index, definition, tint, is_selected })
        }))
    }

    fn colors_row(&self, cx: &App) -> impl IntoElement {
        let selected = self.color;
        let on_present_color_picker = self.on_present_color_picker.clone();
        let on_pick_color = self.on_pick_color.clone();
        hstack(0.)
            .w_full()
            .child(
                plain("editor-brush-rainbow")
                    .on_press(move |window, cx| on_present_color_picker(window, cx))
                    .flex_1()
                    .min_h(px(44.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Self::rainbow_dot(cx)),
            )
            .children(Self::palette().into_iter().enumerate().map(move |(index, dot)| {
                Self::color_dot(index, dot, dot.is_close_to(selected), on_pick_color.clone())
            }))
    }

    fn footer(&self) -> impl IntoElement {
        let on_close = self.on_close.clone();
        let on_confirm = self.on_confirm.clone();
        hstack(0.)
            .w_full()
            .child(
                LiquidGlassButton::new("editor-brush-close", LiquidGlassButtonShape::Circle)
                    .size(48.)
                    .accessibility_label(Some(tr("close")))
                    .on_press(move |window, cx| on_close(window, cx))
                    .children(IconUi::sized("xmark", px(16.), white(), false)),
            )
            .child(spacer_min_w(0.))
            .child(EditorDrawSegmentControl {
                on_stickers_selected: self.on_select_stickers.clone(),
                on_text_selected: self.on_select_text_mode.clone(),
                on_link_selected: self.on_select_link.clone(),
            })
            .child(spacer_min_w(0.))
            .child(
                LiquidGlassButton::new("editor-brush-confirm", LiquidGlassButtonShape::Circle)
                    .size(48.)
                    .accessibility_label(Some(tr("done")))
                    .on_press(move |window, cx| on_confirm(window, cx))
                    .children(IconUi::sized("checkmark", px(16.), white(), false)),
            )
    }

    fn color_dot(index: usize, dot: Hsla, is_selected: bool, on_pick_color: EditorBrushColorAction) -> PlainButton {
        let disc = div()
            .relative()
            .size(px(30.))
            .child(div().size_full().circle().bg(dot))
            .when(is_selected, |disc| {
                disc.child(
                    div()
                        .absolute()
                        .left(px(2.))
                        .top(px(2.))
                        .size(px(26.))
                        .child(centred_stroke(white(), 1.5, Shape::Circle)),
                )
            })
            .child(centred_stroke(white().opacity(0.45), 0.5, Shape::Circle));
        plain(("editor-brush-color", index))
            .on_press(move |window, cx| on_pick_color(dot, window, cx))
            .flex_1()
            .min_h(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .child(disc)
    }

    fn rainbow_dot(cx: &App) -> impl IntoElement {
        let appearance = theme(cx).appearance;
        let gradient = AngularGradient::new([
            UIColor::system_red().resolved_color(appearance),
            UIColor::system_orange().resolved_color(appearance),
            UIColor::system_yellow().resolved_color(appearance),
            UIColor::system_green().resolved_color(appearance),
            UIColor::system_cyan().resolved_color(appearance),
            UIColor::system_blue().resolved_color(appearance),
            UIColor::system_purple().resolved_color(appearance),
            UIColor::system_red().resolved_color(appearance),
        ]);
        div()
            .relative()
            .size(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .child(angular_gradient_circle_stroke(gradient, 15., 15.))
            .child(div().absolute().left(px(10.5)).top(px(10.5)).size(px(9.)).circle().bg(white()))
            .child(centred_stroke(white().opacity(0.55), 0.5, Shape::Circle))
    }
}

impl RenderOnce for EditorBrushPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let selected_tool = self.tool.get(cx);
        let content = vstack(10.)
            .w_full()
            .child(self.brushes_row(selected_tool))
            .child(self.colors_row(cx))
            .child(self.footer())
            .px(px(18.))
            .pt(px(12.))
            .pb(px(22.));
        editor_panel_escape("editor-brush-panel", "editor-brush-panel-appear", self.on_close, content, window, cx)
    }
}

#[derive(IntoElement)]
struct EditorDrawSegmentControl {
    on_stickers_selected: ButtonAction,
    on_text_selected: ButtonAction,
    on_link_selected: Option<ButtonAction>,
}

impl EditorDrawSegmentControl {
    const ICONS: [&'static str; 3] = ["pencil", "face.smiling", "textformat"];

    fn segment(
        id: impl Into<ElementId>,
        icon: &str,
        label: SharedString,
        is_selected: bool,
        action: ButtonAction,
    ) -> PlainButton {
        plain(id).on_press(move |window, cx| action(window, cx)).child(
            div()
                .id(SharedString::from(format!("editor-draw-segment-{icon}")))
                .aria_label(label.clone())
                .tooltip_label(label)
                .flex()
                .items_center()
                .justify_center()
                .w(px(44.))
                .h(px(46.))
                .capsule()
                .when(is_selected, |segment| segment.bg(white().opacity(0.2)))
                .children(IconUi::sized(
                    icon,
                    px(17.),
                    if is_selected { white() } else { white().opacity(0.68) },
                    false,
                )),
        )
    }
}

impl RenderOnce for EditorDrawSegmentControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("editor-draw-segment", cx, |_, _| 0_usize);
        let selected = *state.read(cx);
        let labels = [tr("editor_segment_drawing"), tr("editor_segment_stickers"), tr("editor_segment_text")];
        let segments = labels.into_iter().enumerate().map(|(index, label)| {
            let state = state.clone();
            let on_stickers_selected = self.on_stickers_selected.clone();
            let on_text_selected = self.on_text_selected.clone();
            let action: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
                if index == 1 {
                    on_stickers_selected(window, cx);
                    return;
                }
                if index == 2 {
                    on_text_selected(window, cx);
                    return;
                }
                state.update(cx, |selected, cx| {
                    *selected = index;
                    cx.notify();
                });
            });
            Self::segment(("editor-draw-segment", index), Self::ICONS[index], label.into(), selected == index, action)
        });
        let row = hstack(0.)
            .children(segments)
            .when_some(self.on_link_selected, |row, on_link_selected| {
                row.child(Self::segment("editor-draw-segment-link", "link", tr("link").into(), false, on_link_selected))
            })
            .p(px(3.))
            .liquid_chrome_capsule_with(0.08, true, None, false);
        div().flex_none().child(row)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorBrushDef {
    pub tool: EditorDrawStrokeTool,
    pub icon: &'static str,
}

impl EditorBrushDef {
    pub const ALL: [Self; 7] = [
        Self { tool: EditorDrawStrokeTool::Pen, icon: "pencil.tip" },
        Self { tool: EditorDrawStrokeTool::Marker, icon: "highlighter" },
        Self { tool: EditorDrawStrokeTool::Neon, icon: "wand.and.stars" },
        Self { tool: EditorDrawStrokeTool::Pencil, icon: "pencil" },
        Self { tool: EditorDrawStrokeTool::Arrow, icon: "arrow.up.right" },
        Self { tool: EditorDrawStrokeTool::Blur, icon: "drop" },
        Self { tool: EditorDrawStrokeTool::Eraser, icon: "eraser.fill" },
    ];

    pub fn without_blur() -> Vec<Self> {
        Self::ALL.into_iter().filter(|definition| definition.tool != EditorDrawStrokeTool::Blur).collect()
    }
}

struct BrushIconMotion {
    offset: Animated<f32>,
    tint: Animated<Hsla>,
}

#[derive(IntoElement)]
struct BrushIcon {
    index: usize,
    definition: EditorBrushDef,
    tint: Hsla,
    is_selected: bool,
}

impl BrushIcon {
    const SELECTED_OFFSET: f32 = -4.;

    fn offset(is_selected: bool) -> f32 {
        if is_selected { Self::SELECTED_OFFSET } else { 0. }
    }
}

impl RenderOnce for BrushIcon {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (offset_target, tint_target) = (Self::offset(self.is_selected), self.tint);
        let motion = window.use_keyed_state(("editor-brush-icon", self.index), cx, |_, _| BrushIconMotion {
            offset: Animated::new(offset_target),
            tint: Animated::new(tint_target),
        });
        {
            let motion = motion.clone();
            on_change(
                ("editor-brush-icon-change", self.index),
                (self.is_selected, self.tint),
                false,
                window,
                cx,
                move |old, new, _, cx| {
                    motion.update(cx, |motion, cx| {
                        if old.0 == new.0 {
                            motion.tint.jump(new.1);
                        } else {
                            let now = RiseClock::monotonic();
                            motion.offset.set_in(Self::offset(new.0), spring(0.32, 0.82), now, cx);
                            motion.tint.set_in(new.1, spring(0.32, 0.82), now, cx);
                        }
                        cx.notify();
                    });
                },
            );
        }
        let now = RiseClock::monotonic();
        let (offset, tint, is_running) = {
            let motion = motion.read(cx);
            (motion.offset.value(now), motion.tint.value(now), motion.offset.is_running(now) || motion.tint.is_running(now))
        };
        if is_running {
            window.request_animation_frame();
        }
        div()
            .relative()
            .top(px(offset))
            .children(IconUi::sized(self.definition.icon, px(23.), tint, false))
    }
}

pub trait EditorBrushColorComparison {
    fn is_close_to(&self, other: Hsla) -> bool;
}

impl EditorBrushColorComparison for Hsla {
    fn is_close_to(&self, other: Hsla) -> bool {
        let (first, second) = (self.to_rgb(), other.to_rgb());
        (first.r - second.r).abs() < 0.01 && (first.g - second.g).abs() < 0.01 && (first.b - second.b).abs() < 0.01
    }
}
