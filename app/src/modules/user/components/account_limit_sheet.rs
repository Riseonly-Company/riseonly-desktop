use std::rc::Rc;

use gpui::{App, SharedString, Window};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::core::widgets::overlay::premium_upsell_host::PremiumUpsellSheet;
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellPrompt;

const SHEET_ID: &str = "account-limit-sheet";

struct AccountLimitSheetState {
    prompt_id: Uuid,
}

#[derive(IntoElement)]
pub struct AccountLimitSheet {
    is_presented: Binding<bool>,
    is_premium: bool,
    on_upgrade: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl AccountLimitSheet {
    pub fn new(
        is_presented: Binding<bool>,
        is_premium: bool,
        on_upgrade: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { is_presented, is_premium, on_upgrade: Rc::new(on_upgrade) }
    }
}

impl RenderOnce for AccountLimitSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("account-limit-sheet-state", cx, |_, _| AccountLimitSheetState {
            prompt_id: Uuid::new_v4(),
        });
        let mut prompt = PremiumUpsellPrompt::new(
            state.read(cx).prompt_id.clone(),
            tr(if self.is_premium { "account_limit_premium_message" } else { "account_limit_standard_message" }),
        );
        prompt.can_upgrade = !self.is_premium;

        let dismiss_presented = self.is_presented.clone();
        let upgrade_presented = self.is_presented.clone();
        let on_upgrade = self.on_upgrade;
        PremiumUpsellSheet::new(
            SHEET_ID,
            self.is_presented,
            Some(prompt),
            move |window, cx| dismiss_presented.set(false, window, cx),
            move |window, cx| {
                upgrade_presented.set(false, window, cx);
                on_upgrade(window, cx);
            },
        )
        .title(Some(SharedString::from(tr("account_limit_title"))))
        .shows_explanation(false)
    }
}
