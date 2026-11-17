use gpui::{
    Anchor, AnyElement, App, Bounds, ClickEvent, ContentMask, Corners, CursorStyle, DispatchPhase,
    Element, ElementId, ElementInputHandler, Entity, Focusable, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, ScrollWheelEvent, PaintQuad, Pixels, Point, ShapedLine, SharedString,
    Style, TextAlign,
    UnderlineStyle, Window, anchored, deferred, div, fill, point, prelude::*, px, relative, size,
    transparent_black,
};
use rise_i18n::i18n::tr;

use crate::IconUi;
use crate::input_ui::caret_blink::{CaretSwap, caret_path, caret_swap_key};
use crate::input_ui::input_actions::KEY_CONTEXT;
use crate::input_ui::input_ui_state::{InputEditMenuCommand, InputEditMenuPresentation, InputUiState};
use crate::input_ui::shaped_input::{PlacedAttachment, ShapeKey, ShapedInput};
use crate::swiftui::button_style::keyboard_focus_ring;
use crate::swiftui::font::{Font, SwiftUiFont};
use crate::swiftui::shapes::SwiftUiShapes;
use crate::swiftui::stacks::{hstack, spacer, vstack};

const EDIT_MENU_CORNER_RADIUS: f32 = 13.0;
const EDIT_MENU_MIN_WIDTH: f32 = 250.0;
const EDIT_MENU_ROW_HORIZONTAL_PADDING: f32 = 16.0;
const EDIT_MENU_ROW_VERTICAL_PADDING: f32 = 11.0;
const EDIT_MENU_ROW_FONT_SIZE: f32 = 17.0;
const EDIT_MENU_ICON_SIDE: f32 = 17.0;
const EDIT_MENU_SUBMENU_SYMBOL: &str = "chevron.right";
const EDIT_MENU_ANCHOR_GAP: f32 = 8.0;
const EDIT_MENU_WINDOW_MARGIN: f32 = 8.0;

/// text surface of an input, caret selection marked text and nothing else
pub struct InputUi {
    state: Entity<InputUiState>,
}

impl InputUi {
    pub fn new(state: Entity<InputUiState>) -> Self {
        Self { state }
    }
}

#[derive(Clone, Copy)]
struct InputPalette {
    text: Hsla,
    placeholder: Hsla,
    caret: Hsla,
    selection: Hsla,
    marked_underline: Hsla,
    caret_width: Pixels,
    underline_thickness: Pixels,
}

impl InputPalette {
    /// theme colors with the field own overrides on top
    fn read(state: &InputUiState, cx: &App) -> Self {
        let theme = crate::theme(cx);
        let text = state.text_color().unwrap_or(theme.text.primary);
        Self {
            text,
            placeholder: state.placeholder_color().unwrap_or(theme.text.secondary),
            caret: state.caret_color().unwrap_or(theme.input.caret),
            selection: theme.input.selection,
            marked_underline: text,
            caret_width: theme.input.caret_width,
            underline_thickness: theme.input.marked_underline,
        }
    }
}

pub struct InputPrepaint {
    hitbox: Hitbox,
    origin: Point<Pixels>,
    line_height: Pixels,
    baseline_lift: Pixels,
    lines: Vec<(usize, Pixels, ShapedLine, Vec<PlacedAttachment>)>,
    selection: Vec<PaintQuad>,
    caret: Option<CaretPaint>,
    disabled: bool,
}

/// caret of a focused field, drawn dark on the blink off half so a tick can swap its color
#[derive(Clone, Copy)]
struct CaretPaint {
    bounds: Bounds<Pixels>,
    lit: bool,
    color: Hsla,
}

