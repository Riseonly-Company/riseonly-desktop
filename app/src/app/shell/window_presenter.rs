use gpui::{
    App, AppContext, Bounds, Context, Render, TitlebarOptions, Window, WindowBounds, WindowHandle,
    WindowOptions, point, size,
};
use rise_platform::HostOs;
use rise_platform::materials::{apply_window_material, preferred_window_material};

use crate::app::root_view::RootView;
use crate::core::config::AppEnvironment;

/// Opens and counts the app's windows.
///
/// ONE ACCOUNT PER PROCESS, MANY WINDOWS ONTO IT. `rise-engine` holds one engine
/// per account per process behind a lease on the database path, and the account
/// root entity is dropped whole when the account changes; a second account in a
/// second window would mean a second engine, a second database and two teardown
/// orders in one process, which is the thing the ownership model exists to
/// avoid. Two accounts at once is therefore a second process, and each process
/// already has its own advisory lock. Every window here is a view onto the same
/// account, with its own navigation stacks and its own overlays.
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

        let handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    // No titlebar strip. The reference is a phone and has none;
                    // Telegram for macOS has none either, and the shell's whole
                    // premise is that the rail and the list run to the top edge.
                    // The traffic lights stay — a macOS window without them is
                    // one the user cannot close — and are inset over the content,
                    // which is what `shell.traffic_light_inset` reserves room for.
                    titlebar: Some(TitlebarOptions {
                        title: Some(AppEnvironment::compiled().display_name().into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(
                            metrics.traffic_light_inset,
                            metrics.traffic_light_inset,
                        )),
                    }),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| build(window, cx)),
            )
            .inspect_err(|error| {
                tracing::error!(target: "riseonly", "failed to open a window: {error}");
            })
            .ok()?;

        // Tier 0 of docs/PLATFORM_GLASS.md, and the only tier that is free:
        // gpui_macos already hosts an NSVisualEffectView behind a transparent
        // Metal layer. Elsewhere the request is downgraded to opaque rather than
        // forwarded — see granted_window_material for why a see-through window
        // with no compositor blur is worse than never asking for one.
        let requested = preferred_window_material(HostOs::current());
        let granted = handle.update(cx, |_, window, _| apply_window_material(window, requested));

        if !matches!(granted, Ok(rise_platform::PlatformSupport::Performed)) {
            tracing::info!(
                target: "riseonly",
                "window material {requested:?} was not granted here; the painted tier stands in"
            );
        }

        Some(handle)
    }

    pub fn window_count(cx: &App) -> usize {
        cx.windows().len()
    }
}

/// How many cascade steps in the nth window opens.
///
/// It wraps, because a user who opens twelve windows should get the twelfth back
/// near the first rather than off the bottom of the screen.
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
