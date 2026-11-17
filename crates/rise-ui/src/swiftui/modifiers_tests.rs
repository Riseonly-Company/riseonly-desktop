use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    TestAppContext, Window, div, px,
};

use crate::swiftui::environment::{EnvKey, env, env_scope};
use crate::swiftui::modifiers::{ModifierExt, ViewModifier};

const ROOT: &str = "root";
const CHIP: &str = "chip";
const INSET: &str = "inset";

struct InsetKey;

impl EnvKey for InsetKey {
    type Value = f32;

    fn default_value() -> Self::Value {
        4.0
    }
}

type Seen = Rc<RefCell<Vec<f32>>>;

struct InsetModifier {
    seen: Seen,
}

impl ViewModifier for InsetModifier {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        let inset = env::<InsetKey>(cx);
        self.seen.borrow_mut().push(inset);
        div()
            .debug_selector(|| INSET.to_string())
            .p(px(inset))
            .child(content)
            .into_any_element()
    }
}

struct Probe {
    seen: Seen,
    scoped: bool,
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let chip = div()
            .debug_selector(|| CHIP.to_string())
            .w(px(20.))
            .h(px(10.))
            .modifier(InsetModifier {
                seen: self.seen.clone(),
            });
        let root = div()
            .debug_selector(|| ROOT.to_string())
            .flex()
            .items_start()
            .w(px(200.))
            .h(px(200.));
        if self.scoped {
            root.child(env_scope::<InsetKey>(10.0).child(chip))
        } else {
            root.child(chip)
        }
    }
}

fn draw(cx: &mut TestAppContext, scoped: bool) -> (Seen, (f32, f32, f32, f32)) {
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let probe = seen.clone();
    let (_, cx) = cx.add_window_view(move |_, _| Probe { seen: probe, scoped });
    cx.run_until_parked();
    let root = cx.debug_bounds(ROOT).expect("probe root was laid out");
    let chip = cx.debug_bounds(CHIP).expect("content was laid out");
    let inset = cx.debug_bounds(INSET).expect("modifier view was laid out");
    let geometry = (
        f32::from(chip.origin.x - root.origin.x),
        f32::from(chip.origin.y - root.origin.y),
        f32::from(inset.size.width),
        f32::from(inset.size.height),
    );
    (seen, geometry)
}

#[gpui::test]
fn a_modifier_wraps_its_content(cx: &mut TestAppContext) {
    let (seen, (x, y, width, height)) = draw(cx, false);
    assert_eq!(seen.borrow().last().copied(), Some(4.0));
    assert_eq!((x, y), (4.0, 4.0));
    assert_eq!((width, height), (28.0, 18.0));
}

#[gpui::test]
fn a_modifier_body_reads_the_subtree_environment(cx: &mut TestAppContext) {
    let (seen, (x, y, width, height)) = draw(cx, true);
    assert_eq!(seen.borrow().last().copied(), Some(10.0));
    assert_eq!((x, y), (10.0, 10.0));
    assert_eq!((width, height), (40.0, 30.0));
}
