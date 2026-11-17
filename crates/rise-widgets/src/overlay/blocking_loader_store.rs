use gpui::{App, Context, Entity};

pub type BlockingLoaderCancel = Box<dyn FnOnce(&mut App)>;

pub struct BlockingLoaderStore {
    is_visible: bool,
    on_cancel: Option<BlockingLoaderCancel>,
}

impl BlockingLoaderStore {
    pub const APPEARANCE_DELAY: f64 = 1.0;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_visible: false, on_cancel: None }
    }

    pub fn is_visible(&self) -> bool {
        self.is_visible
    }

    pub fn show(&mut self, on_cancel: impl FnOnce(&mut App) + 'static, cx: &mut Context<Self>) {
        self.on_cancel = Some(Box::new(on_cancel));
        self.is_visible = true;
        cx.notify();
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.is_visible = false;
        self.on_cancel = None;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.hide(cx);
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        let callback = self.on_cancel.take();
        self.is_visible = false;
        cx.notify();
        if let Some(callback) = callback {
            cx.defer(callback);
        }
    }
}
