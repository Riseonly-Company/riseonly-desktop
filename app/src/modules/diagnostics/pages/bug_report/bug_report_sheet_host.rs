use gpui::{App, Context, Entity, IntoElement, Render, Subscription, Window};
use rise_theme::ThemeStore;
use rise_ui::swiftui::prelude::*;

pub use crate::core::desktop_platform::bug_report_surface_hooks::BugReportSurfaceProbe;
use crate::modules::diagnostics::pages::bug_report::bug_report_sheet::BugReportSheet;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportInteractionsStore;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_services::bug_report_services::BugReportServicesStore;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerResult;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;

/// mounts the report sheet and its picker one at a time
/// picker mounts after the sheet is dismissed, sheet returns after the picker
/// zero size container, doesnt eat clicks over the root
pub struct BugReportSheetHost {
    services: Entity<BugReportServicesStore>,
    interactions: Entity<BugReportInteractionsStore>,
    _observers: [Subscription; 2],
}

impl BugReportSheetHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let services = BugReportServicesStore::shared(cx);
        let interactions = BugReportInteractionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let observers = [cx.observe(&services, |_, _, cx| cx.notify()), cx.observe(&theme_store, |_, _, cx| cx.notify())];
        Self { services, interactions, _observers: observers }
    }

    fn picker(&self, cx: &mut Context<Self>) -> MediaPickerSheet {
        let draft = self.services.read(cx).draft();
        let existing_screenshots = i64::try_from(draft.screenshots.len()).unwrap_or(i64::MAX);
        let (read_services, write_interactions) = (self.services.clone(), self.interactions.clone());
        let is_presented = Binding::new(
            move |cx: &App| read_services.read(cx).is_picker_presented(),
            move |presented: bool, _: &mut Window, cx: &mut App| {
                write_interactions.update(cx, |interactions, cx| interactions.set_picker_presented(presented, cx));
            },
        );
        let (complete_interactions, dismissed_interactions) = (self.interactions.clone(), self.interactions.clone());
        MediaPickerSheet::new(
            "bug-report-picker",
            is_presented,
            MediaPickerConfiguration::bug_report_appending(existing_screenshots, draft.video.is_some()),
            ThemeStore::shared(cx),
            move |result: MediaPickerResult, _: &mut Window, cx: &mut App| {
                let interactions = complete_interactions.clone();
                cx.defer(move |cx| {
                    interactions.update(cx, |interactions, cx| interactions.attach(result, cx));
                });
            },
        )
        .experimental_bottom_sheet(true)
        .on_dismissed(move |_: &mut Window, cx: &mut App| {
            dismissed_interactions.update(cx, |interactions, cx| interactions.picker_did_dismiss(cx));
        })
    }
}

impl Render for BugReportSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (mounts_sheet, mounts_picker) = {
            let services = self.services.read(cx);
            (services.mounts_sheet(), services.mounts_picker())
        };
        let mut container = div().w(px(0.0)).h(px(0.0));
        if mounts_sheet {
            container = container.child(BugReportSheet::new(cx));
        }
        if mounts_picker {
            container = container.child(self.picker(cx));
        }
        container
    }
}
