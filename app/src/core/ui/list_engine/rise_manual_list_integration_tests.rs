use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{AnyView, App, AppContext, Bounds, Empty, Pixels, TestAppContext, Window, point, px, size};

use super::rise_manual_list_integration::RiseManualListInstallCost;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListItemViewFactory, RiseManualListTransaction,
    RiseManualListTransactionAnchor,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::{
    RiseManualListMountEnvelopePolicy, RiseManualListView,
};

const HOST_PREFIX: &str = RiseManualListMountEnvelopePolicy::FALLBACK_HOSTED_REUSE_IDENTIFIER_PREFIX;

fn reusable_or_empty() -> RiseManualListItemViewFactory {
    Rc::new(|reusable: Option<AnyView>, _: &mut Window, cx: &mut App| {
        reusable.unwrap_or_else(|| cx.new(|_| Empty).into())
    })
}

fn item(reuse_identifier: &str, install_cost: Option<RiseManualListInstallCost>) -> RiseManualListItem {
    RiseManualListItem::new(
        60.0,
        reuse_identifier.to_string(),
        1,
        RiseManualListItemMountPolicy::Immediate,
        None,
        None,
        None,
        install_cost,
        reusable_or_empty(),
    )
}

#[test]
fn chat_reuse_identifiers_keep_the_string_rule() {
    assert_eq!(item(HOST_PREFIX, None).install_cost, RiseManualListInstallCost::HostedFallback);
    assert_eq!(
        item(&format!("{HOST_PREFIX}.message.photo"), None).install_cost,
        RiseManualListInstallCost::HostedFallback
    );
    assert_eq!(
        item(&format!("{HOST_PREFIX}.date"), None).install_cost,
        RiseManualListInstallCost::HostedFallback
    );
    assert_eq!(item("RiseNativeTextMessageCell", None).install_cost, RiseManualListInstallCost::Native);
    assert_eq!(item(&format!("{HOST_PREFIX}Legacy"), None).install_cost, RiseManualListInstallCost::Native);
    let defaulted = RiseManualListItem::new(
        44.0,
        "default".to_string(),
        0,
        RiseManualListItemMountPolicy::Immediate,
        None,
        None,
        None,
        None,
        reusable_or_empty(),
    );
    assert_eq!(defaulted.install_cost, RiseManualListInstallCost::Native);
}

#[test]
fn default_cost_matches_the_envelope_policy() {
    for identifier in [
        "RiseManualMessageHostingView",
        "RiseManualMessageHostingView.system",
        "RiseNativeSystemRowView",
        "post.hosted",
        "default",
    ] {
        let expected = if RiseManualListMountEnvelopePolicy::is_fallback_hosted_row(identifier) {
            RiseManualListInstallCost::HostedFallback
        } else {
            RiseManualListInstallCost::Native
        };
        assert_eq!(RiseManualListInstallCost::default_cost(identifier), expected);
    }
}

#[test]
fn stated_cost_overrides_the_string_rule() {
    assert_eq!(item("post.hosted", None).install_cost, RiseManualListInstallCost::Native);
    assert_eq!(
        item("post.hosted", Some(RiseManualListInstallCost::HostedFallback)).install_cost,
        RiseManualListInstallCost::HostedFallback
    );
    assert_eq!(
        item("RiseManualMessageHostingView.message.text", Some(RiseManualListInstallCost::Native)).install_cost,
        RiseManualListInstallCost::Native
    );
}

#[gpui::test]
fn stated_hosted_cost_installs_one_row_per_deferred_batch(cx: &mut TestAppContext) {
    let frame: Bounds<Pixels> = Bounds::new(point(px(0.0), px(0.0)), size(px(320.0), px(180.0)));
    let (list, cx) = cx.add_window_view(|window, cx| RiseManualListView::new(window, cx));
    cx.simulate_resize(frame.size);
    cx.update(|window, app| {
        list.update(app, |list, cx| {
            list.set_preload_distance(px(600.0), window, cx);
            list.layout_subviews(frame, window, cx);
        })
    });

    let ids: Vec<String> = (0..30).map(|index| format!("post-hosted-{index}")).collect();
    let factory_calls = Rc::new(Cell::new(0usize));
    let provider_calls = factory_calls.clone();
    cx.update(|window, app| {
        list.update(app, |list, cx| {
            list.enqueue(
                RiseManualListTransaction::new(
                    ids,
                    RiseManualListTransactionAnchor::Top,
                    None,
                    None,
                    None,
                    None,
                    None,
                    false,
                    HashSet::new(),
                    Rc::new(move |_: &str, _: f32, _: &mut Window, _: &mut App| {
                        let factory_calls = provider_calls.clone();
                        RiseManualListItem::new(
                            60.0,
                            "post.hosted".to_string(),
                            1,
                            RiseManualListItemMountPolicy::DeferredDuringFastScroll,
                            None,
                            None,
                            None,
                            Some(RiseManualListInstallCost::HostedFallback),
                            Rc::new(move |reusable: Option<AnyView>, _: &mut Window, cx: &mut App| {
                                factory_calls.set(factory_calls.get() + 1);
                                reusable.unwrap_or_else(|| cx.new(|_| Empty).into())
                            }),
                        )
                    }),
                ),
                None,
                None,
                window,
                cx,
            );
        })
    });

    list.read_with(cx, |list, _| {
        assert!(list.is_item_warm_mounted_for_testing("post-hosted-0"));
        assert!(list.is_item_placeholder_for_testing("post-hosted-5"));
    });

    for _ in 0..2 {
        let generation = list
            .read_with(cx, |list, _| list.scheduled_deferred_mount_generation_for_testing())
            .expect("a deferred mount batch is scheduled");
        let calls_before_batch = factory_calls.get();
        cx.update(|window, app| {
            list.update(app, |list, cx| {
                list.perform_deferred_mount_batch_for_testing(generation, None, window, cx);
            })
        });
        assert_eq!(factory_calls.get() - calls_before_batch, 1);
    }
}
