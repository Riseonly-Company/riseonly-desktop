#![allow(clippy::await_holding_lock)]

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, PoisonError};

use gpui::{App, RenderImage, Task, TestAppContext};
use image::{Frame, Rgba, RgbaImage};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::FileManager;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredPersistedRecord;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch::SaiFetch;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_handle::{SaiFetchHandlePhase, SaiFetchOperation};
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_memory_cache::SaiFetchMemoryCache;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_options::SaiFetchOptions;
use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::SaiWsChunkedFileUpload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_outbound_cancellation::SaiWsOutboundCancelledError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsRequestOptions, SaiWsRequestOptionsInit};
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::core::ui::emoji_picker_ui::emoji_picker_recents_store::EmojiPickerRecentsStore;
use crate::modules::auth::stores::auth_actions::auth_types::AuthAccountLimitPolicy;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{ChatWallpaperPreset, ChatWallpaperStore};
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::media_picker::shared::media_picker_recent_files::{MediaPickerRecentFiles, MediaPickerRecentKind};
use crate::modules::music::shared::utils::music::music_local_state::MusicLocalState;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::user::pages::profile::profile::AccountMenuAvatarLoader;
use crate::modules::vacancy::shared::vacancy_mocks::VacancyLocalState;

static SERIALIZED: Mutex<()> = Mutex::new(());

/// lock for tests, account scope is process wide and tests run on parallel threads
fn serialized() -> MutexGuard<'static, ()> {
    SERIALIZED.lock().unwrap_or_else(PoisonError::into_inner)
}

fn prepare_test(cx: &TestAppContext) -> MutexGuard<'static, ()> {
    let guard = serialized();
    cx.executor().allow_parking();
    crate::install_feature_hooks();
    guard
}

/// clears the active account cache scope when dropped
struct ClearAccountOnDrop;

impl Drop for ClearAccountOnDrop {
    fn drop(&mut self) {
        SaiAccountCacheScope::clear_active_account();
    }
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = FileManager::default().remove_item(&self.0);
    }
}

fn ws_options(service: &str, method: &str, cache_key: Option<&str>) -> SaiWsRequestOptionsInit {
    SaiWsRequestOptionsInit {
        service: service.to_owned(),
        method: method.to_owned(),
        cache_key: cache_key.map(str::to_owned),
        ..Default::default()
    }
}

fn fetch_options(cache_key: &str) -> SaiFetchOptions {
    SaiFetchOptions::with_cache_key(cache_key, true, false)
}

#[test]
fn standard_accounts_are_limited_to_three() {
    assert_eq!(AuthAccountLimitPolicy::limit(false), 3);
    assert!(AuthAccountLimitPolicy::can_add_account(2, false));
    assert!(!AuthAccountLimitPolicy::can_add_account(3, false));
}

#[test]
fn premium_accounts_are_limited_to_ten() {
    assert_eq!(AuthAccountLimitPolicy::limit(true), 10);
    assert!(AuthAccountLimitPolicy::can_add_account(9, true));
    assert!(!AuthAccountLimitPolicy::can_add_account(10, true));
}

#[test]
fn account_menu_avatar_bitmap_has_transparent_corners() {
    let source = RenderImage::new(vec![Frame::new(RgbaImage::from_pixel(20, 20, Rgba([255, 0, 0, 255])))]);
    let avatar = AccountMenuAvatarLoader::circular(&source).expect("a non-empty source gives a bitmap");
    let size = avatar.size(0);
    let (width, height) = (size.width.0 as usize, size.height.0 as usize);

    assert_eq!(alpha(&avatar, width, 0, 0), 0);
    assert!(alpha(&avatar, width, width / 2, height / 2) > 0);
}

