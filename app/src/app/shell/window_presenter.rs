use gpui::{
    App, AppContext, Bounds, Context, Render, TitlebarOptions, Window, WindowBounds, WindowHandle,
    WindowOptions, point, size,
};
use rise_platform::HostOs;
use rise_platform::app_info::identity_for;
use rise_platform::materials::{apply_window_material, preferred_window_material};
use rise_platform::window_chrome::{round_window_corner, shell_window_decorations};

use crate::core::push_notifications::socket_notification_source::SocketNotificationSource;
use crate::app::root_view::RootView;
use crate::app::shell::window_frame_memory::WindowFrameMemory;
use crate::app::shell::window_title_bar::shell_chrome;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::tray_controller::LaunchPresentation;

pub struct WindowPresenter;

impl WindowPresenter {
    pub fn open_shell(cx: &mut App) {
        Self::open(cx, RootView::new);
    }

    pub fn open_shell_for_launch(presentation: LaunchPresentation, cx: &mut App) {
        match presentation {
            LaunchPresentation::Window => Self::open_shell(cx),
            LaunchPresentation::HiddenWindow => {
                Self::open_with_visibility(cx, false, RootView::new);
            }
            LaunchPresentation::MinimizedWindow => {
                if let Some(handle) = Self::open_with_visibility(cx, true, RootView::new) {
                    let minimized = handle.update(cx, |_, window, _| window.minimize_window());

                    if minimized.is_err() {
                        tracing::warn!(target: "riseonly", "the window closed before it could be minimized");
                    }
                }
            }
            LaunchPresentation::NoWindow => {}
        }
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
        Self::open_with_visibility(cx, true, build)
    }

    fn open_with_visibility<V: Render + 'static>(
        cx: &mut App,
        show: bool,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
    ) -> Option<WindowHandle<V>> {
        let metrics = rise_ui::theme(cx).shell;
        let existing = Self::window_count(cx);
        let restored = if existing == 0 {
            WindowFrameMemory::restored(cx)
        } else {
            None
        };

        let (window_bounds, display_id) = match restored {
            Some((window_bounds, display_id)) => (window_bounds, Some(display_id)),
            None => {
                let mut bounds = Bounds::centered(
                    None,
                    size(metrics.window_default_width, metrics.window_default_height),
                    cx,
                );
                let step = metrics.window_cascade_offset * cascade_step(existing);
                bounds.origin += point(step, step);
                (WindowBounds::Windowed(bounds), None)
            }
        };

        let host = HostOs::current();
        let requested = preferred_window_material(host);
        let decorations = shell_window_decorations(&shell_chrome(host, &metrics));

        let handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(window_bounds),
                    window_min_size: Some(metrics.window_min_size()),
                    display_id,
                    window_background: requested.into_gpui(),
                    window_decorations: Some(decorations),
                    app_id: Self::app_id(),
                    focus: show,
                    show,
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

        let rounded = handle.update(cx, |_, window, _| round_window_corner(window));

        if !matches!(rounded, Ok(rise_platform::PlatformSupport::Performed)) {
            tracing::info!(
                target: "riseonly",
                "the window kept the square corner gpui opens it with"
            );
        }

        let observed = handle.update(cx, |_, window, cx| {
            WindowFrameMemory::remember(window, cx);
            cx.observe_window_bounds(window, |_, window, cx| {
                WindowFrameMemory::remember(window, cx);
            })
            .detach();
        });

        if observed.is_err() {
            tracing::warn!(
                target: "riseonly",
                "the window closed before its frame could be remembered"
            );
        }

        let should_close = handle.update(cx, |_, window, cx| {
            window.on_window_should_close(cx, SocketNotificationSource::close_or_keep);
        });
        if should_close.is_err() {
            tracing::warn!(target: "riseonly", "the window closed before its closing rule was set");
        }

        Some(handle)
    }

    /// wayland app_id and x11 wm_class, the shell ties a window to its desktop entry by this
    /// every window of the app carries it
    pub fn app_id() -> Option<String> {
        identity_for(AppEnvironment::compiled().as_str()).map(|identity| identity.bundle_identifier.to_owned())
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
    fn the_window_never_narrows_below_the_rail_and_the_narrowest_iphone_layout() {
        let metrics = rise_theme::AppTheme::dark().shell;
        let minimum = metrics.window_min_size();
        assert_eq!(
            minimum.width,
            metrics.rail_width + metrics.rail_outer_inset * 2.0 + metrics.content_min_width
        );
        assert_eq!(minimum.height, metrics.window_min_height);
        assert!(minimum.width <= metrics.window_default_width);
        assert!(minimum.height <= metrics.window_default_height);
    }

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