impl IntoElement for InputUi {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for InputUi {
    type RequestLayoutState = ();
    type PrepaintState = InputPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let line_height = window.line_height();
        let state = self.state.read(cx);
        let rows = state.layout_rows();

        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if state.fills_parent_height() {
            style.min_size.height = line_height.into();
            style.flex_grow = 1.;
        } else {
            style.size.height = (line_height * (rows as f32)).into();
        }
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let line_height = window.line_height();
        let text_style = window.text_style();
        let font = text_style.font();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let palette = InputPalette::read(self.state.read(cx), cx);
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        self.state.update(cx, |state, cx| {
            state.sync_focus(focused, cx);
            state.set_viewport(bounds, line_height);
            state.set_caret_reserve(palette.caret_width);
            let line_count = state.display().line_count();
            let caret_line = state.caret_line();
            let rows_visible = rows_that_fit(bounds.size.height, line_height);
            let wrap_width = state
                .wraps()
                .then(|| (bounds.size.width - palette.caret_width).max(px(1.)));

            let mut scroll = state.scroll();
            let follows_caret = focused || state.fills_parent_height();
            let wheel_held = state.wheel_scroll_holds();
            let visible = if wrap_width.is_some() {
                0..line_count
            } else {
                scroll.y = if wheel_held {
                    held_vertical_scroll(scroll.y, line_count, bounds.size.height, line_height)
                } else {
                    vertical_scroll_for(
                        scroll.y,
                        if follows_caret { caret_line } else { 0 },
                        rows_visible,
                        line_count,
                        bounds.size.height,
                        line_height,
                    )
                };
                let first_line = row_at(scroll.y, line_height);
                first_line..(first_line + rows_visible + 1).min(line_count)
            };

            let color = if state.display().is_placeholder {
                palette.placeholder
            } else {
                palette.text
            };
            let key = ShapeKey {
                revision: state.revision(),
                font_size,
                font: font.clone(),
                color,
                marked: state.marked_range(),
                visible,
                wrap_width,
                placeholder_width: (wrap_width.is_none() && state.display().is_placeholder)
                    .then(|| (bounds.size.width - palette.caret_width).max(px(1.))),
            };

            if state.shaped().is_none_or(|shaped| !shaped.matches(&key)) {
                let rows_before = state.visual_row_count();
                let underline = UnderlineStyle {
                    thickness: palette.underline_thickness,
                    color: Some(palette.marked_underline),
                    wavy: false,
                };
                let shaped = ShapedInput::shape(
                    key,
                    state.display(),
                    state.attributed_runs(),
                    underline,
                    window,
                );
                state.set_shaped(shaped);
                if state.visual_row_count() != rows_before {
                    cx.notify();
                }
            }
            state.publish_fitted_height(cx);

            let selection = state.selection();
            let display = state.display();
            let shaped = state.shaped().expect("shaped above");

            let caret_offset = display.offset_within_line(caret_line, selection.head);
            let caret_row = shaped.row_for(caret_line, caret_offset).unwrap_or(caret_line);
            let line_width = bounds.size.width - palette.caret_width;
            let caret_x = shaped.x_for(caret_line, caret_offset)
                + state.row_alignment_shift(caret_row, line_width);
            let mut painted_rows = shaped.row_range();
            if shaped.wraps() {
                scroll.y = if wheel_held {
                    held_vertical_scroll(scroll.y, painted_rows.end, bounds.size.height, line_height)
                } else {
                    vertical_scroll_for(
                        scroll.y,
                        if follows_caret { caret_row } else { 0 },
                        rows_visible,
                        painted_rows.end,
                        bounds.size.height,
                        line_height,
                    )
                };
                let first_row = row_at(scroll.y, line_height);
                painted_rows = first_row..(first_row + rows_visible + 1).min(painted_rows.end);
                scroll.x = px(0.);
            } else {
                let content_width = shaped
                    .row(caret_row)
                    .map_or(caret_x, |row| row.shaped.width() + state.row_alignment_shift(caret_row, line_width))
                    .max(caret_x);
                scroll.x = horizontal_scroll_for(
                    scroll.x,
                    if follows_caret { caret_x } else { px(0.) },
                    content_width,
                    bounds.size.width,
                    palette.caret_width,
                );
            }

            let origin = point(bounds.left() - scroll.x, bounds.top() - scroll.y);
            let selected = selection.range();
            let mut selection_quads = Vec::new();
            let mut lines = Vec::with_capacity(painted_rows.len());

            for row_index in painted_rows {
                let Some(row) = shaped.row(row_index) else {
                    break;
                };
                lines.push((
                    row_index,
                    state.row_alignment_shift(row_index, line_width),
                    row.shaped.clone(),
                    row.attachments.clone(),
                ));

                if display.is_placeholder || selected.is_empty() {
                    continue;
                }
                let row_source = display.source_offset_within_line(row.line, row.start)
                    ..display.source_offset_within_line(row.line, row.end);
                let start = selected.start.max(row_source.start);
                let end = selected.end.min(row_source.end);
                if start > row_source.end || end < row_source.start {
                    continue;
                }

                let left = row
                    .shaped
                    .x_for_index(display.offset_within_line(row.line, start).saturating_sub(row.start));
                let mut right = row
                    .shaped
                    .x_for_index(display.offset_within_line(row.line, end).saturating_sub(row.start));
                if selected.end > row_source.end {
                    right = right.max(row.shaped.width());
                }
                if right <= left {
                    continue;
                }

                let top = origin.y + line_height * (row_index as f32);
                let shift = state.row_alignment_shift(row_index, line_width);
                selection_quads.push(fill(
                    Bounds::from_corners(
                        point(origin.x + shift + left, top),
                        point(origin.x + shift + right, top + line_height),
                    ),
                    palette.selection,
                ));
            }

            let caret = (state.caret_is_shown() && selection.is_empty()).then(|| {
                let top = origin.y + line_height * (caret_row as f32);
                CaretPaint {
                    bounds: Bounds::new(
                        point(origin.x + caret_x, top),
                        size(palette.caret_width, line_height),
                    ),
                    lit: state.caret_is_visible(),
                    color: palette.caret,
                }
            });

            state.set_scroll(scroll);

            InputPrepaint {
                hitbox: hitbox.clone(),
                origin,
                line_height,
                baseline_lift: state.baseline_lift().min(line_height),
                lines,
                selection: selection_quads,
                caret,
                disabled: state.is_disabled(),
            }
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.state.read(cx).focus_handle(cx);
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.state.clone()),
            cx,
        );
        window.set_cursor_style(
            if prepaint.disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::IBeam
            },
            &prepaint.hitbox,
        );