#[test]
fn account_cache_keys_are_isolated_by_active_identity() {
    let _serial = serialized();
    SaiAccountCacheScope::activate("first-user");
    let first = SaiCache::new(["messages", "chat-1"]).key;
    SaiAccountCacheScope::activate("second-user");
    let second = SaiCache::new(["messages", "chat-1"]).key;
    SaiAccountCacheScope::clear_active_account();

    assert_ne!(first, second);
    assert!(first.starts_with("account:first-user:"));
    assert!(second.starts_with("account:second-user:"));
    assert_eq!(SaiAccountCacheScope::logical_key(&first), "messages-chat-1");
}

#[test]
fn raw_sai_cache_options_are_scoped_and_never_double_prefixed() {
    let _serial = serialized();
    SaiAccountCacheScope::activate("first-user");

    let ws = SaiWsRequestOptions::new(ws_options("post", "get_feed", Some("raw-feed")));
    let fetch = fetch_options("raw-profile");
    let scoped_key = SaiCache::new(["already-scoped"]).key;
    let already_scoped = SaiWsRequestOptions::new(ws_options("post", "get_feed", Some(&scoped_key)));

    SaiAccountCacheScope::clear_active_account();

    assert_eq!(ws.cache_key.as_deref(), Some("account:first-user:raw-feed"));
    assert_eq!(fetch.cache_key.as_deref(), Some("account:first-user:raw-profile"));
    assert_eq!(fetch.account_user_id.as_deref(), Some("first-user"));
    assert_eq!(fetch.account_epoch, ws.account_epoch);
    assert_eq!(already_scoped.cache_key.as_deref(), Some("account:first-user:already-scoped"));
}

#[test]
fn persisted_outbound_work_keeps_its_originating_account() {
    let _serial = serialized();
    let _clear = ClearAccountOnDrop;
    SaiAccountCacheScope::activate("first-user");
    let original = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        track_until_acknowledged: true,
        offline_ack_timeout_seconds: Some(42.0),
        ..ws_options("message", "create_message", Some("messages-chat-1"))
    });
    let mut data = Map::new();
    data.insert("chat_id".to_owned(), Value::from("chat-1"));
    let record = SaiWsDeferredPersistedRecord::make(Uuid::new_v4(), &Value::Object(data), &original)
        .expect("a dictionary body is valid");

    SaiAccountCacheScope::activate("second-user");
    let restored = record.to_request_options();

    assert_eq!(restored.account_user_id.as_deref(), Some("first-user"));
    assert_eq!(restored.account_epoch, original.account_epoch);
    assert_eq!(restored.cache_key.as_deref(), Some("account:first-user:messages-chat-1"));
    assert!(restored.track_until_acknowledged);
    assert_eq!(restored.offline_ack_timeout_seconds, Some(42.0));
    assert!(!SaiAccountCacheScope::is_current_account_epoch(
        restored.account_user_id.as_deref(),
        restored.account_epoch.as_deref()
    ));
}

#[test]
fn account_epoch_rejects_work_from_an_earlier_login_of_the_same_user() {
    let _serial = serialized();
    let _clear = ClearAccountOnDrop;
    SaiAccountCacheScope::clear_active_account();
    SaiAccountCacheScope::activate("same-user");
    let old_options = SaiWsRequestOptions::new(ws_options("post", "create_post", None));
    let first_epoch = old_options.account_epoch.clone().expect("an active account stamps its epoch");

    SaiAccountCacheScope::activate("same-user");
    assert_eq!(SaiAccountCacheScope::active_epoch().as_deref(), Some(first_epoch.as_str()));

    SaiAccountCacheScope::clear_active_account();
    SaiAccountCacheScope::activate("same-user");

    assert_ne!(SaiAccountCacheScope::active_epoch().as_deref(), Some(first_epoch.as_str()));
    assert!(!SaiAccountCacheScope::is_current_account_epoch(
        old_options.account_user_id.as_deref(),
        old_options.account_epoch.as_deref()
    ));
}

