#![cfg(debug_assertions)]

use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use gpui::App;
use rise_canon::{RiseGuideAnchor, RiseGuideCanon, RiseGuideScreen};
use rise_ui::guide_anchor_ui::guide_anchor_registry::GuideAnchorRegistry;

use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use crate::core::guides::guide_screen_resolver::{GuideScreenObservation, GuideScreenObserver, GuideScreenResolver};

thread_local! {
    static OBSERVATION: RefCell<Option<GuideScreenObservation>> = const { RefCell::new(None) };
    static AUDITED_SCREENS: RefCell<HashSet<RiseGuideScreen>> = RefCell::new(HashSet::new());
}

pub enum GuideAnchorInventory {}

impl GuideAnchorInventory {
    /// anchors never mounted since launch
    /// a run result and not a build result, an anchor on a screen this session never opened is listed too
    /// read it after a pass over the app
    pub fn unregistered() -> Vec<RiseGuideAnchor> {
        let ever = GuideAnchorRegistry::ever_registered();
        RiseGuideAnchor::ALL.into_iter().filter(|anchor| !ever.contains(anchor)).collect()
    }

    /// anchors the canon declares for screen
    pub fn anchors(screen: RiseGuideScreen) -> Vec<RiseGuideAnchor> {
        RiseGuideAnchor::ALL.into_iter().filter(|anchor| anchor.screen() == screen.raw_value()).collect()
    }

    pub fn missing_on_active_screen(cx: &mut App) -> Vec<RiseGuideAnchor> {
        let Some(screen) = GuideScreenResolver::current_active_screen(cx) else {
            return Vec::new();
        };
        Self::anchors(screen).into_iter().filter(|anchor| !GuideAnchorRegistry::is_registered(*anchor)).collect()
    }

    #[track_caller]
    pub fn assert_complete() {
        let missing = Self::unregistered();
        debug_assert!(missing.is_empty(), "guide anchors never registered this run: {}", names(&missing));
    }

    pub fn install_screen_audit() {
        if OBSERVATION.with(|observation| observation.borrow().is_some()) {
            return;
        }
        let observation = GuideScreenObserver::observe(|_, cx| {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(RiseGuideCanon::TIMINGS_ANCHOR_REVEAL_MS.unsigned_abs()))
                    .await;
                cx.update(|cx| Self::audit_active_screen(cx));
            })
            .detach();
        });
        OBSERVATION.with(|slot| *slot.borrow_mut() = Some(observation));
    }

    pub fn summary() -> String {
        let absent = Self::unregistered();
        let total = RiseGuideAnchor::ALL.len();
        let seen = total - absent.len();
        let never = if absent.is_empty() { "none".to_owned() } else { names(&absent) };
        format!("guide anchors: {seen}/{total} registered this run\nnever registered: {never}")
    }

    fn audit_active_screen(cx: &mut App) {
        let Some(screen) = GuideScreenResolver::current_active_screen(cx) else { return };
        if AUDITED_SCREENS.with(|audited| audited.borrow().contains(&screen)) {
            return;
        }
        let declared = Self::anchors(screen);
        if declared.is_empty() {
            return;
        }
        AUDITED_SCREENS.with(|audited| {
            audited.borrow_mut().insert(screen);
        });

        let absent: Vec<RiseGuideAnchor> =
            declared.into_iter().filter(|anchor| !GuideAnchorRegistry::is_registered(*anchor)).collect();
        if absent.is_empty() {
            return;
        }
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Screen, || {
            format!("guide anchors absent on {}: {}", screen.raw_value(), names(&absent))
        });
    }
}

fn names(anchors: &[RiseGuideAnchor]) -> String {
    anchors.iter().map(|anchor| anchor.raw_value()).collect::<Vec<_>>().join(", ")
}
