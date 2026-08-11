use gpui::{
    App, AppContext, Bounds, Context, Render, TitlebarOptions, Window, WindowBounds, WindowHandle,
    WindowOptions, point, size,
};
use rise_platform::HostOs;
use rise_platform::materials::{apply_window_material, preferred_window_material};
use rise_platform::window_chrome::round_window_corner;

use crate::app::root_view::RootView;
use crate::core::config::AppEnvironment;

pub struct WindowPresenter;

impl WindowPresenter {
    pub fn open_shell(cx: &mut App) {
        Self::open(cx, RootView::new);
    }

    pub fn open<V: Render + 'static>(
        cx: &mut App,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
    ) {
        Self::open_handle(cx, build);
    }

    pub fn open_handle<V: Render + 'static>(
        cx: &mut App,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
    ) -> Option<WindowHandle<V>> {
        let metrics = rise_ui::theme(cx).shell;
        let existing = Self::window_count(cx);
        let mut bounds = Bounds::centered(
            None,
            size(metrics.window_default_width, metrics.window_default_height),
            cx,
        );

        let step = metrics.window_cascade_offset * cascade_step(existing);
        bounds.origin += point(step, step);

        let requested = preferred_window_material(HostOs::current());

        let handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    // The mac answer here is NOT opaque, and not for a material:
                    // a rounded corner is a piece of the window cut away, and an
                    // opaque window has nowhere to cut it out of. See
                    // `preferred_window_material`. Set at open as well as
                    // through `apply_window_material` below so the very first
                    // frame is already right rather than flashing one that is
                    // not.
                    window_background: requested.into_gpui(),
                    titlebar: Some(TitlebarOptions {
                        title: Some(AppEnvironment::compiled().display_name().into()),
                        appears_transparent: true,
                        traffic_light_position: Some(metrics.traffic_light_origin()),
                    }),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| build(window, cx)),
            )
            .inspect_err(|error| {
                tracing::error!(target: "riseonly", "failed to open a window: {error}");
            })
            .ok()?;

        let granted = handle.update(cx, |_, window, _| apply_window_material(window, requested));

        if !matches!(granted, Ok(rise_platform::PlatformSupport::Performed)) {
            tracing::info!(
                target: "riseonly",
                "window material {requested:?} was not granted here; the painted tier stands in"
            );
        }

        // After the material, never before it: the mask cuts the corner away and
        // only a window that is not opaque has anywhere for the cut to go.
        let rounded = handle.update(cx, |_, window, _| round_window_corner(window));

        if !matches!(rounded, Ok(rise_platform::PlatformSupport::Performed)) {
            tracing::info!(
                target: "riseonly",
                "the window kept the square corner gpui opens it with"
            );
        }

        Some(handle)
    }

    pub fn window_count(cx: &App) -> usize {
        cx.windows().len()
    }
}

fn cascade_step(existing: usize) -> f32 {
    const WRAP: usize = 6;
    (existing % WRAP) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_window_opens_centred_and_the_next_ones_step_off_it() {
        assert_eq!(cascade_step(0), 0.0);
        assert_eq!(cascade_step(1), 1.0);
        assert_eq!(cascade_step(5), 5.0);
    }

    #[test]
    fn the_cascade_wraps_instead_of_walking_off_the_screen() {
        assert_eq!(cascade_step(6), 0.0);
        assert_eq!(cascade_step(13), 1.0);
        assert!((0..200).all(|n| cascade_step(n) < 6.0));
    }
}