#[gpui::test]
async fn stale_chunked_upload_stops_before_reading_or_using_the_new_account(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let _clear = ClearAccountOnDrop;
    SaiAccountCacheScope::activate("upload-owner");
    let owner_user_id = SaiAccountCacheScope::active_user_id();
    let owner_epoch = SaiAccountCacheScope::active_epoch();
    SaiAccountCacheScope::activate("different-account");

    let upload = cx.update(|cx| {
        SaiWsChunkedFileUpload::upload(
            PathBuf::from("/file-that-must-never-be-read"),
            "private.jpg".to_owned(),
            "image/jpeg".to_owned(),
            "chat".to_owned(),
            None,
            owner_user_id,
            owner_epoch,
            None,
            None,
            cx,
        )
    });
    let cancelled_by_account_guard = match upload.await {
        Ok(_) => false,
        Err(error) => error.downcast_ref::<SaiWsOutboundCancelledError>().is_some(),
    };

    assert!(cancelled_by_account_guard);
}

#[gpui::test]
fn stale_http_completion_cannot_populate_or_call_back_into_the_new_account(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let _clear = ClearAccountOnDrop;
    SaiAccountCacheScope::activate("http-owner");
    let did_succeed = Rc::new(Cell::new(false));
    let did_fail = Rc::new(Cell::new(false));
    let operation: SaiFetchOperation<String> = Rc::new(|_cx: &mut App| {
        SaiAccountCacheScope::activate("different-http-account");
        Task::ready(Ok("private-response".to_owned()))
    });
    let handle = cx.update(|cx| {
        let handle = SaiFetch::request_operation(operation, fetch_options("private-http-response"), cx);
        let (succeeded, failed) = (did_succeed.clone(), did_fail.clone());
        handle.update(cx, |handle, cx| {
            handle.start(move |_, _| succeeded.set(true), move |_, _| failed.set(true), cx);
        });
        handle
    });
    cx.run_until_parked();

    assert!(!did_succeed.get());
    assert!(!did_fail.get());
    cx.read(|cx| {
        let handle = handle.read(cx);
        assert!(handle.data().is_none());
        assert_eq!(handle.phase(), SaiFetchHandlePhase::Idle);
        assert!(SaiFetchMemoryCache::value("account:http-owner:private-http-response", cx).is_none());
    });
}

fn reset_local_states(cx: &mut App) {
    MusicLocalState::shared(cx).update(cx, |state, cx| state.reset_account_state(cx));
    VacancyLocalState::shared(cx).update(cx, |state, cx| state.reset_account_state(cx));
    MediaPickerRecentFiles::shared(cx).update(cx, |state, cx| state.reset_account_state(cx));
    EmojiPickerRecentsStore::shared(cx).update(cx, |state, cx| state.reset_account_state(cx));
}

fn activate_local_states(cx: &mut App) {
    MusicLocalState::shared(cx).update(cx, |state, cx| state.activate_account_state(cx));
    VacancyLocalState::shared(cx).update(cx, |state, cx| state.activate_account_state(cx));
    MediaPickerRecentFiles::shared(cx).update(cx, |state, cx| state.activate_account_state(cx));
    EmojiPickerRecentsStore::shared(cx).update(cx, |state, cx| state.activate_account_state(cx));
}

fn remove_persisted_local_states(user_id: &str, cx: &mut App) {
    MusicLocalState::shared(cx).read(cx).remove_persisted_account(user_id);
    VacancyLocalState::shared(cx).update(cx, |state, cx| state.remove_persisted_account(user_id, cx));
    MediaPickerRecentFiles::shared(cx).read(cx).remove_persisted_account(user_id);
    EmojiPickerRecentsStore::shared(cx).read(cx).remove_persisted_account(user_id);
    ChatWallpaperStore::shared(cx).read(cx).remove_persisted_account(user_id);
}

fn remove_persisted_account(user_id: &str, cx: &mut App) {
    remove_persisted_local_states(user_id, cx);
    MusicPlaybackDiskCache::remove_persisted_account(user_id);
    VoiceMessageDiskCache::remove_persisted_account(user_id);
    CleverImageDiskCache::remove_persisted_account(user_id);
}

