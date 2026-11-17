use std::rc::Rc;

use gpui::{
    App, Bounds, Canvas, Context, Entity, FocusHandle, Hsla, KeyDownEvent, Pixels, Point, Render, Rgba, Subscription, Window,
    canvas, fill, hsla, point, size, white,
};
use rise_canon::rise_theme_tokens::HEX_RGB_DIGITS;
use rise_ui::IconUi;
use rise_ui::input_ui::{InputUiEvent, InputUiState};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::slider::slider;
use rise_ui::theme;
use rise_widgets::overlay::overlay_layer::OverlayLayer;
use rise_widgets::overlay::overlay_placement::{OverlayAnchor, OverlaySide, place};

pub type EditorColorPickerCallback = Rc<dyn Fn(Hsla, &mut Window, &mut App)>;

/// the open color sheet and its selected color
#[derive(Clone)]
pub struct EditorColorPickerInFlight {
    pub selected_color: Hsla,
    pub supports_alpha: bool,
    anchor: Point<Pixels>,
    spectrum_position: Point<f32>,
    hex_field: Entity<InputUiState>,
    focus_handle: FocusHandle,
    focus_before: Option<FocusHandle>,
}

pub struct EditorColorPickerBridge {
    live_callback: Option<EditorColorPickerCallback>,
    done_callback: Option<EditorColorPickerCallback>,
    picker_in_flight: Option<EditorColorPickerInFlight>,
    hex_field_events: Option<Subscription>,
}

impl EditorColorPickerBridge {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {
            live_callback: None,
            done_callback: None,
            picker_in_flight: None,
            hex_field_events: None,
        })
    }

    pub fn picker_in_flight(&self) -> Option<&EditorColorPickerInFlight> {
        self.picker_in_flight.as_ref()
    }

    /// opens the sheet at the given point
    /// on the desktop one window hosts everything
    #[allow(clippy::too_many_arguments)]
    pub fn present(
        &mut self,
        initial: Hsla,
        supports_alpha: bool,
        on: Point<Pixels>,
        live: impl Fn(Hsla, &mut Window, &mut App) + 'static,
        done: impl Fn(Hsla, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.live_callback = Some(Rc::new(live));
        self.done_callback = Some(Rc::new(done));
        let focus_handle = cx.focus_handle();
        let focus_before = window.focused(cx);
        window.focus(&focus_handle, cx);
        let hex_field = cx.new(InputUiState::single_line);
        hex_field.update(cx, |field, cx| {
            field.set_escape_to_layer(false);
            field.set_height(Some(px(EditorColorPickerHost::CLOSE_SIZE)), cx);
            field.set_font_size(Some(px(EditorColorPickerHost::HEX_FONT_SIZE)), cx);
            field.reset(EditorColorHex::text_of(initial), cx);
        });
        self.hex_field_events = Some(window.subscribe(&hex_field, cx, Self::on_hex_field_event));
        self.picker_in_flight = Some(EditorColorPickerInFlight {
            selected_color: initial,
            supports_alpha,
            anchor: on,
            spectrum_position: EditorColorSpectrum::position_of(initial),
            hex_field,
            focus_handle,
            focus_before,
        });
        cx.notify();
    }

    /// called when the sheet picks a color
    pub fn color_picker_view_controller_did_select_color(color: Hsla, window: &mut Window, cx: &mut App) {
        let live = Self::shared(cx).update(cx, |bridge, cx| {
            if let Some(picker) = &mut bridge.picker_in_flight {
                let digits = EditorColorHex::text_of(color);
                if EditorColorHex::text_of(picker.selected_color) != digits {
                    picker.hex_field.update(cx, |field, cx| field.reset(digits, cx));
                }
                picker.selected_color = color;
            }
            cx.notify();
            bridge.live_callback.clone()
        });
        if let Some(live) = live {
            live(color, window, cx);
        }
    }

    /// called when the sheet finishes
    pub fn color_picker_view_controller_did_finish(window: &mut Window, cx: &mut App) {
        let (picker, done) = Self::shared(cx).update(cx, |bridge, cx| {
            bridge.live_callback = None;
            bridge.hex_field_events = None;
            cx.notify();
            (bridge.picker_in_flight.take(), bridge.done_callback.take())
        });
        let Some(picker) = picker else {
            return;
        };
        if let Some(previous) = &picker.focus_before {
            window.focus(previous, cx);
        }
        if let Some(done) = done {
            done(picker.selected_color, window, cx);
        }
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.picker_in_flight = None;
        self.live_callback = None;
        self.done_callback = None;
        self.hex_field_events = None;
        cx.notify();
    }

    fn on_hex_field_event(field: Entity<InputUiState>, event: &InputUiEvent, window: &mut Window, cx: &mut App) {
        match event {
            InputUiEvent::Changed => {
                let state = field.read(cx);
                if state.marked_range().is_some() {
                    return;
                }
                let kept = EditorColorHex::kept_digits(state.text());
                if kept != state.text() {
                    field.update(cx, |field, cx| field.set_text(kept, cx));
                }
            }
            InputUiEvent::Submitted => Self::apply_hex_field(&field, window, cx),
            InputUiEvent::Cancelled => Self::color_picker_view_controller_did_finish(window, cx),
        }
    }

    fn apply_hex_field(field: &Entity<InputUiState>, window: &mut Window, cx: &mut App) {
        if let Some(parsed) = EditorColorHex::color_of(field.read(cx).text()) {
            let alpha = Self::shared(cx).update(cx, |bridge, _| {
                let picker = bridge.picker_in_flight.as_mut()?;
                picker.spectrum_position = EditorColorSpectrum::position_of(parsed);
                Some(picker.selected_color.a)
            });
            if let Some(alpha) = alpha {
                Self::color_picker_view_controller_did_select_color(parsed.alpha(alpha), window, cx);
            }
        }
        let current = Self::shared(cx).read(cx).picker_in_flight().map(|picker| picker.selected_color);
        if let Some(current) = current {
            let digits = EditorColorHex::text_of(current);
            field.update(cx, |field, cx| {
                if field.text() != digits {
                    field.reset(digits, cx);
                }
            });
        }
    }

    fn set_spectrum_position(position: Point<f32>, window: &mut Window, cx: &mut App) {
        let alpha = Self::shared(cx).update(cx, |bridge, _| {
            let picker = bridge.picker_in_flight.as_mut()?;
            picker.spectrum_position = position;
            Some(picker.selected_color.a)
        });
        if let Some(alpha) = alpha {
            let color = EditorColorSpectrum::color_at(position).alpha(alpha);
            Self::color_picker_view_controller_did_select_color(color, window, cx);
        }
    }
}

