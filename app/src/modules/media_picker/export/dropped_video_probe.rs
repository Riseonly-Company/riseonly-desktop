use std::path::PathBuf;

use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::video_export::{VideoProbe, probe};

use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerEditedVideo, MediaPickerItem, MediaPickerItemKind,
};

pub struct DroppedVideoProbe;

impl DroppedVideoProbe {
    /// a video that cant be probed stays without a duration
    /// with no encoder in the build the servers refusal is the working answer
    pub async fn probing_durations(mut items: Vec<MediaPickerItem>) -> Vec<MediaPickerItem> {
        let sources: Vec<(usize, PathBuf)> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind == MediaPickerItemKind::Video && item.video_payload.is_none())
            .filter_map(|(index, item)| Some((index, item.url.to_file_path()?)))
            .collect();
        if sources.is_empty() {
            return items;
        }
        let probed = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                sources
                    .into_iter()
                    .map(|(index, source)| {
                        (index, probe(&source).ok().as_ref().and_then(Self::readable_seconds))
                    })
                    .collect::<Vec<_>>()
            })
            .await;
        let Ok(probed) = probed else {
            return items;
        };
        for (index, seconds) in probed {
            if let (Some(item), Some(seconds)) = (items.get_mut(index), seconds) {
                Self::apply_duration(item, seconds);
            }
        }
        items
    }

    /// duration_ms is 0 on an unreadable length
    /// reading that as a short clip would let any video past the ceiling
    pub(crate) fn readable_seconds(video: &VideoProbe) -> Option<f64> {
        (video.duration_ms() > 0).then_some(video.duration_seconds)
    }

    /// untrimmed payload duration_ms comes from, the whole file
    /// so it doesnt count as trimmed to the ceiling
    pub(crate) fn apply_duration(item: &mut MediaPickerItem, seconds: f64) {
        item.video_payload = Some(MediaPickerEditedVideo::new(
            item.url.clone(),
            0.0,
            seconds,
            seconds,
            false,
            String::new(),
        ));
    }
}
