/// which surface takes composer input
/// the text field with the system keyboard or the emoji and sticker picker
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChatComposerInputSurface {
    #[default]
    SystemKeyboard,
    Picker,
}

impl ChatComposerInputSurface {
    /// icon key of the toggle button, it shows the surface the tap switches to
    pub fn toggle_icon_system_name(self) -> &'static str {
        match self {
            Self::SystemKeyboard => "face.smiling",
            Self::Picker => "keyboard",
        }
    }

    pub fn toggled(self) -> ChatComposerInputSurface {
        match self {
            Self::SystemKeyboard => Self::Picker,
            Self::Picker => Self::SystemKeyboard,
        }
    }
}

pub enum ChatComposerBottomSafeAreaPolicy {}

impl ChatComposerBottomSafeAreaPolicy {
    /// true means all regions ignored, false means none
    /// the picker draws under the bottom safe area while shown or handing over to the keyboard
    pub fn ignored_regions(picker_visible: bool, is_picker_to_keyboard_transitioning: bool) -> bool {
        picker_visible || is_picker_to_keyboard_transitioning
    }
}