pub enum EditorColorPickerLauncher {}

impl EditorColorPickerLauncher {
    pub fn present(
        initial: Hsla,
        supports_alpha: bool,
        live: impl Fn(Hsla, &mut Window, &mut App) + 'static,
        done: impl Fn(Hsla, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut App,
    ) {
        let anchor = window.mouse_position();
        EditorColorPickerBridge::shared(cx).update(cx, |bridge, cx| {
            bridge.present(initial, supports_alpha, anchor, live, done, window, cx);
        });
    }
}

/// colors of the spectrum tab
/// hue runs left to right and tints fade in above the middle line and shades below it
pub enum EditorColorSpectrum {}

impl EditorColorSpectrum {
    /// position is a fraction of the surface on both axes
    pub fn color_at(position: Point<f32>) -> Hsla {
        let (hue, y) = (position.x.clamp(0., 1.), position.y.clamp(0., 1.));
        let (saturation, value) = if y < 0.5 { (y * 2., 1.) } else { (1., (1. - y) * 2.) };
        let lightness = value * (1. - saturation / 2.);
        let spread = lightness.min(1. - lightness);
        let hsl_saturation = if spread <= 0. { 0. } else { (value - lightness) / spread };
        hsla(hue, hsl_saturation, lightness, 1.)
    }