        let origin = prepaint.origin;
        let line_height = prepaint.line_height;
        let baseline_lift = prepaint.baseline_lift;
        let glyph_line_height = line_height - baseline_lift;
        let selection = std::mem::take(&mut prepaint.selection);
        let lines = std::mem::take(&mut prepaint.lines);
        let caret = prepaint.caret.take();

        let field = self.state.entity_id();
        let caret_swap = window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in selection {
                window.paint_quad(quad);
            }
            for (row_index, shift, line, attachments) in lines {
                let position = point(
                    origin.x + shift,
                    origin.y + line_height * (row_index as f32) + baseline_lift,
                );
                line.paint(position, glyph_line_height, TextAlign::Left, None, window, cx)
                    .ok();
                paint_attachments(&line, &attachments, position, glyph_line_height, window);
            }
            caret.map(|caret| {
                let key = caret_swap_key(field);
                let color = if caret.lit { caret.color } else { transparent_black() };
                window.paint_swappable_path(caret_path(caret.bounds), color, key, caret.bounds.origin);
                CaretSwap {
                    key,
                    size: caret.bounds.size,
                    color: caret.color,
                    window: window.window_handle(),
                }
            })
        });
        self.state.update(cx, |state, _| state.note_caret_painted(caret_swap));

        let hitbox = prepaint.hitbox.clone();
        let state = self.state.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            let delta = event.delta.pixel_delta(line_height);
            if state.update(cx, |state, cx| state.scroll_by_wheel(delta.y, cx)) {
                cx.stop_propagation();
            }
        });

        let hitbox = prepaint.hitbox.clone();
        let state = self.state.clone();
        let menu_focus = focus_handle.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Right
                || !hitbox.is_hovered(window)
            {
                return;
            }
            cx.stop_propagation();
            window.focus(&menu_focus, cx);
            state.update(cx, |state, cx| state.open_edit_menu(event.position, window, cx));
        });

        if prepaint.disabled {
            return;
        }

        let hitbox = prepaint.hitbox.clone();
        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !hitbox.is_hovered(window)
            {
                return;
            }
            cx.stop_propagation();
            window.focus(&focus_handle, cx);
            crate::keyboard_dismiss::ScreenFocus::keep_text_field_on_press(hitbox.bounds, window, cx);
            state.update(cx, |state, cx| {
                state.on_mouse_down(event.position, event.click_count, event.modifiers.shift, cx);
            });
        });

        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            state.update(cx, |state, cx| {
                if state.is_dragging() {
                    state.on_mouse_drag(event.position, cx);
                }
            });
        });

        let state = self.state.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            state.update(cx, |state, cx| state.on_mouse_up(cx));
        });
    }
}

