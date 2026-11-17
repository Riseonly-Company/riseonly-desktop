use std::rc::Rc;

use gpui::{App, Context, Entity, Window};

type BindingGetter<T> = Rc<dyn Fn(&App) -> T>;
type BindingSetter<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

pub struct Binding<T> {
    get: BindingGetter<T>,
    set: BindingSetter<T>,
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            get: self.get.clone(),
            set: self.set.clone(),
        }
    }
}

impl<T: 'static> Binding<T> {
    /// binding from a getter and a setter
    pub fn new(
        get: impl Fn(&App) -> T + 'static,
        set: impl Fn(T, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            get: Rc::new(get),
            set: Rc::new(set),
        }
    }

    /// binding to a field of an entity, writes inside its update and then notifies
    /// so every view watching the store re-renders
    pub fn entity<E: 'static>(
        entity: Entity<E>,
        get: impl Fn(&E) -> T + 'static,
        set: impl Fn(&mut E, T, &mut Context<E>) + 'static,
    ) -> Self {
        let reader = entity.clone();
        Self {
            get: Rc::new(move |cx| get(reader.read(cx))),
            set: Rc::new(move |value, _window, cx| {
                entity.update(cx, |state, cx| {
                    set(state, value, cx);
                    cx.notify();
                });
            }),
        }
    }

    pub fn get(&self, cx: &App) -> T {
        (self.get)(cx)
    }

    pub fn set(&self, value: T, window: &mut Window, cx: &mut App) {
        (self.set)(value, window, cx);
    }
}

impl<T: Clone + 'static> Binding<T> {
    /// binding with a fixed value, writes are dropped
    pub fn constant(value: T) -> Self {
        Self {
            get: Rc::new(move |_| value.clone()),
            set: Rc::new(|_, _, _| {}),
        }
    }
}

impl Binding<bool> {
    /// flips the bool value
    pub fn toggle(&self, window: &mut Window, cx: &mut App) {
        let current = self.get(cx);
        self.set(!current, window, cx);
    }
}
