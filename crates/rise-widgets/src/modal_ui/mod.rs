mod modal_frame;
mod modal_ui;

pub use modal_frame::{ModalFrame, ModalWidth, resolve_frame};
pub use modal_ui::{
    DismissModal, KEY_CONTEXT, ModalAction, ModalDismiss, ModalUi, install_key_bindings,
};
