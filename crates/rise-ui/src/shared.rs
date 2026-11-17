use gpui::{App, AppContext, Context, Entity, Global};

struct SharedEntity<T: 'static>(Entity<T>);

impl<T: 'static> Global for SharedEntity<T> {}

/// returns the app wide Entity, built with new on the first call only
/// new must not ask for the same shared again, the slot fills after it returns
/// a nested call builds another instance and recurses forever
pub fn shared<T: 'static>(cx: &mut App, new: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
    if let Some(existing) = cx.try_global::<SharedEntity<T>>() {
        return existing.0.clone();
    }
    let entity = cx.new(new);
    cx.set_global(SharedEntity(entity.clone()));
    entity
}

/// returns the app wide T if shared already built it, never builds it
/// for read only callers with just &App
pub fn try_read<T: 'static>(cx: &App) -> Option<&T> {
    cx.try_global::<SharedEntity<T>>().map(|existing| existing.0.read(cx))
}
