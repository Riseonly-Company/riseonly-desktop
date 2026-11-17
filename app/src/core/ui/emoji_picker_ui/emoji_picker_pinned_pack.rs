use std::rc::Rc;

use gpui::{App, Window};

/// a pack the picker draws itself and pins to the bottom of the grid
/// some selectable options are not sticker items
/// the built in premium diamond is a bundled lottie with no ce id
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EmojiPickerPinnedItem {
    pub id: String,
    pub lottie_name: String,
}

#[derive(Clone)]
pub struct EmojiPickerPinnedPack {
    pub id: String,
    pub title: String,
    pub symbol: String,
    pub items: Vec<EmojiPickerPinnedItem>,
    pub on_pick: Rc<dyn Fn(EmojiPickerPinnedItem, &mut Window, &mut App)>,
}
