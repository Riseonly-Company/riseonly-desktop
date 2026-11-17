use gpui::{App, Bounds, Canvas, ElementId, Pixels, Styled, Window, canvas};

/// bounds the measurer reported last kept in element state so the handler runs only on real changes
/// notifying every frame would redraw forever
struct LastBounds(Bounds<Pixels>);

/// transparent layer covering its parent, add it as a child of the parent
/// action runs after the frame once parent size is known and on change
/// takes no clicks
pub fn measure(
    id: impl Into<ElementId>,
    action: impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static,
) -> Canvas<()> {
    let id = id.into();
    canvas(
        move |bounds, window, cx| {
            window.with_global_id(id, |global_id, window| {
                window.with_element_state::<LastBounds, _>(global_id, |last, window| {
                    if last.as_ref().is_none_or(|last| last.0 != bounds) {
                        window.defer(cx, move |window, cx| action(bounds, window, cx));
                    }
                    ((), LastBounds(bounds))
                })
            })
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}
