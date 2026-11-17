use gpui::{App, Context, Entity};

use crate::core::ui::purchase_celebration::{PurchaseCelebration, PurchaseCelebrationDirector};

pub struct SubscriptionSuccessStore {
    is_visible: bool,
}

impl SubscriptionSuccessStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_visible: false }
    }

    pub fn is_visible(&self) -> bool {
        self.is_visible
    }

    /// opens the success sheet with the premium celebration, it plays its own haptics
    pub fn show(&mut self, cx: &mut Context<Self>) {
        if self.is_visible {
            return;
        }
        PurchaseCelebration::Premium.prepare(cx);
        PurchaseCelebrationDirector::premium(cx).update(cx, |director, cx| director.arm(cx));
        self.is_visible = true;
        cx.notify();
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.is_visible = false;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.hide(cx);
    }
}
