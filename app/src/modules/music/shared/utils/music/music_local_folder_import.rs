use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use futures::channel::oneshot;
use gpui::{App, AppContext, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::{RiseUploadContextFormats, RiseUploadFormat};
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::file_manager::FileManager;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::notifier::notifier_upload_progress::NotifierUploadProgress;
use crate::modules::music::shared::music_field_rules::MusicFieldRules;
use crate::modules::music::shared::utils::music::music_track_file_upload::MusicTrackFileUpload;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;

const MINIMUM_AUDIO_DURATION_MS: i64 = 500;

/// one folder going into a local playlist, settled counts finished uploads either way
#[derive(Clone, Debug, PartialEq)]
pub struct MusicLocalFolderImportProgress {
    pub playlist_id: String,
    pub total: i64,
    pub settled: i64,
    pub failed: i64,
    pub current: f64,
}

impl MusicLocalFolderImportProgress {
    pub fn fraction(&self) -> f64 {
        if self.total <= 0 {
            return 0.0;
        }
        ((self.settled as f64 + self.current) / self.total as f64).min(1.0)
    }
}

/// every mp3 of a folder goes through the single track queue under one shared banner
pub struct MusicLocalFolderImport {
    progress: Option<MusicLocalFolderImportProgress>,
    members: HashSet<String>,
    is_queuing: bool,
    banner: Option<Rc<NotifierUploadProgress>>,
    generation: u64,
    task: Option<Task<()>>,
}

impl MusicLocalFolderImport {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { progress: None, members: HashSet::new(), is_queuing: false, banner: None, generation: 0, task: None }
    }

    pub fn progress(&self) -> Option<&MusicLocalFolderImportProgress> {
        self.progress.as_ref()
    }

    pub fn owns(&self, temp_id: &str) -> bool {
        self.members.contains(temp_id)
    }

    pub fn start(&mut self, folder: PathBuf, playlist_id: String, cx: &mut Context<Self>) {
        if self.progress.is_some() || self.is_queuing {
            Self::toast(tr("music_local_folder_busy"), NotifierType::Info, cx);
            return;
        }
        let Some(account_user_id) = SaiAccountCacheScope::active_user_id() else {
            return;
        };
        if playlist_id.is_empty() {
            return;
        }
        let account_epoch = SaiAccountCacheScope::active_epoch();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.is_queuing = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let files = cx.background_spawn(async move { Self::mp3_files(&folder) }).await;
            let account = (account_user_id, account_epoch);
            let files = this
                .update(cx, |this, cx| this.begin_batch(generation, &account, files, &playlist_id, cx))
                .ok()
                .flatten();
            let Some(files) = files else {
                return;
            };
            for file in files {
                let queued = Self::queue(&this, file, &playlist_id, cx).await;
                let current = this
                    .update(cx, |this, cx| {
                        if !this.is_current(generation, &account) {
                            return false;
                        }
                        if !queued {
                            this.settle(true, cx);
                        }
                        true
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.is_queuing = false;
                this.finish_if_done(cx);
            });
        }));
    }

    pub fn member_progress(&mut self, temp_id: &str, fraction: f64, cx: &mut Context<Self>) {
        if !self.members.contains(temp_id) {
            return;
        }
        let Some(progress) = self.progress.as_mut() else {
            return;
        };
        let clamped = fraction.clamp(0.0, 1.0);
        if (clamped * 100.0) as i64 == (progress.current * 100.0) as i64 {
            return;
        }
        progress.current = clamped;
        let overall = progress.fraction();
        if let Some(banner) = self.banner.clone() {
            banner.update(overall, cx);
        }
        cx.notify();
    }

    pub fn member_settled(&mut self, temp_id: &str, succeeded: bool, cx: &mut Context<Self>) {
        if !self.members.remove(temp_id) {
            return;
        }
        self.settle(!succeeded, cx);
        self.finish_if_done(cx);
    }

    pub fn cancel_for_account_change(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.task = None;
        self.members.clear();
        self.is_queuing = false;
        if let Some(banner) = self.banner.take() {
            banner.finish(cx);
        }
        self.progress = None;
        cx.notify();
    }

    fn begin_batch(
        &mut self,
        generation: u64,
        account: &(String, Option<String>),
        files: Option<Vec<PathBuf>>,
        playlist_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Vec<PathBuf>> {
        if !self.is_current(generation, account) {
            return None;
        }
        let Some(files) = files else {
            self.is_queuing = false;
            Self::toast(tr("music_local_folder_unreadable"), NotifierType::Error, cx);
            return None;
        };
        if files.is_empty() {
            self.is_queuing = false;
            Self::toast(tr("music_local_folder_empty"), NotifierType::Info, cx);
            return None;
        }
        self.progress = Some(MusicLocalFolderImportProgress {
            playlist_id: playlist_id.to_owned(),
            total: files.len() as i64,
            settled: 0,
            failed: 0,
            current: 0.0,
        });
        self.banner = Some(Rc::new(NotifierUploadProgress::new(tr("music_local_folder_upload_banner"), None, None, cx)));
        cx.notify();
        Some(files)
    }

    fn is_current(&self, generation: u64, account: &(String, Option<String>)) -> bool {
        generation == self.generation
            && SaiAccountCacheScope::is_current_account_epoch(Some(account.0.as_str()), account.1.as_deref())
    }

    fn settle(&mut self, failed: bool, cx: &mut Context<Self>) {
        let Some(progress) = self.progress.as_mut() else {
            return;
        };
        progress.settled += 1;
        if failed {
            progress.failed += 1;
        }
        progress.current = 0.0;
        let overall = progress.fraction();
        if let Some(banner) = self.banner.clone() {
            banner.update(overall, cx);
        }
        cx.notify();
    }

    fn finish_if_done(&mut self, cx: &mut Context<Self>) {
        let done = !self.is_queuing
            && self.members.is_empty()
            && self.progress.as_ref().is_some_and(|progress| progress.settled >= progress.total);
        if !done {
            return;
        }
        let Some(progress) = self.progress.take() else {
            return;
        };
        let added = progress.total - progress.failed;
        let title = tr_args("music_local_folder_done", &[FormatArg::Int(added), FormatArg::Int(progress.total)]);
        let content = (progress.failed > 0)
            .then(|| tr_args("music_local_folder_failed_count", &[FormatArg::Int(progress.failed)]));
        if let Some(banner) = self.banner.take() {
            if added > 0 {
                banner.succeed(title, content, cx);
            } else {
                banner.fail(title, content, cx);
            }
        }
        cx.notify();
    }

    async fn queue(this: &WeakEntity<Self>, file: PathBuf, playlist_id: &str, cx: &mut AsyncApp) -> bool {
        let source = file.clone();
        let prepared = cx.background_spawn(async move { MusicTrackFileUpload::prepare_mp3(&source).ok() }).await;
        let Some(prepared) = prepared else {
            return false;
        };
        let original_name = file.file_name().map(|name| name.to_string_lossy().into_owned());
        let metadata = MusicTrackFileUpload::imported_metadata(prepared.clone(), original_name).await;
        let title_rule = MusicFieldRules::local_track_title();
        let artist_rule = MusicFieldRules::local_track_artist();
        let title = title_rule.clamped(&metadata.title);
        let artist = artist_rule.clamped(&metadata.artist.clone().unwrap_or_else(|| tr("music_unknown_artist")));
        let normalized_title = title_rule.schema.map(|schema| schema.normalize(&title)).unwrap_or_else(|| title.clone());
        let normalized_artist =
            artist_rule.schema.map(|schema| schema.normalize(&artist)).unwrap_or_else(|| artist.clone());
        let file_url = Url::from_file_path(&prepared);
        let valid = metadata.duration_ms >= MINIMUM_AUDIO_DURATION_MS
            && title_rule.is_satisfied(&normalized_title)
            && artist_rule.is_satisfied(&normalized_artist);
        let Some(file_url) = file_url.filter(|_| valid) else {
            let _ = FileManager.remove_item(&prepared);
            return false;
        };
        let artwork_data = metadata.artwork_data.clone();
        let cover = cx
            .background_spawn(async move { artwork_data.and_then(|data| MusicTrackFileUpload::local_artwork_file(&data)) })
            .await;
        let filename = file_url.last_path_component();
        let mut audio = SaiWsOptimisticLocalFile::new(file_url, filename, RiseUploadFormat::AUDIO_MPEG.mime.to_owned());
        audio.file_context_override = Some(RiseUploadContextFormats::MUSIC_AUDIO.context.to_owned());

        let temp_id = format!("temp_local_{}", Uuid::new_v4().uuid_string());
        if this.update(cx, |this, _| this.members.insert(temp_id.clone())).is_err() {
            return false;
        }
        let (sender, receiver) = oneshot::channel::<bool>();
        let sender = Rc::new(RefCell::new(Some(sender)));
        let accepted_sender = sender.clone();
        let refused_sender = sender;
        let request_id = Uuid::new_v4().uuid_string().to_lowercase();
        let playlist_id = playlist_id.to_owned();
        let created_id = temp_id.clone();
        let _ = cx.update(move |cx| {
            MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.create_local_track_in_playlist(
                    &request_id,
                    &created_id,
                    &playlist_id,
                    &title,
                    None,
                    &artist,
                    metadata.duration_ms,
                    audio,
                    cover,
                    Some(Box::new(move |_: &mut App| {
                        if let Some(sender) = accepted_sender.borrow_mut().take() {
                            let _ = sender.send(true);
                        }
                    })),
                    None,
                    Some(Rc::new(move |_: String, _: &mut App| {
                        if let Some(sender) = refused_sender.borrow_mut().take() {
                            let _ = sender.send(false);
                        }
                    })),
                    cx,
                );
            });
        });
        let accepted = receiver.await.unwrap_or(false);
        if !accepted {
            let _ = this.update(cx, |this, _| this.members.remove(&temp_id));
        }
        accepted
    }

    fn toast(title: String, kind: NotifierType, cx: &mut App) {
        Notifier::send(
            title,
            NotifierSendOptions { r#type: kind, position: NotifierPosition::Bottom, ..NotifierSendOptions::default() },
            cx,
        );
    }

    fn mp3_files(folder: &Path) -> Option<Vec<PathBuf>> {
        let mut files = Vec::new();
        let mut pending = vec![folder.to_path_buf()];
        let mut is_root = true;
        while let Some(directory) = pending.pop() {
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(_) if is_root => return None,
                Err(_) => continue,
            };
            is_root = false;
            for entry in entries.flatten() {
                let path = entry.path();
                let hidden = path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with('.'));
                if hidden {
                    continue;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    pending.push(path);
                } else if kind.is_file() && Self::is_mp3(&path) {
                    files.push(path);
                }
            }
        }
        files.sort_by(|left, right| Self::standard_order(&left.to_string_lossy(), &right.to_string_lossy()));
        Some(files)
    }

    fn is_mp3(path: &Path) -> bool {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| MusicTrackFileUpload::MP3_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
    }

    fn standard_order(left: &str, right: &str) -> Ordering {
        let (mut left_chunks, mut right_chunks) = (Self::chunks(left).into_iter(), Self::chunks(right).into_iter());
        loop {
            match (left_chunks.next(), right_chunks.next()) {
                (None, None) => return compare_case_insensitive(left, right),
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
                (Some(a), Some(b)) => {
                    let order = match (a.parse::<u128>(), b.parse::<u128>()) {
                        (Ok(a_value), Ok(b_value)) => a_value.cmp(&b_value).then(a.len().cmp(&b.len())),
                        _ => compare_case_insensitive(a, b),
                    };
                    if order != Ordering::Equal {
                        return order;
                    }
                }
            }
        }
    }

    fn chunks(text: &str) -> Vec<&str> {
        let mut chunks = Vec::new();
        let mut start = 0;
        let mut digits: Option<bool> = None;
        for (index, character) in text.char_indices() {
            let is_digit = character.is_ascii_digit();
            if digits.is_some_and(|current| current != is_digit) {
                chunks.push(&text[start..index]);
                start = index;
            }
            digits = Some(is_digit);
        }
        if start < text.len() {
            chunks.push(&text[start..]);
        }
        chunks
    }
}
