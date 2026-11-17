use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{
    AnyElement, AnyWeakView, App, Bounds, ElementId, IntoElement, ParentElement, Pixels, Point, RenderOnce, Styled,
    Window, canvas, px,
};
use rise_core::foundation::foundation_uuid::Uuid;

struct BubbleScrollPassthroughRegion {
    window_frame: Bounds<Pixels>,
    touch_target: Option<AnyWeakView>,
}

#[derive(Default)]
pub struct BubbleScrollPassthrough {
    regions: HashMap<Uuid, BubbleScrollPassthroughRegion>,
}

thread_local! {
    static SHARED: RefCell<BubbleScrollPassthrough> = RefCell::new(BubbleScrollPassthrough::default());
}

impl BubbleScrollPassthrough {
    /// runs f on the shared registry, one per ui thread (the one gpui draws on)
    pub fn with_shared<R>(f: impl FnOnce(&mut BubbleScrollPassthrough) -> R) -> R {
        SHARED.with(|shared| f(&mut shared.borrow_mut()))
    }

    pub fn register(&mut self, window_frame: Bounds<Pixels>, touch_target: Option<AnyWeakView>) -> Uuid {
        let id = Uuid::new_v4();
        self.regions.insert(id, BubbleScrollPassthroughRegion { window_frame, touch_target });
        id
    }

    pub fn update(&mut self, id: Uuid, window_frame: Bounds<Pixels>) {
        if let Some(region) = self.regions.get_mut(&id) {
            region.window_frame = window_frame;
        }
    }

    pub fn unregister(&mut self, id: Uuid) {
        self.regions.remove(&id);
    }

    pub fn contains(&self, point: Point<Pixels>) -> bool {
        self.regions.values().any(|region| region.window_frame.contains(&point))
    }

    /// view that must own a press at this point
    /// an overlay above a scroller hands the press to the scroller
    pub fn touch_target(&self, at_point: Point<Pixels>) -> Option<&AnyWeakView> {
        self.regions.values().find_map(|region| {
            let target = region.touch_target.as_ref()?;
            target.upgrade()?;
            region.window_frame.contains(&at_point).then_some(target)
        })
    }
}

#[derive(Default)]
struct ReporterView {
    id: Option<Uuid>,
}

impl ReporterView {
    fn did_move_to_window(&mut self, frame: Option<Bounds<Pixels>>) {
        match frame {
            Some(frame) => self.sync(frame),
            None => self.unregister(),
        }
    }

    fn sync(&mut self, window_frame: Bounds<Pixels>) {
        if !(window_frame.size.width > px(1.0) && window_frame.size.height > px(1.0)) {
            self.unregister();
            return;
        }
        BubbleScrollPassthrough::with_shared(|shared| match self.id {
            Some(id) => shared.update(id, window_frame),
            None => self.id = Some(shared.register(window_frame, None)),
        });
    }

    fn unregister(&mut self) {
        if let Some(id) = self.id.take() {
            BubbleScrollPassthrough::with_shared(|shared| shared.unregister(id));
        }
    }
}

impl Drop for ReporterView {
    fn drop(&mut self) {
        self.unregister();
    }
}

#[derive(IntoElement)]
struct BubbleScrollPassthroughReporter {
    id: ElementId,
}

impl RenderOnce for BubbleScrollPassthroughReporter {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = self.id;
        canvas(
            move |bounds, window, _cx| {
                let region = window.with_global_id(id, |global_id, window| {
                    window.with_element_state::<ReporterView, _>(global_id, |view, _| {
                        let mut view = view.unwrap_or_default();
                        view.did_move_to_window(Some(bounds));
                        (view.id, view)
                    })
                });
                if let Some(region) = region {
                    window.on_replayed_move(move |offset, _, _, _| {
                        let moved = Bounds { origin: bounds.origin + offset, size: bounds.size };
                        BubbleScrollPassthrough::with_shared(|shared| shared.update(region, moved));
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}

/// adds a reporter behind the element children, takes no clicks
pub trait BubbleScrollPassthroughModifiers: ParentElement + Sized {
    /// id names the reporter among its siblings
    fn bubble_scroll_passthrough(mut self, id: impl Into<ElementId>) -> Self {
        let reporter: AnyElement = BubbleScrollPassthroughReporter { id: id.into() }.into_any_element();
        self.extend(std::iter::once(reporter));
        self
    }
}

impl<T: ParentElement> BubbleScrollPassthroughModifiers for T {}
