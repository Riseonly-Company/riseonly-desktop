use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Context, IntoElement, ParentElement, Render, RenderOnce, TestAppContext,
    VisualTestContext, Window, div, px,
};

use super::environment::{
    EnvKey, LayoutDirection, LayoutDirectionKey, ScenePhase, ScenePhaseKey, data_fingerprint, env,
    env_scope, layout_direction, scene_phase, set_scene_phase,
};

struct DepthKey;

impl EnvKey for DepthKey {
    type Value = u32;

    fn default_value() -> Self::Value {
        7
    }
}

type Seen = Rc<RefCell<Vec<(&'static str, u32)>>>;

#[derive(IntoElement)]
struct Probe {
    tag: &'static str,
    seen: Seen,
}

impl RenderOnce for Probe {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.seen.borrow_mut().push((self.tag, env::<DepthKey>(cx)));
        div()
    }
}

struct Nested {
    seen: Seen,
}

impl Render for Nested {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let probe = |tag| Probe {
            tag,
            seen: self.seen.clone(),
        };
        div()
            .child(probe("outside"))
            .child(
                env_scope::<DepthKey>(1)
                    .child(probe("outer"))
                    .child(env_scope::<DepthKey>(2).child(probe("inner")))
                    .child(probe("outer_after_inner")),
            )
            .child(probe("after_scope"))
    }
}

fn draw_nested(cx: &mut TestAppContext) -> Seen {
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let probe = seen.clone();
    let window = cx.add_window(|_, _| Nested { seen: probe });
    let visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    seen
}

#[gpui::test]
fn env_without_a_scope_is_the_key_default(cx: &mut TestAppContext) {
    cx.update(|cx| assert_eq!(env::<DepthKey>(cx), 7));
}

#[gpui::test]
fn nested_scopes_read_the_top_of_the_stack(cx: &mut TestAppContext) {
    let seen = draw_nested(cx);
    let seen = seen.borrow();
    let value_of = |tag| {
        seen.iter()
            .find(|(seen_tag, _)| *seen_tag == tag)
            .map(|(_, value)| *value)
    };
    assert_eq!(value_of("outside"), Some(7));
    assert_eq!(value_of("outer"), Some(1));
    assert_eq!(value_of("inner"), Some(2));
    assert_eq!(
        value_of("outer_after_inner"),
        Some(1),
        "the inner scope must pop its value once its children are laid out"
    );
    assert_eq!(value_of("after_scope"), Some(7));
}

#[gpui::test]
fn the_stack_is_empty_after_the_frame(cx: &mut TestAppContext) {
    draw_nested(cx);
    cx.update(|cx| assert_eq!(env::<DepthKey>(cx), 7));
}

#[gpui::test]
fn layout_direction_defaults_to_left_to_right(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert_eq!(LayoutDirectionKey::default_value(), LayoutDirection::LeftToRight);
        assert_eq!(layout_direction(cx), LayoutDirection::LeftToRight);
    });
}

#[gpui::test]
fn a_window_without_a_root_scope_reads_the_installed_direction(cx: &mut TestAppContext) {
    cx.update(|cx| {
        LayoutDirection::install(LayoutDirection::RightToLeft);
        let without_scope = LayoutDirectionKey::default_value();
        let read = layout_direction(cx);
        LayoutDirection::install(LayoutDirection::LeftToRight);
        assert_eq!(without_scope, LayoutDirection::RightToLeft);
        assert_eq!(read, LayoutDirection::RightToLeft);
    });
}

#[gpui::test]
fn scene_phase_follows_what_the_shell_published(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert_eq!(scene_phase(cx), ScenePhase::Active);
        set_scene_phase(ScenePhase::Background, cx);
        assert_eq!(scene_phase(cx), ScenePhase::Background);
        assert_eq!(ScenePhaseKey::default_value(), None);
    });
}

#[test]
fn data_fingerprint_follows_plain_values_only() {
    assert_eq!(data_fingerprint(&1.5_f32), data_fingerprint(&1.5_f32));
    assert_ne!(data_fingerprint(&1.5_f32), data_fingerprint(&2.0_f32));
    assert_ne!(data_fingerprint(&px(4.0)), data_fingerprint(&px(6.0)));
    assert!(data_fingerprint(&Some(LayoutDirection::RightToLeft)).is_none());
    assert!(data_fingerprint(&LayoutDirection::RightToLeft).is_some());
    assert!(data_fingerprint(&Rc::new(1_u8)).is_none());
}
