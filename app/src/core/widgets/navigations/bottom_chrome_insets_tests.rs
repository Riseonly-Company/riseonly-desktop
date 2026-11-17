use crate::core::widgets::navigations::bottom_chrome_insets::{
    BottomChromeCandidate, BottomChromeOwner, BottomChromePolicy, BottomChromeSafeArea,
};

fn candidate(owner: BottomChromeOwner, top: f32) -> BottomChromeCandidate {
    BottomChromeCandidate {
        owner,
        top_edge_in_window: top,
    }
}

#[test]
fn no_chrome_means_no_clearance() {
    assert_eq!(
        BottomChromePolicy::clearance(818.0, 818.0, None, BottomChromePolicy::NOTIFIER_GAP),
        0.0
    );
}

#[test]
fn clearance_lifts_content_above_the_measured_chrome_top() {
    assert_eq!(BottomChromePolicy::clearance(818.0, 818.0, Some(740.0), 10.0), 88.0);
}

#[test]
fn chrome_below_the_host_bottom_never_pulls_content_down() {
    assert_eq!(BottomChromePolicy::clearance(700.0, 700.0, Some(740.0), 0.0), 0.0);
}

#[test]
fn floating_action_sits_one_gap_above_an_already_inset_container() {
    assert_eq!(BottomChromePolicy::floating_bottom_padding(818.0, 818.0, None, 16.0), 16.0);
}

#[test]
fn floating_action_rides_on_the_chrome_when_there_is_chrome() {
    assert_eq!(BottomChromePolicy::floating_bottom_padding(818.0, 818.0, Some(740.0), 16.0), 94.0);
}

#[test]
fn nothing_in_the_window_means_no_chrome() {
    assert_eq!(BottomChromePolicy::top_edge_in_window(&[], None, &[]), None);
}

#[test]
fn floating_action_raises_the_notifier_but_not_itself() {
    let candidates = [
        candidate(BottomChromeOwner::PushedMiniPlayer, 760.0),
        candidate(BottomChromeOwner::FloatingAction, 700.0),
    ];

    assert_eq!(BottomChromePolicy::top_edge_in_window(&candidates, None, &[]), Some(700.0));
    assert_eq!(
        BottomChromePolicy::top_edge_in_window(&candidates, None, &[BottomChromeOwner::FloatingAction]),
        Some(760.0)
    );
}

#[test]
fn the_real_tab_bar_supersedes_its_reserve_estimate() {
    let candidates = [candidate(BottomChromeOwner::MainTabBar, 723.0)];

    assert_eq!(BottomChromePolicy::top_edge_in_window(&candidates, Some(754.0), &[]), Some(754.0));
    assert_eq!(BottomChromePolicy::top_edge_in_window(&candidates, None, &[]), Some(723.0));

    let with_mini_player = [
        candidate(BottomChromeOwner::MainTabBar, 723.0),
        candidate(BottomChromeOwner::TabBarMiniPlayer, 670.0),
    ];
    assert_eq!(
        BottomChromePolicy::top_edge_in_window(&with_mini_player, Some(754.0), &[]),
        Some(670.0)
    );
}

#[test]
fn excluding_the_tab_bar_excludes_the_real_bar_too() {
    assert_eq!(
        BottomChromePolicy::top_edge_in_window(
            &[candidate(BottomChromeOwner::MainTabBar, 723.0)],
            Some(754.0),
            &[BottomChromeOwner::MainTabBar],
        ),
        None
    );
}

#[test]
fn real_chrome_stack_stays_below_the_ceiling() {
    let clearance =
        BottomChromePolicy::clearance(818.0, 818.0, Some(818.0 - 179.0), BottomChromePolicy::NOTIFIER_GAP);

    assert_eq!(clearance, 189.0);
    assert!(clearance < BottomChromePolicy::max_clearance(818.0));
}

#[test]
fn a_chrome_top_from_the_middle_of_the_screen_is_clamped() {
    let raw = 818.0 - 371.0 + BottomChromePolicy::NOTIFIER_GAP;
    let clearance = BottomChromePolicy::clearance(818.0, 818.0, Some(371.0), BottomChromePolicy::NOTIFIER_GAP);

    assert_eq!(raw, 457.0);
    assert_eq!(clearance, BottomChromePolicy::max_clearance(818.0));
    assert!(clearance < raw);
}

#[test]
fn keyboard_height_is_not_a_container_inset() {
    assert_eq!(BottomChromeSafeArea::container_bottom_inset(34.0), 34.0);
    assert_eq!(BottomChromeSafeArea::container_bottom_inset(0.0), 0.0);
    assert!(BottomChromeSafeArea::container_bottom_inset(336.0) <= 60.0);
}
