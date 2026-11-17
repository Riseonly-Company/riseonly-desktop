use std::collections::BTreeMap;

use gpui::{App, Bounds, DisplayId, Pixels, PlatformDisplay, Window, WindowBounds, point, px, size};
use rise_platform::user_defaults::{DefaultsValue, UserDefaults};
use rise_platform::user_defaults_keys;

/// how the remembered window stood when last moved, resized or zoomed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowFrameMode {
    Windowed,
    Maximized,
    Fullscreen,
}

impl WindowFrameMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Windowed => "windowed",
            Self::Maximized => "maximized",
            Self::Fullscreen => "fullscreen",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "windowed" => Some(Self::Windowed),
            "maximized" => Some(Self::Maximized),
            "fullscreen" => Some(Self::Fullscreen),
            _ => None,
        }
    }

    pub fn window_bounds(self, bounds: Bounds<Pixels>) -> WindowBounds {
        match self {
            Self::Windowed => WindowBounds::Windowed(bounds),
            Self::Maximized => WindowBounds::Maximized(bounds),
            Self::Fullscreen => WindowBounds::Fullscreen(bounds),
        }
    }
}

/// frame of the last window the person touched, the next launch opens its first window there
/// bounds are the restore bounds in gpui coordinates for the display
#[derive(Clone, Debug, PartialEq)]
pub struct WindowFrame {
    pub mode: WindowFrameMode,
    pub bounds: Bounds<Pixels>,
    pub display: Option<String>,
}

impl WindowFrame {
    pub fn of(window: &Window, cx: &App) -> Self {
        let (reported, bounds) = match window.window_bounds() {
            WindowBounds::Windowed(bounds) => (WindowFrameMode::Windowed, bounds),
            WindowBounds::Maximized(bounds) => (WindowFrameMode::Maximized, bounds),
            WindowBounds::Fullscreen(bounds) => (WindowFrameMode::Fullscreen, bounds),
        };
        let mode = if window.is_fullscreen() {
            WindowFrameMode::Fullscreen
        } else {
            reported
        };
        let display = window
            .display(cx)
            .and_then(|display| display_identity(display.as_ref()));

        Self { mode, bounds, display }
    }

    pub fn to_defaults(&self) -> DefaultsValue {
        let mut fields = BTreeMap::new();
        fields.insert("mode".to_owned(), DefaultsValue::from(self.mode.name()));
        fields.insert("x".to_owned(), DefaultsValue::from(f64::from(self.bounds.origin.x)));
        fields.insert("y".to_owned(), DefaultsValue::from(f64::from(self.bounds.origin.y)));
        fields.insert(
            "width".to_owned(),
            DefaultsValue::from(f64::from(self.bounds.size.width)),
        );
        fields.insert(
            "height".to_owned(),
            DefaultsValue::from(f64::from(self.bounds.size.height)),
        );
        if let Some(display) = &self.display {
            fields.insert("display".to_owned(), DefaultsValue::from(display.as_str()));
        }
        DefaultsValue::from(fields)
    }

    /// none for anything a damaged or foreign preferences file could hold instead of a frame
    pub fn from_defaults(value: &DefaultsValue) -> Option<Self> {
        let fields = value.as_dictionary()?;
        let number = |name: &str| {
            fields
                .get(name)
                .and_then(DefaultsValue::as_f64)
                .filter(|value| value.is_finite())
        };

        let mode = WindowFrameMode::from_name(fields.get("mode")?.as_str()?)?;
        let x = number("x")?;
        let y = number("y")?;
        let width = number("width")?;
        let height = number("height")?;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let display = fields
            .get("display")
            .and_then(DefaultsValue::as_str)
            .map(str::to_owned);

        Some(Self {
            mode,
            bounds: Bounds::new(
                point(px(x as f32), px(y as f32)),
                size(px(width as f32), px(height as f32)),
            ),
            display,
        })
    }

    /// a maximized or fullscreen frame keeps the restore bounds of the frame saved before it
    pub fn keeping_restore_bounds(self, previous: Option<&WindowFrame>) -> Self {
        match (self.mode, previous) {
            (WindowFrameMode::Windowed, _) | (_, None) => self,
            (_, Some(previous)) => Self {
                bounds: previous.bounds,
                ..self
            },
        }
    }

    /// remembered bounds pulled fully inside visible
    /// a shrunk display or grown dock must not leave the title bar out of reach
    pub fn fitted(&self, visible: Bounds<Pixels>) -> Bounds<Pixels> {
        let fitted_size = self.bounds.size.min(&visible.size);
        let x = self
            .bounds
            .origin
            .x
            .min(visible.right() - fitted_size.width)
            .max(visible.origin.x);
        let y = self
            .bounds
            .origin
            .y
            .min(visible.bottom() - fitted_size.height)
            .max(visible.origin.y);

        Bounds::new(point(x, y), fitted_size)
    }
}

/// window size and position survive a relaunch
/// every open window writes and the last one moved wins
pub struct WindowFrameMemory;

impl WindowFrameMemory {
    pub fn remember(window: &Window, cx: &App) {
        let frame = WindowFrame::of(window, cx).keeping_restore_bounds(Self::recall().as_ref());
        UserDefaults::standard().set(user_defaults_keys::DESKTOP_WINDOW_FRAME, frame.to_defaults());
    }

    pub fn recall() -> Option<WindowFrame> {
        UserDefaults::standard()
            .object(user_defaults_keys::DESKTOP_WINDOW_FRAME)
            .as_ref()
            .and_then(WindowFrame::from_defaults)
    }

    /// where the first window of a launch opens, a display that is gone hands the frame to the primary one
    pub fn restored(cx: &App) -> Option<(WindowBounds, DisplayId)> {
        let frame = Self::recall()?;
        let remembered = frame.display.as_deref().and_then(|saved| {
            cx.displays()
                .into_iter()
                .find(|display| display_identity(display.as_ref()).as_deref() == Some(saved))
        });
        let display = remembered.or_else(|| cx.primary_display())?;
        let bounds = frame.fitted(display.visible_bounds());

        Some((frame.mode.window_bounds(bounds), display.id()))
    }
}

fn display_identity(display: &dyn PlatformDisplay) -> Option<String> {
    display.uuid().ok().map(|identity| identity.to_string())
}
