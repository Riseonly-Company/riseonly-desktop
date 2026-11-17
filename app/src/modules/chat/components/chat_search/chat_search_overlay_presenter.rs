use gpui::{App, Context, Entity};

pub struct ChatSearchOverlayPresenter {
    is_open: bool,
}

impl ChatSearchOverlayPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_open: false }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn set_is_open(&mut self, is_open: bool, cx: &mut Context<Self>) {
        self.is_open = is_open;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.set_is_open(false, cx);
    }
}
