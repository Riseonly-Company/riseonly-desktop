use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext, Bounds, Context, Empty, Entity, IntoElement, Render, TestAppContext, Window, point, px,
    size,
};
use rise_theme::Appearance;
use rise_theme::ui_color_system::UIColor;

use super::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListItemProvider, RiseManualListReusableView,
    RiseManualListSelectionItem, RiseManualListSelectionLayout, RiseManualListSelectionPresentable,
    RiseManualListSelectionPresentation, RiseManualListTransaction, RiseManualListTransactionAnchor,
};
use super::rise_manual_list_view::RiseManualListView;
use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListConformances;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;

#[derive(Default)]
struct RiseManualListSelectionSpyView {
    presentations: Vec<(bool, f32)>,
}

impl Render for RiseManualListSelectionSpyView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

impl RiseManualListReusableView for RiseManualListSelectionSpyView {
    fn prepare_for_manual_list_reuse(&mut self, _cx: &mut Context<Self>) {
        self.presentations.clear();
    }
}

impl RiseManualListSelectionPresentable for RiseManualListSelectionSpyView {
    fn apply_manual_list_selection_presentation(
        &mut self,
        is_active: bool,
        content_translation_x: f32,
        _cx: &mut Context<Self>,
    ) {
        self.presentations.push((is_active, content_translation_x));
    }
}

#[test]
fn incoming_layout_uses_existing_avatar_gutter_or_creates_one_without_moving_own_messages() {
    assert!((RiseManualListSelectionLayout::indicator_center_x() - 27.0).abs() <= 0.001);
    assert_eq!(RiseManualListSelectionLayout::content_translation_x(false, true), 0.0);
    assert_eq!(
        RiseManualListSelectionLayout::content_translation_x(false, false),
        MessageSizes::avatar_size() + MessageSizes::avatar_gap()
    );
    assert_eq!(RiseManualListSelectionLayout::content_translation_x(true, false), 0.0);
}