fn paint_attachments(
    line: &ShapedLine,
    attachments: &[PlacedAttachment],
    row_origin: Point<Pixels>,
    line_height: Pixels,
    window: &mut Window,
) {
    if attachments.is_empty() {
        return;
    }
    let baseline = row_origin.y + (line_height - line.ascent - line.descent) / 2.0 + line.ascent;
    for placed in attachments {
        let Some(image) = placed.attachment.image.clone() else {
            continue;
        };
        let bounds = placed.attachment.bounds;
        let slot = attachment_slot(
            row_origin.x + line.x_for_index(placed.offset),
            baseline,
            bounds,
        );
        let image_size = image.size(0);
        if slot.size.width <= px(0.)
            || slot.size.height <= px(0.)
            || image_size.width.0 <= 0
            || image_size.height.0 <= 0
        {
            continue;
        }
        let fitted = ObjectFit::Contain.get_bounds(slot, image_size);
        window
            .paint_image(slot, fitted, Corners::default(), image, 0, false)
            .ok();
    }
}

/// where an attachment draws on screen, bounds are read in y up text space at pen x on the baseline
pub(crate) fn attachment_slot(
    x: Pixels,
    baseline: Pixels,
    bounds: Bounds<Pixels>,
) -> Bounds<Pixels> {
    Bounds::new(
        point(
            x + bounds.origin.x,
            baseline - bounds.origin.y - bounds.size.height,
        ),
        bounds.size,
    )
}

fn rows_that_fit(height: Pixels, line_height: Pixels) -> usize {
    if line_height <= px(0.) {
        return 1;
    }
    ((f32::from(height) / f32::from(line_height)).floor() as usize).max(1)
}

fn row_at(offset: Pixels, line_height: Pixels) -> usize {
    if line_height <= px(0.) {
        return 0;
    }
    (f32::from(offset) / f32::from(line_height))
        .floor()
        .max(0.0) as usize
}

fn vertical_scroll_for(
    current: Pixels,
    caret_line: usize,
    rows_visible: usize,
    line_count: usize,
    height: Pixels,
    line_height: Pixels,
) -> Pixels {
    let mut scroll = current;
    let first_visible = row_at(scroll, line_height);
    if caret_line < first_visible {
        scroll = line_height * (caret_line as f32);
    } else if caret_line + 1 > first_visible + rows_visible {
        scroll = line_height * ((caret_line + 1 - rows_visible) as f32);
    }
    let overflow = (line_height * (line_count as f32) - height).max(px(0.));
    scroll.clamp(px(0.), overflow)
}

/// offset a wheel left, kept inside the text when it shrinks under it
fn held_vertical_scroll(current: Pixels, line_count: usize, height: Pixels, line_height: Pixels) -> Pixels {
    let overflow = (line_height * (line_count as f32) - height).max(px(0.));
    current.clamp(px(0.), overflow)
}

fn horizontal_scroll_for(
    current: Pixels,
    caret_x: Pixels,
    content_width: Pixels,
    width: Pixels,
    caret_width: Pixels,
) -> Pixels {
    let mut scroll = current;
    if caret_x < scroll {
        scroll = caret_x;
    } else if caret_x + caret_width > scroll + width {
        scroll = caret_x + caret_width - width;
    }
    let overflow = content_width + caret_width - width;
    scroll.min(overflow).max(px(0.))
}

