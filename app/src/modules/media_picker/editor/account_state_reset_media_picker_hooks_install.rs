use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetMediaPickerHooks, set_account_state_reset_media_picker_hooks,
};
use crate::modules::media_picker::api::media_picker_coordinator::MediaPickerCoordinator;
use crate::modules::media_picker::editor::editor_adjustments_renderer::EditorAdjustmentsRenderer;
use crate::modules::media_picker::editor::editor_color_picker_bridge::EditorColorPickerBridge;
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorWindowPresenter;
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;
use crate::modules::media_picker::shared::media_picker_recent_files::MediaPickerRecentFiles;

pub fn install_account_state_reset_media_picker_hooks() {
    set_account_state_reset_media_picker_hooks(AccountStateResetMediaPickerHooks {
        activate_account_state: |cx| {
            MediaPickerRecentFiles::shared(cx).update(cx, |files, cx| files.activate_account_state(cx));
            MediaReeditStore::shared(cx).update(cx, |store, _| store.activate_account_state());
        },
        reset_account_state: |cx| {
            MediaPickerCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.reset_account_state(cx));
            MediaEditorWindowPresenter::shared(cx).update(cx, |presenter, cx| presenter.reset_account_state(cx));
            EditorColorPickerBridge::shared(cx).update(cx, |bridge, cx| bridge.reset_account_state(cx));
            EditorAdjustmentsRenderer::shared().reset_account_state();
            MediaPickerRecentFiles::shared(cx).update(cx, |files, cx| files.reset_account_state(cx));
            MediaReeditStore::shared(cx).update(cx, |store, _| store.reset_account_state());
        },
        remove_persisted_account: |user_id, cx| {
            MediaPickerRecentFiles::shared(cx).update(cx, |files, _| files.remove_persisted_account(user_id));
            MediaReeditStore::shared(cx).update(cx, |store, _| store.remove_persisted_account(user_id));
        },
    });
}