#[gpui::test]
fn selection_toggle_does_not_rebuild_items_or_move_the_viewport(cx: &mut TestAppContext) {
    let frame = Bounds::new(point(px(0.0), px(0.0)), size(px(320.0), px(120.0)));
    let (list, cx) = cx.add_window_view(|window, cx| RiseManualListView::new(window, cx));
    cx.simulate_resize(frame.size);
    cx.update(|_, app| {
        RiseManualListConformances::register_reusable_view::<RiseManualListSelectionSpyView>(app);
        RiseManualListConformances::register_selection_presentable::<RiseManualListSelectionSpyView>(app);
    });
    let with_list = |cx: &mut gpui::VisualTestContext,
                     f: &mut dyn FnMut(&mut RiseManualListView, &mut Window, &mut Context<RiseManualListView>)| {
        cx.update(|window, app| list.update(app, |list, cx| f(list, window, cx)))
    };
    with_list(cx, &mut |list, window, cx| list.layout_subviews(frame, window, cx));

    let factory_call_count = Rc::new(Cell::new(0usize));
    let views: Rc<RefCell<HashMap<String, Entity<RiseManualListSelectionSpyView>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let provider: RiseManualListItemProvider = {
        let factory_call_count = factory_call_count.clone();
        let views = views.clone();
        Rc::new(move |id: &str, _: f32, _: &mut Window, _: &mut App| {
            let id = id.to_string();
            let factory_call_count = factory_call_count.clone();
            let views = views.clone();
            RiseManualListItem::new(
                60.0,
                "selection-test".to_string(),
                1,
                RiseManualListItemMountPolicy::Immediate,
                None,
                None,
                None,
                None,
                Rc::new(move |reusable: Option<AnyView>, _: &mut Window, cx: &mut App| {
                    factory_call_count.set(factory_call_count.get() + 1);
                    let view = reusable
                        .and_then(|reusable| reusable.downcast::<RiseManualListSelectionSpyView>().ok())
                        .unwrap_or_else(|| cx.new(|_| RiseManualListSelectionSpyView::default()));
                    views.borrow_mut().insert(id.clone(), view.clone());
                    view.into()
                }),
            )
        })
    };
    let mut transaction = Some(RiseManualListTransaction::new(
        vec!["msg-a".to_string(), "msg-b".to_string()],
        RiseManualListTransactionAnchor::Top,
        None,
        None,
        None,
        None,
        None,
        false,
        HashSet::new(),
        provider,
    ));
    with_list(cx, &mut |list, window, cx| {
        if let Some(transaction) = transaction.take() {
            list.enqueue(transaction, None, None, window, cx);
        }
    });

    let before_factory_call_count = factory_call_count.get();
    let (before_height, before_offset, before_a, before_b) = list.read_with(cx, |list, _| {
        (
            list.scroll_metrics().content_height,
            list.scroll_metrics().content_offset_y,
            list.capture_viewport_anchor("msg-a").expect("msg-a anchor"),
            list.capture_viewport_anchor("msg-b").expect("msg-b anchor"),
        )
    });

    let items: HashMap<String, RiseManualListSelectionItem> = HashMap::from([
        (
            "msg-a".to_string(),
            RiseManualListSelectionItem {
                item_id: "msg-a".to_string(),
                selection_id: Some("server-a".to_string()),
                indicator_center_x: RiseManualListSelectionLayout::indicator_center_x(),
                content_translation_x: 40.0,
                accessibility_label: "Alice. First".to_string(),
            },
        ),
        (
            "msg-b".to_string(),
            RiseManualListSelectionItem {
                item_id: "msg-b".to_string(),
                selection_id: Some("server-b".to_string()),
                indicator_center_x: RiseManualListSelectionLayout::indicator_center_x(),
                content_translation_x: 0.0,
                accessibility_label: "Me. Second".to_string(),
            },
        ),
    ]);
    let presentation = |selected_ids: &[&str]| RiseManualListSelectionPresentation {
        is_active: true,
        selected_ids: selected_ids.iter().map(|id| id.to_string()).collect(),
        items_by_item_id: items.clone(),
        tint_color: UIColor::system_blue().resolved_color(Appearance::Light),
        on_toggle: None,
    };
    let mut first = Some(presentation(&["server-a"]));
    with_list(cx, &mut |list, window, cx| {
        if let Some(presentation) = first.take() {
            list.apply_selection_presentation(presentation, window, cx);
        }
    });
    let mut second = Some(presentation(&["server-a", "server-b"]));
    with_list(cx, &mut |list, window, cx| {
        if let Some(presentation) = second.take() {
            list.apply_selection_presentation(presentation, window, cx);
        }
    });

    let view = |id: &str| views.borrow().get(id).cloned().expect("mounted spy view");
    let last_presentation = |id: &str, cx: &gpui::VisualTestContext| {
        view(id).read_with(cx, |spy, _| spy.presentations.last().copied())
    };
    assert_eq!(factory_call_count.get(), before_factory_call_count);
    list.read_with(cx, |list, _| {
        assert_eq!(list.scroll_metrics().content_height, before_height);
        assert_eq!(list.scroll_metrics().content_offset_y, before_offset);
        assert_eq!(list.capture_viewport_anchor("msg-a"), Some(before_a));
        assert_eq!(list.capture_viewport_anchor("msg-b"), Some(before_b));
        assert!(!list.is_user_interaction_enabled_for_testing(&view("msg-a").into()));
        assert!(!list.is_user_interaction_enabled_for_testing(&view("msg-b").into()));
    });
    assert_eq!(last_presentation("msg-a", cx).map(|(is_active, _)| is_active), Some(true));
    assert_eq!(last_presentation("msg-a", cx).map(|(_, translation_x)| translation_x), Some(40.0));
    assert_eq!(last_presentation("msg-b", cx).map(|(_, translation_x)| translation_x), Some(0.0));

    with_list(cx, &mut |list, window, cx| {
        list.apply_selection_presentation(RiseManualListSelectionPresentation::inactive(), window, cx)
    });
    assert_eq!(factory_call_count.get(), before_factory_call_count);
    list.read_with(cx, |list, _| {
        assert!(list.is_user_interaction_enabled_for_testing(&view("msg-a").into()));
    });
    assert_eq!(last_presentation("msg-a", cx).map(|(is_active, _)| is_active), Some(false));
    assert_eq!(last_presentation("msg-a", cx).map(|(_, translation_x)| translation_x), Some(0.0));
}
