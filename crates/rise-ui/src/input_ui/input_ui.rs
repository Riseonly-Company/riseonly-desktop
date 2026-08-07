use gpui::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler,
    Entity, Focusable, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, ShapedLine, Style, TextAlign, UnderlineStyle, Window, div, fill, point,
    prelude::*, px, relative, size,
};

use crate::input_ui::input_actions::KEY_CONTEXT;
use crate::input_ui::input_ui_state::InputUiState;
use crate::input_ui::shaped_input::{ShapeKey, ShapedInput};

/// The text surface of an input: caret, selection, marked text and nothing else.
///
/// The chrome around it — background, border, radius, key context, actions — is
/// [`InputUiState`]'s own `Render`, so a caller writes `div().child(state)` and
/// never has to assemble the two halves correctly.
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
    fn read(cx: &App) -> Self {
        let theme = crate::theme(cx);
        Self {
            text: theme.text.primary,
            placeholder: theme.text.secondary,
            caret: theme.input.caret,
            selection: theme.input.selection,
            marked_underline: theme.text.primary,
            caret_width: theme.input.caret_width,
            underline_thickness: theme.input.marked_underline,
        }
    }
}

pub struct InputPrepaint {
    hitbox: Hitbox,
    origin: Point<Pixels>,
    line_height: Pixels,
    lines: Vec<(usize, ShapedLine)>,
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    disabled: bool,
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
        let rows = if state.mode().is_multiline() {
            state
                .display()
                .line_count()
                .clamp(1, state.max_visible_lines())
        } else {
            1
        };

        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (line_height * (rows as f32)).into();
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
        let palette = InputPalette::read(cx);
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        self.state.update(cx, |state, cx| {
            state.sync_focus(focused, cx);
            state.set_viewport(bounds, line_height);

            let line_count = state.display().line_count();
            let caret_line = state.caret_line();
            let rows_visible = rows_that_fit(bounds.size.height, line_height);

            let mut scroll = state.scroll();
            scroll.y = vertical_scroll_for(
                scroll.y,
                caret_line,
                rows_visible,
                line_count,
                bounds.size.height,
                line_height,
            );

            let first_line = row_at(scroll.y, line_height);
            let visible = first_line..(first_line + rows_visible + 1).min(line_count);

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
                visible: visible.clone(),
            };

            if state.shaped().is_none_or(|shaped| !shaped.matches(&key)) {
                let underline = UnderlineStyle {
                    thickness: palette.underline_thickness,
                    color: Some(palette.marked_underline),
                    wavy: false,
                };
                let shaped = ShapedInput::shape(key, state.display(), underline, window);
                state.set_shaped(shaped);
            }

            let selection = state.selection();
            let display = state.display();
            let shaped = state.shaped().expect("shaped above");

            let caret_x = shaped.x_for(
                caret_line,
                display.offset_within_line(caret_line, selection.head),
            );
            scroll.x =
                horizontal_scroll_for(scroll.x, caret_x, bounds.size.width, palette.caret_width);

            let origin = point(bounds.left() - scroll.x, bounds.top() - scroll.y);
            let selected = selection.range();
            let mut selection_quads = Vec::new();
            let mut lines = Vec::with_capacity(visible.len());

            for line_index in visible.clone() {
                let Some(line) = display.lines.get(line_index) else {
                    break;
                };
                let Some(shaped_line) = shaped.line(line_index) else {
                    break;
                };
                lines.push((line_index, shaped_line.clone()));

                if display.is_placeholder || selected.is_empty() {
                    continue;
                }
                let start = selected.start.max(line.source.start);
                let end = selected.end.min(line.source.end);
                if start > line.source.end || end < line.source.start {
                    continue;
                }

                let left = shaped.x_for(line_index, display.offset_within_line(line_index, start));
                let mut right =
                    shaped.x_for(line_index, display.offset_within_line(line_index, end));
                if selected.end > line.source.end {
                    right = right.max(shaped.width(line_index));
                }
                if right <= left {
                    continue;
                }

                let top = origin.y + line_height * (line_index as f32);
                selection_quads.push(fill(
                    Bounds::from_corners(
                        point(origin.x + left, top),
                        point(origin.x + right, top + line_height),
                    ),
                    palette.selection,
                ));
            }

            let caret = (state.caret_is_visible() && selection.is_empty()).then(|| {
                let top = origin.y + line_height * (caret_line as f32);
                fill(
                    Bounds::new(
                        point(origin.x + caret_x, top),
                        size(palette.caret_width, line_height),
                    ),
                    palette.caret,
                )
            });

            state.set_scroll(scroll);

            InputPrepaint {
                hitbox: hitbox.clone(),
                origin,
                line_height,
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
        let selection = std::mem::take(&mut prepaint.selection);
        let lines = std::mem::take(&mut prepaint.lines);
        let caret = prepaint.caret.take();

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in selection {
                window.paint_quad(quad);
            }
            for (line_index, line) in lines {
                let position = point(origin.x, origin.y + line_height * (line_index as f32));
                line.paint(position, line_height, TextAlign::Left, None, window, cx)
                    .ok();
            }
            if let Some(caret) = caret {
                window.paint_quad(caret);
            }
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
            window.focus(&focus_handle, cx);
            state.update(cx, |state, cx| {
                state.on_mouse_down(event.position, event.click_count, event.modifiers.shift, cx);
            });
        });

        // Registered on the window, not on a hitbox: a drag that leaves the
        // element must keep selecting, and a hitbox-filtered listener stops at
        // the border.
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

fn horizontal_scroll_for(
    current: Pixels,
    caret_x: Pixels,
    width: Pixels,
    caret_width: Pixels,
) -> Pixels {
    let mut scroll = current;
    if caret_x < scroll {
        scroll = caret_x;
    } else if caret_x + caret_width > scroll + width {
        scroll = caret_x + caret_width - width;
    }
    scroll.max(px(0.))
}

impl Render for InputUiState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme(cx);
        let typography = theme.typography.body();
        let multiline = self.mode().is_multiline();
        let disabled = self.is_disabled();
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
        let text_color = if disabled {
            theme.text.secondary
        } else {
            theme.text.primary
        };
        let padding = theme.input.padding_x;
        let radius = theme.input.radius_300;
        let height = self.height().unwrap_or(theme.input.height_300);

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
            .flex()
            .w_full()
            .when(!multiline, |chrome| chrome.h(height).items_center())
            .when(multiline, |chrome| chrome.min_h(height))
            .px(padding)
            .when(multiline, |chrome| chrome.py(padding))
            .rounded(radius)
            .bg(background)
            .border_1()
            .border_color(border)
            .text_size(typography.size)
            .line_height(typography.line_height)
            .font(typography.font.clone())
            .text_color(text_color)
            .child(InputUi::new(cx.entity()))
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
            horizontal_scroll_for(px(0.), px(300.), px(100.), px(2.)),
            px(202.)
        );
        assert_eq!(
            horizontal_scroll_for(px(202.), px(10.), px(100.), px(2.)),
            px(10.)
        );
        assert_eq!(
            horizontal_scroll_for(px(0.), px(50.), px(100.), px(2.)),
            px(0.)
        );
    }
}