impl InputUiState {
    fn render_edit_menu(
        &self,
        presentation: InputEditMenuPresentation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = crate::theme(cx);
        let menu_bg = theme.bg._200;
        let label_color = theme.text.primary;
        let separator = theme.border._200;

        let row = |id: SharedString, label: String, icon: Option<String>| {
            hstack(None)
                .id(id)
                .w_full()
                .px(px(EDIT_MENU_ROW_HORIZONTAL_PADDING))
                .py(px(EDIT_MENU_ROW_VERTICAL_PADDING))
                .cursor_pointer()
                .hover(move |style| style.bg(separator.opacity(0.35)))
                .swiftui_font(Font::system_size(EDIT_MENU_ROW_FONT_SIZE))
                .text_color(label_color)
                .child(label)
                .child(spacer())
                .children(icon.and_then(|symbol| {
                    IconUi::sized(&symbol, px(EDIT_MENU_ICON_SIDE), label_color, false)
                }))
        };

        let mut rows: Vec<AnyElement> = Vec::new();
        let submenu = self.edit_submenu().filter(|_| presentation.in_submenu);
        if let Some(submenu) = submenu {
            for item in &submenu.items {
                let callback = item.callback.clone();
                rows.push(
                    row(
                        SharedString::from(format!("edit-menu-{}-{}", item.id, item.key)),
                        item.label.clone(),
                        item.icon.clone(),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.close_edit_menu(window, cx);
                        let callback = callback.clone();
                        window.defer(cx, move |window, cx| callback(window, cx));
                    }))
                    .into_any_element(),
                );
            }
        } else {
            for command in self.edit_menu_commands() {
                let (label, icon) = match command.label_key() {
                    Some(key) => (tr(key), None),
                    None => (
                        self.edit_submenu()
                            .map(|submenu| submenu.title.clone())
                            .unwrap_or_default(),
                        Some(EDIT_MENU_SUBMENU_SYMBOL.to_owned()),
                    ),
                };
                rows.push(
                    row(SharedString::from(format!("edit-menu-{command:?}")), label, icon)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.perform_edit_menu_command(command, window, cx);
                        }))
                        .into_any_element(),
                );
            }
        }

        let rows = rows.into_iter().enumerate().map(|(index, row)| {
            if index == 0 {
                row
            } else {
                vstack(0.0)
                    .w_full()
                    .child(div().w_full().h(px(0.5)).bg(separator))
                    .child(row)
                    .into_any_element()
            }
        });

        deferred(
            anchored()
                .anchor(Anchor::BottomCenter)
                .position(presentation.anchor)
                .offset(point(px(0.), px(-EDIT_MENU_ANCHOR_GAP)))
                .snap_to_window_with_margin(px(EDIT_MENU_WINDOW_MARGIN))
                .child(
                    vstack(0.0)
                        .id("input-edit-menu")
                        .occlude()
                        .min_w(px(EDIT_MENU_MIN_WIDTH))
                        .bg(menu_bg)
                        .continuous_corners(EDIT_MENU_CORNER_RADIUS)
                        .overflow_hidden()
                        .shadow_lg()
                        .children(rows)
                        .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.close_edit_menu(window, cx);
                        })),
                ),
        )
        .with_priority(1)
        .into_any_element()
    }
}

