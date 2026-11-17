use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Global, RenderImage};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName};

pub(crate) const MEDIA_PICKER_EDITS_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("mediaPickerEditsDidChange");

/// main thread state held as an app global
#[derive(Default)]
struct EditorEditPreviewImages(HashMap<String, Arc<RenderImage>>);

impl Global for EditorEditPreviewImages {}

pub(crate) enum EditorEditPreviewCache {}

impl EditorEditPreviewCache {
    pub(crate) fn image(asset_id: &str, cx: &App) -> Option<Arc<RenderImage>> {
        cx.try_global::<EditorEditPreviewImages>()
            .and_then(|images| images.0.get(asset_id).cloned())
    }

    pub(crate) fn store(image: Option<Arc<RenderImage>>, asset_id: &str, cx: &mut App) {
        let images = &mut cx.default_global::<EditorEditPreviewImages>().0;
        match image {
            Some(image) => {
                images.insert(asset_id.to_string(), image);
            }
            None => {
                images.remove(asset_id);
            }
        }
        RiseAppEvents::default_center().post_with(MEDIA_PICKER_EDITS_DID_CHANGE, asset_id.to_string());
    }

    pub(crate) fn remove(asset_id: &str, cx: &mut App) {
        Self::store(None, asset_id, cx);
    }
}
