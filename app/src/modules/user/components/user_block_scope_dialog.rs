use std::time::Duration;

use gpui::{AnyElement, App, ElementId, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::native_confirm_alert::{NativeConfirmAlert, NativeConfirmAlertAction};
use rise_ui::swiftui::prelude::*;

use crate::core::widgets::sheets::report_sheet::{ReportSheetStore, ReportSubject};
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

#[derive(IntoElement)]
pub struct UserBlockScopeDialog {
    id: ElementId,
    target_user_id: Binding<Option<String>>,
    content: AnyElement,
}

impl UserBlockScopeDialog {
    fn present(target: String, cx: &mut App) {
        let message = format!(
            "{}: {}\n{}: {}",
            tr("contact_block_scope_standard"),
            tr("contact_block_scope_standard_desc"),
            tr("contact_block_scope_total"),
            tr("contact_block_scope_total_desc"),
        );
        let standard_target = target.clone();
        let total_target = target.clone();
        let on_standard: NativeConfirmAlertAction =
            Box::new(move |cx: &mut App| Self::block(&standard_target, RiseBlockScope::Standard, cx));
        let on_total: NativeConfirmAlertAction =
            Box::new(move |cx: &mut App| Self::block(&total_target, RiseBlockScope::Total, cx));
        let on_block_and_report: NativeConfirmAlertAction =
            Box::new(move |cx: &mut App| UserBlockAndReport::run(&target, None, None, cx));
        NativeConfirmAlert::present_destructive_choices(
            &tr("contact_block_confirmation_title"),
            &message,
            vec![
                (tr("contact_block_scope_standard"), on_standard),
                (tr("contact_block_scope_total"), on_total),
                (tr("user_block_and_report"), on_block_and_report),
            ],
            None,
            cx,
        );
    }

    fn block(target: &str, scope: RiseBlockScope, cx: &mut App) {
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.set_user_block_scope(target, Some(scope), |_, _| {}, cx);
        });
    }
}

impl RenderOnce for UserBlockScopeDialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let target_user_id = self.target_user_id.clone();
        let target = self.target_user_id.get(cx);
        on_change(self.id, target, false, window, cx, move |_, target, window, cx| {
            let Some(target) = target.clone().filter(|target| !target.is_empty()) else {
                return;
            };
            target_user_id.set(None, window, cx);
            Self::present(target, cx);
        });
        self.content
    }
}

pub trait UserBlockScopeDialogModifiers: IntoElement + Sized {
    fn user_block_scope_dialog(
        self,
        id: impl Into<ElementId>,
        target_user_id: Binding<Option<String>>,
    ) -> UserBlockScopeDialog {
        UserBlockScopeDialog { id: id.into(), target_user_id, content: self.into_any_element() }
    }
}

impl<E: IntoElement> UserBlockScopeDialogModifiers for E {}

/// block and report, blocks then opens the report sheet
/// sheet waits for the block to land
/// failed block shows an error, no sheet
pub enum UserBlockAndReport {}

impl UserBlockAndReport {
    const SHEET_DELAY: Duration = Duration::from_millis(350);

    /// scope is the tier picked,
    /// none keeps an existing blocks tier or places a standard block so a full block never gets
    /// lowered
    pub fn run(target_user_id: &str, scope: Option<RiseBlockScope>, subject: Option<ReportSubject>, cx: &mut App) {
        let target = trimming_whitespaces_and_newlines(target_user_id).to_owned();
        if target.is_empty() {
            return;
        }
        let current = UserServicesStore::shared(cx).update(cx, |services, cx| services.viewer_block_scope(&target, cx));
        let wanted = scope.or(current).unwrap_or(RiseBlockScope::Standard);
        if current == Some(wanted) {
            Self::present_report(target, subject, cx);
            return;
        }
        let report_target = target.clone();
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.set_user_block_scope(
                &target,
                Some(wanted),
                move |result, cx| {
                    if result.is_err() {
                        return;
                    }
                    Self::present_report(report_target.clone(), subject.clone(), cx);
                },
                cx,
            );
        });
    }

    fn present_report(target: String, subject: Option<ReportSubject>, cx: &mut App) {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Self::SHEET_DELAY).await;
            cx.update(|cx| {
                ReportSheetStore::shared(cx).update(cx, |store, cx| {
                    store.present(ReportTarget::User(target), subject, None, cx);
                });
            });
        })
        .detach();
    }
}
