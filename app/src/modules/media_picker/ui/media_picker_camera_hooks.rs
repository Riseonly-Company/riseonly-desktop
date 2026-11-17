use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::camera::capture::rise_camera_capture::RiseCameraShot;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;

pub type MediaPickerCameraCaptureSink = Box<dyn FnOnce(RiseCameraShot, &mut App)>;

pub struct MediaPickerCameraHooks {
    pub present_for_capture: fn(MediaPickerConfiguration, MediaPickerCameraCaptureSink, &mut App),
}

pub(crate) static MEDIA_PICKER_CAMERA_HOOKS: FeatureHookSlot<MediaPickerCameraHooks> =
    FeatureHookSlot::new("MEDIA_PICKER_CAMERA_HOOKS");

pub fn set_media_picker_camera_hooks(hooks: MediaPickerCameraHooks) {
    let _ = MEDIA_PICKER_CAMERA_HOOKS.set(hooks);
}

pub fn media_picker_camera_hooks() -> Option<&'static MediaPickerCameraHooks> {
    MEDIA_PICKER_CAMERA_HOOKS.get()
}
