use gpui::{App, IntoElement, Window, div};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_moderation::PlatformRole;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationDetent,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::swiftui::binding::Binding;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::moderation::pages::moderation_case_sheets::AdminSanctionSheet;
use crate::modules::moderation::stores::moderation_actions::moderation_types::AdminSanctionKind;
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;

/// platform admins ban on a foreign profile and a direct chat profile
/// row is hidden for everybody else, report-service checks the server role before the sanction lands
pub enum AdminBanEntry {}

impl AdminBanEntry {
    pub const ACTION_ID: &'static str = "admin_ban";

    /// not the viewer and not another admin, the server refuses both so the row never offers them
    pub fn is_available(target_user_id: &str, target_role: Option<&str>, cx: &mut App) -> bool {
        let id = trimming_whitespaces_and_newlines(target_user_id);
        if id.is_empty() || !ModerationServicesStore::is_admin(cx) {
            return false;
        }
        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        if viewer.as_deref() == Some(id) {
            return false;
        }
        !Self::is_admin_role(target_role)
    }

    pub fn title() -> String {
        tr("admin_ban_action")
    }

    pub fn action() -> HoldContextMenuAction {
        HoldContextMenuAction::new(Self::ACTION_ID, Self::title()).system_image("nosign").destructive(true)
    }

    fn is_admin_role(role: Option<&str>) -> bool {
        let Some(role) = role.filter(|role| !role.is_empty()) else {
            return false;
        };
        let wire = trimming_whitespaces_and_newlines(role).to_uppercase();
        PlatformRole::ALL.into_iter().find(|candidate| candidate.wire() == wire) == Some(PlatformRole::Admin)
    }
}

/// account block sheet of the case pages, no report or appeal behind it
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AdminBanTarget {
    pub user_id: String,
    pub name: Option<String>,
}

impl AdminBanTarget {
    pub fn id(&self) -> &str {
        &self.user_id
    }
}

/// sheet is up while target holds a value, closing it clears it
pub fn admin_ban_sheet(
    id: impl Into<gpui::ElementId>,
    target: Binding<Option<AdminBanTarget>>,
) -> NativeBottomSheetPresentationHost {
    let reader = target.clone();
    let writer = target.clone();
    NativeBottomSheetPresentationHost::new(
        id,
        Binding::new(
            move |cx: &App| reader.get(cx).is_some(),
            move |presented, window, cx| {
                if !presented {
                    writer.set(None, window, cx);
                }
            },
        ),
        move |_: &mut Window, cx: &mut App| match target.get(cx) {
            Some(value) => {
                let done = target.clone();
                AdminSanctionSheet::new(
                    AdminSanctionKind::Block,
                    Some(value.user_id),
                    value.name,
                    None,
                    None,
                    move |window, cx| done.set(None, window, cx),
                )
                .into_any_element()
            }
            None => div().into_any_element(),
        },
    )
    .presentation_detents([PresentationDetent::Large])
    .presentation_drag_indicator(true)
}
