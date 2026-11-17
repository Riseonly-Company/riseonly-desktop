use std::cell::Cell;

thread_local! {
    pub static GIFT_SHOP_FOOTER_WINDOW_LIFT: Cell<Option<f32>> = const { Cell::new(None) };
}

/// how far the top of the gift shop footer sits above the window bottom, none before its first layout
pub fn gift_shop_footer_window_lift() -> Option<f32> {
    GIFT_SHOP_FOOTER_WINDOW_LIFT.with(Cell::get)
}
