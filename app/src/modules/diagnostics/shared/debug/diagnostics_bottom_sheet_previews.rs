#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, AsyncApp, Context, IntoElement, Render, Styled, Subscription, Window, div, px};
use rise_canon::RiseBugReportSurface;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::lifecycle::on_change;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationMode, BottomSheetDebugRegistrationModule,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportInteractionsStore;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_services::bug_report_services::BugReportServicesStore;

pub enum DiagnosticsBottomSheetPreviews {}

impl DiagnosticsBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        let app: Rc<RefCell<Option<AsyncApp>>> = Rc::new(RefCell::new(None));
        let content_app = app.clone();
        vec![
            BottomSheetDebugRegistration::new(
                "diagnostics.report",
                BottomSheetDebugRegistrationModule::Diagnostics,
                "report_riseonly_title",
                "BugReportSheet",
                move |open: Binding<bool>, window: &mut Window, cx: &mut App| {
                    *content_app.borrow_mut() = Some(cx.to_async());
                    window.use_keyed_state("diagnostics.report.launcher", cx, move |_, cx| {
                        DebugBugReportLauncher::new(open, cx)
                    })
                },
            )
            .with_mode(BottomSheetDebugRegistrationMode::Live)
            .on_cancel(move || {
                let Some(app) = app.borrow().clone() else {
                    return;
                };
                app.spawn(async move |cx| {
                    cx.update(|cx| {
                        BugReportInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.set_sheet_presented(false, cx));
                    });
                })
                .detach();
            }),
        ]
    }
}

struct DebugBugReportLauncher {
    is_presented: Binding<bool>,
    _services_observer: Subscription,
}

impl DebugBugReportLauncher {
    fn new(is_presented: Binding<bool>, cx: &mut Context<Self>) -> Self {
        let services = BugReportServicesStore::shared(cx);
        let services_observer = cx.observe(&services, |_, _, cx| cx.notify());
        Self { is_presented, _services_observer: services_observer }
    }
}

impl Render for DebugBugReportLauncher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        on_change(
            "diagnostics.report.launcher.open",
            self.is_presented.get(cx),
            true,
            window,
            cx,
            |_, open: &bool, _, cx| {
                if *open {
                    BugReportInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.present(RiseBugReportSurface::Other, cx));
                }
            },
        );
        let services = BugReportServicesStore::shared(cx);
        let sheet_presented = services.read(cx).is_sheet_presented();
        let is_presented = self.is_presented.clone();
        on_change(
            "diagnostics.report.launcher.sheet",
            sheet_presented,
            false,
            window,
            cx,
            move |_, open: &bool, window, cx| {
                if !*open {
                    is_presented.set(false, window, cx);
                }
            },
        );
        div().w(px(0.0)).h(px(0.0))
    }
}