    /// painted surface of columns by rows cells filling its parent
    /// shared with the custom color sheet of the theme pages which embeds the same spectrum
    pub fn surface(columns: usize, rows: usize) -> Canvas<()> {
        canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                let cell_width = f32::from(bounds.size.width) / columns as f32;
                let cell_height = f32::from(bounds.size.height) / rows as f32;
                let painted = size(px(cell_width + 0.5), px(cell_height + 0.5));
                for column in 0..columns {
                    for row in 0..rows {
                        let center = point((column as f32 + 0.5) / columns as f32, (row as f32 + 0.5) / rows as f32);
                        let origin = point(
                            bounds.origin.x + px(cell_width * column as f32),
                            bounds.origin.y + px(cell_height * row as f32),
                        );
                        window.paint_quad(fill(Bounds { origin, size: painted }, Self::color_at(center)));
                    }
                }
            },
        )
        .size_full()
    }

    /// place on the surface nearest to color
    /// a color off the surface goes to the closer of the two halves
    /// eg dim and pale at once
    pub fn position_of(color: Hsla) -> Point<f32> {
        let value = color.l + color.s * color.l.min(1. - color.l);
        let saturation = if value <= 0. { 0. } else { 2. * (1. - color.l / value) };
        let y = if value >= 1. { saturation / 2. } else { 1. - value / 2. };
        point(color.h.clamp(0., 1.), y.clamp(0., 1.))
    }
}

/// hex field of the sheet, six digits and opacity has its own slider
pub enum EditorColorHex {}

impl EditorColorHex {
    pub fn text_of(color: Hsla) -> String {
        let rgba = Rgba::from(color);
        let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
        format!("{:02X}{:02X}{:02X}", channel(rgba.r), channel(rgba.g), channel(rgba.b))
    }

    /// what may stay in the field
    /// a pasted #AABBCC keeps its digits and anything else falls out
    pub fn kept_digits(typed: &str) -> String {
        typed.chars().filter(char::is_ascii_hexdigit).take(HEX_RGB_DIGITS).collect()
    }

    /// none unless typed is all six digits, the canon parser would read three as a short form
    pub fn color_of(typed: &str) -> Option<Hsla> {
        let complete = typed.len() == HEX_RGB_DIGITS && typed.bytes().all(|byte| byte.is_ascii_hexdigit());
        complete.then(|| rise_theme::color(typed))
    }
}

/// where the sheet lives
/// the screen that can open it renders one of these as a child of a container filling the window
pub struct EditorColorPickerHost {
    bridge: Entity<EditorColorPickerBridge>,
    _observation: Subscription,
}

impl EditorColorPickerHost {
    pub const LAYER_PRIORITY: usize = 1600;
    const SPECTRUM_WIDTH: f32 = 246.;
    const SPECTRUM_HEIGHT: f32 = 162.;
    const SPECTRUM_COLUMNS: usize = 41;
    const SPECTRUM_ROWS: usize = 27;
    const PADDING: f32 = 12.;
    const SPACING: f32 = 12.;
    const SLIDER_HEIGHT: f32 = 31.;
    const CLOSE_SIZE: f32 = 32.;
    const HEX_FIELD_WIDTH: f32 = 112.;
    const HEX_FONT_SIZE: f32 = 13.;
    const HEX_ROW_SPACING: f32 = 8.;
    const OPEN_GAP: f32 = 8.;
    const WINDOW_MARGIN: f32 = 8.;

    pub fn new(cx: &mut Context<Self>) -> Self {
        let bridge = EditorColorPickerBridge::shared(cx);
        let observation = cx.observe(&bridge, |_, _, cx| cx.notify());
        Self { bridge, _observation: observation }
    }

    fn sheet_height(supports_alpha: bool) -> f32 {
        let opacity_row = if supports_alpha { Self::SLIDER_HEIGHT + Self::SPACING } else { 0. };
        Self::PADDING * 2. + Self::SPECTRUM_HEIGHT + Self::SPACING + opacity_row + Self::CLOSE_SIZE
    }