impl Render for InputUiState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let edit_menu = self.edit_menu().map(|menu| self.render_edit_menu(menu, cx));
        let theme = crate::theme(cx);
        let typography = theme.typography.body();
        let font_size = self.font_size().unwrap_or(typography.size);
        let line_height = if typography.size > px(0.) {
            typography.line_height * (f32::from(font_size) / f32::from(typography.size))
        } else {
            typography.line_height
        };
        let line_height = self
            .tallest_attachment()
            .map_or(line_height, |tallest| line_height.max(tallest));
        let held_line_height = self.held_line_height(line_height);
        self.set_baseline_lift(held_line_height - line_height);
        let line_height = held_line_height;
        let fills_parent_height = self.fills_parent_height();
        let multiline = self.mode().is_multiline();
        let disabled = self.is_disabled();
        let chromeless = self.is_chromeless();
        let focused = self.focus_handle(cx).is_focused(window);

        let background = if disabled {
            theme.input.bg_100
        } else {
            theme.input.bg_200
        };
        let border = if focused {
            theme.primary._200
        } else {
            theme.input.border_300
        };
        let focus_ring_color = theme.primary._100;
        let text_color = if disabled {
            theme.text.secondary
        } else {
            self.text_color().unwrap_or(theme.text.primary)
        };
        let padding = theme.input.padding_x;
        let radius = theme.input.radius_300;
        let height = self.height().unwrap_or(theme.input.height_300);
        let focus_ring = if chromeless {
            None
        } else {
            keyboard_focus_ring(&self.focus_handle(cx), focus_ring_color, radius, window)
        };

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle(cx))
            .cursor(if disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::IBeam
            })
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::move_word_left))
            .on_action(cx.listener(Self::move_word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::move_line_start))
            .on_action(cx.listener(Self::move_line_end))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::move_document_start))
            .on_action(cx.listener(Self::move_document_end))
            .on_action(cx.listener(Self::select_document_start))
            .on_action(cx.listener(Self::select_document_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::backspace_word))
            .on_action(cx.listener(Self::backspace_line))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .flex()
            .w_full()
            .when(fills_parent_height, |field| field.h_full())
            .when(!multiline, |field| field.items_center())
            .when(!chromeless, |chrome| {
                chrome
                    .when(!multiline, |chrome| chrome.h(height))
                    .when(multiline, |chrome| chrome.min_h(height).py(padding))
                    .px(padding)
                    .rounded(radius)
                    .bg(background)
                    .border_1()
                    .border_color(border)
            })
            .text_size(font_size)
            .line_height(line_height)
            .font(typography.font.clone())
            .when_some(self.font_weight(), |field, weight| field.font_weight(weight))
            .text_color(text_color)
            .child(InputUi::new(cx.entity()))
            .children(focus_ring)
            .children(edit_menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_viewport_holds_at_least_one_row_even_before_layout() {
        assert_eq!(rows_that_fit(px(0.), px(0.)), 1);
        assert_eq!(rows_that_fit(px(10.), px(0.)), 1);
        assert_eq!(rows_that_fit(px(100.), px(20.)), 5);
        assert_eq!(rows_that_fit(px(10.), px(20.)), 1);
    }

    #[test]
    fn scrolling_follows_the_caret_in_both_directions() {
        let line_height = px(20.);
        let height = px(60.);

        let scrolled_down = vertical_scroll_for(px(0.), 5, 3, 10, height, line_height);
        assert_eq!(scrolled_down, px(60.));

        let scrolled_up = vertical_scroll_for(px(60.), 1, 3, 10, height, line_height);
        assert_eq!(scrolled_up, px(20.));

        let unchanged = vertical_scroll_for(px(60.), 4, 3, 10, height, line_height);
        assert_eq!(unchanged, px(60.));
    }

    #[test]
    fn scrolling_never_runs_past_the_last_line() {
        let scroll = vertical_scroll_for(px(400.), 9, 3, 10, px(60.), px(20.));
        assert_eq!(scroll, px(140.));
    }

    #[test]
    fn a_document_shorter_than_the_viewport_never_scrolls() {
        let scroll = vertical_scroll_for(px(0.), 0, 5, 2, px(100.), px(20.));
        assert_eq!(scroll, px(0.));
    }

    #[test]
    fn horizontal_scroll_keeps_the_whole_caret_on_screen() {
        assert_eq!(
            horizontal_scroll_for(px(0.), px(300.), px(300.), px(100.), px(2.)),
            px(202.)
        );
        assert_eq!(
            horizontal_scroll_for(px(202.), px(10.), px(300.), px(100.), px(2.)),
            px(10.)
        );
        assert_eq!(
            horizontal_scroll_for(px(0.), px(50.), px(50.), px(100.), px(2.)),
            px(0.)
        );
    }

    #[test]
    fn horizontal_scroll_pulls_back_when_the_field_grows() {
        assert_eq!(
            horizontal_scroll_for(px(38.), px(40.), px(40.), px(300.), px(2.)),
            px(0.)
        );
        assert_eq!(
            horizontal_scroll_for(px(202.), px(300.), px(300.), px(250.), px(2.)),
            px(52.)
        );
    }
}
