use gpui::{
    AnyElement, App, AsyncApp, AsyncWindowContext, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Task, Window,
};

type LifecycleHandler = Box<dyn FnOnce(&mut Window, &mut App)>;
type DisappearHandler = Box<dyn FnOnce(Option<&mut Window>, &mut App)>;

/// on_appear and on_disappear for any element
/// id has to stay the same across frames and be unique among siblings, so a row passes its domain id
pub trait LifecycleModifiers: IntoElement + Sized {
    /// runs once each time the element starts being drawn
    fn on_appear(
        self,
        id: impl Into<ElementId>,
        action: impl FnOnce(&mut Window, &mut App) + 'static,
    ) -> Lifecycle {
        Lifecycle {
            id: id.into(),
            child: self.into_any_element(),
            on_appear: Some(Box::new(action)),
            on_disappear: None,
        }
    }

    /// runs once when the element stops being drawn, latest frame handler wins
    /// window is none when the element went away because its window closed
    fn on_disappear(
        self,
        id: impl Into<ElementId>,
        action: impl FnOnce(Option<&mut Window>, &mut App) + 'static,
    ) -> Lifecycle {
        Lifecycle {
            id: id.into(),
            child: self.into_any_element(),
            on_appear: None,
            on_disappear: Some(Box::new(action)),
        }
    }
}

impl<E: IntoElement> LifecycleModifiers for E {}

/// element from the lifecycle modifiers, no layout node of its own
pub struct Lifecycle {
    id: ElementId,
    child: AnyElement,
    on_appear: Option<LifecycleHandler>,
    on_disappear: Option<DisappearHandler>,
}

impl Lifecycle {
    /// both handlers on one view without a second wrapper
    pub fn and_on_disappear(
        mut self,
        action: impl FnOnce(Option<&mut Window>, &mut App) + 'static,
    ) -> Self {
        self.on_disappear = Some(Box::new(action));
        self
    }
}

impl IntoElement for Lifecycle {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// lives in element state as long as the element is drawn
struct Presence {
    on_disappear: Option<DisappearHandler>,
    cx: AsyncWindowContext,
}

impl Drop for Presence {
    fn drop(&mut self) {
        if let Some(action) = self.on_disappear.take() {
            let mut cx = self.cx.clone();
            self.cx
                .foreground_executor()
                .spawn(async move {
                    let mut pending = Some(action);
                    let _ = cx.update(|window, cx| {
                        if let Some(action) = pending.take() {
                            action(Some(window), cx);
                        }
                    });
                    if let Some(action) = pending {
                        let app: &AsyncApp = &cx;
                        app.update(|cx| action(None, cx));
                    }
                })
                .detach();
        }
    }
}

impl Element for Lifecycle {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(global_id) = id {
            let on_appear = self.on_appear.take();
            let on_disappear = self.on_disappear.take();
            window.with_element_state::<Presence, _>(global_id, |presence, window| {
                let presence = match presence {
                    Some(mut presence) => {
                        presence.on_disappear = on_disappear;
                        presence
                    }
                    None => {
                        if let Some(action) = on_appear {
                            window.defer(cx, action);
                        }
                        Presence {
                            on_disappear,
                            cx: window.to_async(cx),
                        }
                    }
                };
                ((), presence)
            });
        }
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

struct TaskSlot<T> {
    task_id: T,
    _task: Task<()>,
}

/// async task called from render, starts on first render
/// restarts when task_id changes or the view appears again, old one cancelled first
/// cancelled when the view stops rendering
pub fn use_task<T: PartialEq + 'static>(
    id: impl Into<ElementId>,
    task_id: T,
    window: &mut Window,
    cx: &mut App,
    action: impl AsyncFnOnce(&mut AsyncWindowContext) + 'static,
) {
    window.with_global_id(id.into(), |global_id, window| {
        window.with_element_state::<TaskSlot<T>, _>(global_id, |slot, window| {
            if let Some(slot) = slot {
                if slot.task_id == task_id {
                    return ((), slot);
                }
                drop(slot);
            }
            let task = window.spawn(cx, action);
            ((), TaskSlot { task_id, _task: task })
        })
    });
}

struct ChangeSlot<V> {
    value: V,
}

/// runs action with old and new when the value changes, called from render
/// after the frame never during render, initial runs it once on first render
pub fn on_change<V: Clone + PartialEq + 'static>(
    id: impl Into<ElementId>,
    value: V,
    initial: bool,
    window: &mut Window,
    cx: &mut App,
    action: impl FnOnce(&V, &V, &mut Window, &mut App) + 'static,
) {
    let mut created = false;
    let slot = window.use_keyed_state(id, cx, |_, _| {
        created = true;
        ChangeSlot {
            value: value.clone(),
        }
    });
    if created {
        if initial {
            let old = value.clone();
            window.defer(cx, move |window, cx| action(&old, &value, window, cx));
        }
        return;
    }
    if slot.read(cx).value == value {
        return;
    }
    let old = slot.update(cx, |slot, _| std::mem::replace(&mut slot.value, value.clone()));
    window.defer(cx, move |window, cx| action(&old, &value, window, cx));
}