#[gpui::test]
fn local_histories_scroll_positions_and_voice_paths_are_partitioned_by_account(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let _clear = ClearAccountOnDrop;
    let first_user = "local-state-first";
    let second_user = "local-state-second";
    let source_file = std::env::temp_dir()
        .join(format!("multi-account-private-attachment-{}.txt", Uuid::new_v4().uuid_string()));
    std::fs::write(&source_file, "private attachment").expect("the temp directory is writable");
    let _remove_source = RemoveOnDrop(source_file.clone());
    let first_image_bytes = b"first-private-image".to_vec();

    let (first_voice_path, first_image_cache_path, first_music_path) = cx.update(|cx| {
        SaiAccountCacheScope::activate(first_user);
        remove_persisted_local_states(first_user, cx);
        reset_local_states(cx);
        ChatScrollPositionStore::clear_all();
        activate_local_states(cx);
        let wallpapers = ChatWallpaperStore::shared(cx);
        wallpapers.update(cx, |store, cx| {
            store.reset_account_state(cx);
            store.activate_account_state(cx);
        });
        ChatScrollPositionStore::activate_account_state(cx);
        MusicLocalState::shared(cx).update(cx, |state, cx| state.add_search("first-private-search", cx));
        VacancyLocalState::shared(cx).update(cx, |state, cx| {
            state.mark_viewed(101, cx);
            state.set_default_resume_id(501, cx);
        });
        MediaPickerRecentFiles::shared(cx)
            .update(cx, |files, cx| {
                files.import_file(
                    &source_file,
                    MediaPickerRecentKind::File,
                    "private.txt",
                    None,
                    None,
                    None,
                    None,
                    cx,
                )
            })
            .expect("the attachment imports");
        EmojiPickerRecentsStore::shared(cx).update(cx, |store, cx| store.register("\u{1F170}\u{FE0F}", cx));
        ChatScrollPositionStore::save_offset(240.0, "same-chat", false, cx);
        wallpapers.update(cx, |store, cx| {
            store.select(&ChatWallpaperPreset::all()[1], cx);
            store.set_pattern_intensity(0.42, cx);
            store.set_motion_enabled(false, cx);
        });
        let first_voice_path = VoiceMessageDiskCache::file_url("same-chat", "same-message").expect("a voice path");
        let first_image_cache_path = CleverImageDiskCache::folder_path_for_inspector(None);
        CleverImageDiskCache::store_data(&first_image_bytes, "same-image", None, None);
        let first_music_path = MusicPlaybackDiskCache::file_url("same-track", "mp3", None).expect("a music path");
        std::fs::write(&first_music_path, [0x49, 0x44, 0x33, 0x04]).expect("the music directory is writable");
        let file_name = first_music_path.file_name().and_then(|name| name.to_str()).expect("a file name");
        MusicPlaybackDiskCache::register_saved_file("same-track", file_name, None);
        (first_voice_path, first_image_cache_path, first_music_path)
    });

    let (second_voice_path, second_image_cache_path, second_music_path) = cx.update(|cx| {
        reset_local_states(cx);
        ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        ChatScrollPositionStore::reset_account_state(true, cx);
        SaiAccountCacheScope::activate(second_user);
        MusicLocalState::shared(cx).read(cx).remove_persisted_account(second_user);
        VacancyLocalState::shared(cx).update(cx, |state, cx| state.remove_persisted_account(second_user, cx));
        MusicLocalState::shared(cx).update(cx, |state, cx| state.activate_account_state(cx));
        VacancyLocalState::shared(cx).update(cx, |state, cx| state.activate_account_state(cx));
        MediaPickerRecentFiles::shared(cx).read(cx).remove_persisted_account(second_user);
        EmojiPickerRecentsStore::shared(cx).read(cx).remove_persisted_account(second_user);
        ChatWallpaperStore::shared(cx).read(cx).remove_persisted_account(second_user);
        MediaPickerRecentFiles::shared(cx).update(cx, |state, cx| state.activate_account_state(cx));
        EmojiPickerRecentsStore::shared(cx).update(cx, |state, cx| state.activate_account_state(cx));
        ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
        ChatScrollPositionStore::activate_account_state(cx);
        (
            VoiceMessageDiskCache::file_url("same-chat", "same-message").expect("a voice path"),
            CleverImageDiskCache::folder_path_for_inspector(None),
            MusicPlaybackDiskCache::file_url("same-track", "mp3", None).expect("a music path"),
        )
    });

    cx.update(|cx| {
        assert!(MusicLocalState::shared(cx).read(cx).recent_searches.is_empty());
        assert!(!VacancyLocalState::shared(cx).read(cx).viewed_ids().contains(&101));
        assert_eq!(VacancyLocalState::shared(cx).update(cx, |state, cx| state.default_resume_id(cx)), None);
        assert!(MediaPickerRecentFiles::shared(cx).read(cx).entries().is_empty());
        assert!(EmojiPickerRecentsStore::shared(cx).read(cx).recents().is_empty());
        assert_eq!(ChatScrollPositionStore::saved_offset("same-chat", cx), None);
        let wallpapers = ChatWallpaperStore::shared(cx);
        assert_eq!(wallpapers.read(cx).selected_preset_id(), ChatWallpaperPreset::default_preset().id);
        assert_eq!(wallpapers.read(cx).pattern_intensity(), 0.86);
        assert!(!wallpapers.read(cx).motion_enabled());
    });
    assert_ne!(first_voice_path, second_voice_path);
    assert_ne!(first_image_cache_path, second_image_cache_path);
    assert_eq!(CleverImageDiskCache::image_data("same-image", None, None), None);
    assert_ne!(first_music_path, second_music_path);
    assert_eq!(MusicPlaybackDiskCache::find_cached_url("same-track", None), None);

    cx.update(|cx| {
        reset_local_states(cx);
        ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        ChatScrollPositionStore::reset_account_state(true, cx);
        SaiAccountCacheScope::activate(first_user);
        activate_local_states(cx);
        ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
        ChatScrollPositionStore::activate_account_state(cx);

        assert_eq!(MusicLocalState::shared(cx).read(cx).recent_searches, vec!["first-private-search"]);
        assert!(VacancyLocalState::shared(cx).read(cx).viewed_ids().contains(&101));
        assert_eq!(VacancyLocalState::shared(cx).update(cx, |state, cx| state.default_resume_id(cx)), Some(501));
        let entries = MediaPickerRecentFiles::shared(cx)
            .read(cx)
            .entries()
            .iter()
            .map(|entry| entry.display_name.clone())
            .collect::<Vec<_>>();
        assert_eq!(entries, vec!["private.txt"]);
        assert_eq!(EmojiPickerRecentsStore::shared(cx).read(cx).recents(), vec!["\u{1F170}\u{FE0F}"]);
        assert_eq!(ChatScrollPositionStore::saved_offset("same-chat", cx), Some(240.0));
        let wallpapers = ChatWallpaperStore::shared(cx);
        assert_eq!(wallpapers.read(cx).selected_preset_id(), ChatWallpaperPreset::all()[1].id);
        assert_eq!(wallpapers.read(cx).pattern_intensity(), 0.42);
        assert!(!wallpapers.read(cx).motion_enabled());
    });
    assert_eq!(CleverImageDiskCache::image_data("same-image", None, None), Some(first_image_bytes));
    assert_eq!(MusicPlaybackDiskCache::find_cached_url("same-track", None), Some(first_music_path));

    cx.update(|cx| {
        remove_persisted_account(first_user, cx);
        remove_persisted_account(second_user, cx);
        ChatScrollPositionStore::clear_all();
    });
}

fn alpha(image: &RenderImage, width: usize, x: usize, y: usize) -> u8 {
    let bytes = image.as_bytes(0).expect("the bitmap has a frame");
    bytes[(y * width + x) * 4 + 3]
}