    fn spectrum(position: Point<f32>, selected: Hsla) -> impl IntoElement {
        let surface = EditorColorSpectrum::surface(Self::SPECTRUM_COLUMNS, Self::SPECTRUM_ROWS);
        let on_changed = |gesture: &DragGestureValue, window: &mut Window, cx: &mut App| {
            let position = point(
                (f32::from(gesture.location.x) / Self::SPECTRUM_WIDTH).clamp(0., 1.),
                (f32::from(gesture.location.y) / Self::SPECTRUM_HEIGHT).clamp(0., 1.),
            );
            EditorColorPickerBridge::set_spectrum_position(position, window, cx);
        };
        div()
            .relative()
            .w(px(Self::SPECTRUM_WIDTH))
            .h(px(Self::SPECTRUM_HEIGHT))
            .child(surface)
            .child(
                div()
                    .absolute()
                    .left(px(position.x * Self::SPECTRUM_WIDTH - 10.))
                    .top(px(position.y * Self::SPECTRUM_HEIGHT - 10.))
                    .size(px(20.))
                    .circle()
                    .bg(selected.alpha(1.))
                    .child(centred_stroke(white(), 3., Shape::Circle)),
            )
            .gesture("editor-color-picker-spectrum", DragGesture::new(0.).on_changed(on_changed))
    }

    fn opacity_slider(bridge: Entity<EditorColorPickerBridge>) -> impl IntoElement {
        let value = Binding::new(
            move |cx| bridge.read(cx).picker_in_flight().map_or(1., |picker| f64::from(picker.selected_color.a)),
            |alpha: f64, window, cx| {
                let selected = EditorColorPickerBridge::shared(cx)
                    .read(cx)
                    .picker_in_flight()
                    .map(|picker| picker.selected_color);
                if let Some(selected) = selected {
                    let color = selected.alpha(alpha as f32);
                    EditorColorPickerBridge::color_picker_view_controller_did_select_color(color, window, cx);
                }
            },
        );
        slider("editor-color-picker-opacity", value, 0.0..=1.0).tint(white())
    }

    fn sheet(&self, picker: &EditorColorPickerInFlight, cx: &App) -> impl IntoElement {
        div()
            .id("editor-color-picker")
            .track_focus(&picker.focus_handle)
            .on_key_down(|event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    EditorColorPickerBridge::color_picker_view_controller_did_finish(window, cx);
                }
            })
            .w(px(Self::SPECTRUM_WIDTH + Self::PADDING * 2.))
            .p(px(Self::PADDING))
            .continuous_corners(18.)
            .bg(theme(cx).bg._200)
            .child(
                vstack(Self::SPACING)
                    .w_full()
                    .child(Self::spectrum(picker.spectrum_position, picker.selected_color))
                    .when(picker.supports_alpha, |sheet| {
                        sheet.child(div().w_full().child(Self::opacity_slider(self.bridge.clone())))
                    })
                    .child(
                        hstack(Self::HEX_ROW_SPACING)
                            .w_full()
                            .child(div().size(px(24.)).circle().bg(picker.selected_color))
                            .child(MainText::new("#").px(Self::HEX_FONT_SIZE).color(theme(cx).text.secondary))
                            .child(div().w(px(Self::HEX_FIELD_WIDTH)).child(picker.hex_field.clone()))
                            .child(spacer())
                            .child(
                                LiquidGlassButton::new("editor-color-picker-close", LiquidGlassButtonShape::Circle)
                                    .size(Self::CLOSE_SIZE)
                                    .on_press(EditorColorPickerBridge::color_picker_view_controller_did_finish)
                                    .children(IconUi::sized("xmark", px(14.), white(), false)),
                            ),
                    ),
            )
    }
}

impl Render for EditorColorPickerHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(picker) = self.bridge.read(cx).picker_in_flight().cloned() else {
            return div().into_any_element();
        };
        let sheet_size = size(
            px(Self::SPECTRUM_WIDTH + Self::PADDING * 2.),
            px(Self::sheet_height(picker.supports_alpha)),
        );
        let placement = place(
            OverlayAnchor::Point(picker.anchor),
            sheet_size,
            window.viewport_size(),
            px(Self::OPEN_GAP),
            px(Self::WINDOW_MARGIN),
            OverlaySide::Above,
        );
        let sheet = self.sheet(&picker, cx);
        let layer = OverlayLayer::new(placement)
            .priority(Self::LAYER_PRIORITY)
            .on_dismiss(EditorColorPickerBridge::color_picker_view_controller_did_finish)
            .child(sheet)
            .render(cx);
        div().absolute().top_0().left_0().size_full().child(layer).into_any_element()
    }
}
