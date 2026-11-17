use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Global};
use rise_i18n::tr;

use crate::core::widgets::notifier::notifier_upload_progress::NotifierUploadProgress;

/// static banner and download maps of the upload notifier
#[derive(Default)]
struct MusicTrackUploadNotifierState {
    banners: HashMap<String, Rc<NotifierUploadProgress>>,
    downloads: HashSet<String>,
}

impl Global for MusicTrackUploadNotifierState {}

pub enum MusicTrackUploadNotifier {}

impl MusicTrackUploadNotifier {
    const DOWNLOAD_SHARE: f64 = 0.35;

    /// a retry after the network came back keeps the banner and its percent, no step back
    pub fn begin(temp_id: &str, cx: &mut App) {
        if temp_id.is_empty() {
            return;
        }
        let state = cx.default_global::<MusicTrackUploadNotifierState>();
        if state.banners.get(temp_id).is_some_and(|banner| banner.is_open()) {
            return;
        }
        state.downloads.remove(temp_id);
        let banner = Rc::new(NotifierUploadProgress::new(tr("music_track_upload_background"), None, None, cx));
        cx.default_global::<MusicTrackUploadNotifierState>()
            .banners
            .insert(temp_id.to_owned(), banner);
    }

    pub fn download(temp_id: &str, completed: i64, total: i64, cx: &mut App) {
        let state = cx.default_global::<MusicTrackUploadNotifierState>();
        let Some(banner) = state.banners.get(temp_id).cloned() else {
            return;
        };
        if total <= 0 {
            return;
        }
        state.downloads.insert(temp_id.to_owned());
        banner.update(completed as f64 / total as f64 * Self::DOWNLOAD_SHARE, cx);
    }

    pub fn upload(temp_id: &str, completed: i64, total: i64, cx: &mut App) {
        let state = cx.default_global::<MusicTrackUploadNotifierState>();
        let Some(banner) = state.banners.get(temp_id).cloned() else {
            return;
        };
        if total <= 0 {
            return;
        }
        let fraction = completed as f64 / total as f64;
        let value = if state.downloads.contains(temp_id) {
            Self::DOWNLOAD_SHARE + fraction * (1.0 - Self::DOWNLOAD_SHARE)
        } else {
            fraction
        };
        banner.update(value, cx);
    }

    pub fn succeed(temp_id: &str, title: impl Into<String>, content: Option<String>, cx: &mut App) {
        if let Some(banner) = Self::take(temp_id, cx) {
            banner.succeed(title, content, cx);
        }
    }

    pub fn fail(temp_id: &str, title: impl Into<String>, content: Option<String>, cx: &mut App) {
        if let Some(banner) = Self::take(temp_id, cx) {
            banner.fail(title, content, cx);
        }
    }

    /// caller shows its own outcome or has none (cancel, account switch)
    pub fn finish(temp_id: &str, cx: &mut App) {
        if let Some(banner) = Self::take(temp_id, cx) {
            banner.finish(cx);
        }
    }

    pub fn finish_all(cx: &mut App) {
        let state = cx.default_global::<MusicTrackUploadNotifierState>();
        let all: Vec<Rc<NotifierUploadProgress>> = state.banners.drain().map(|(_, banner)| banner).collect();
        state.downloads.clear();
        for banner in all {
            banner.finish(cx);
        }
    }

    fn take(temp_id: &str, cx: &mut App) -> Option<Rc<NotifierUploadProgress>> {
        let state = cx.default_global::<MusicTrackUploadNotifierState>();
        state.downloads.remove(temp_id);
        state.banners.remove(temp_id)
    }
}
