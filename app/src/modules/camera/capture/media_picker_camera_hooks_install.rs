use crate::modules::camera::capture::rise_camera_presenter::RiseCameraPresenter;
use crate::modules::media_picker::ui::media_picker_camera_hooks::{
    MediaPickerCameraHooks, set_media_picker_camera_hooks,
};

pub fn install_media_picker_camera_hooks() {
    set_media_picker_camera_hooks(MediaPickerCameraHooks {
        present_for_capture: RiseCameraPresenter::present_for_capture,
    });
}
