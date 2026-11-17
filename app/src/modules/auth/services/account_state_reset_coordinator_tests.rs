#![allow(clippy::await_holding_lock)]

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use gpui::{App, Entity, Task, TestAppContext};
use rise_canon::rise_key;
use rise_canon::rise_view;
use rise_canon::rise_wire_canon::key as wire_key;
use rise_core::foundation::foundation_date::RiseClock;
use rise_widgets::overlay::blocking_loader_store::BlockingLoaderStore;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice, SaiWsSliceStatus};
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredOutboundCoordinator;
use crate::core::lib::sai::core::websocket::sai_ws_response_debt_coordinator::SaiWsResponseDebtCoordinator;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{SaiDebouncer, SaiDebouncerOptions};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsPendingCacheBinding, SaiWsRequestOptions,
    SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::auth::services::account_state_reset_coordinator::{
    AccountStateResetCoordinator, AccountStateResetReason,
};
use crate::modules::auth::services::auth_local_storage::AuthLocalStorage;
use crate::modules::auth::services::auth_session_guard::{AuthSessionGuard, AuthSessionGuardRecovery};
use crate::modules::auth::services::auth_token_refresh::{AuthTokenRefresh, AuthTokenRefreshRequest};
use crate::modules::auth::stores::auth_actions::auth_types::StoredAccessTokens;
use crate::modules::auth::stores::auth_actions::auth_wire_types::{
    AuthRefreshResponse, AuthRegisterResponse, AuthUserDTO,
};
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthLogoutWorkflow;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::shared::bot_ui_validation::{BotCreateFlowState, BotTagAvailabilityState};
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, MessageMediaKind};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct AccountResetFixtureDTO {
    owner: String,
}

static SERIALIZED: Mutex<()> = Mutex::new(());

/// serializes tests, account scope and defaults and credential store live in the process
/// and libtest runs tests on parallel threads
fn serialized() -> MutexGuard<'static, ()> {
    SERIALIZED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// makes the stores of the whole app answer the reset like after real app start
/// the engine runtime has its own threads so the test executor may park while it finishes
fn prepare_test(cx: &TestAppContext) -> MutexGuard<'static, ()> {
    let guard = serialized();
    cx.executor().allow_parking();
    crate::install_feature_hooks();
    guard
}

fn auth_register_fixture(user_id: &str, access_token: &str, refresh_token: &str, session_id: &str) -> AuthRegisterResponse {
    AuthRegisterResponse {
        user: Some(rise_view::auth::UserData {
            id: user_id.to_owned(),
            name: user_id.to_owned(),
            tag: user_id.to_owned(),
            ..Default::default()
        }),
        access_token: access_token.to_owned(),
        refresh_token: refresh_token.to_owned(),
        session_id: session_id.to_owned(),
        success: true,
        ..Default::default()
    }
}

fn auth_refresh_fixture(access_token: &str, refresh_token: &str, session_id: &str) -> AuthRefreshResponse {
    AuthRefreshResponse {
        access_token: access_token.to_owned(),
        refresh_token: refresh_token.to_owned(),
        session_id: session_id.to_owned(),
        ..Default::default()
    }
}

fn account_reset_jwt(user_id: &str, expires_at: f64) -> String {
    let encode_object = |entries: Vec<(&str, Value)>| {
        let object: Map<String, Value> = entries.into_iter().map(|(name, value)| (name.to_owned(), value)).collect();
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Value::Object(object)).expect("jwt part"))
    };
    let header = encode_object(vec![("alg", Value::from("none")), ("typ", Value::from("JWT"))]);
    let payload = encode_object(vec![("id", Value::from(user_id)), ("exp", Value::from(expires_at as i64))]);
    format!("{header}.{payload}.test")
}

fn hour_ago() -> f64 {
    RiseClock::now().time_interval_since_1970() - 3_600.0
}

fn hour_ahead() -> f64 {
    RiseClock::now().time_interval_since_1970() + 3_600.0
}

fn persist_restore_account_fixture(user_id: &str, access_token: &str, refresh_token: &str, last_used_at: i64) {
    let user = AuthUserDTO {
        id: user_id.to_owned(),
        name: Some(user_id.to_owned()),
        tag: Some(user_id.to_owned()),
        ..Default::default()
    };
    let tokens = StoredAccessTokens {
        access_token: access_token.to_owned(),
        refresh_token: refresh_token.to_owned(),
    };
    AuthLocalStorage::save_account(&user, &tokens, &format!("session-{user_id}"), last_used_at, None, false)
        .expect("account saved");
}

fn reset_refresh_account_fixtures(auth: &Entity<AuthServicesStore>, cx: &TestAppContext) {
    AuthLocalStorage::clear_all_accounts();
    cx.update(|cx| auth.update(cx, |auth, cx| auth.full_clear(AccountStateResetReason::ForcedLogout, cx)));
    cx.run_until_parked();
    AuthLocalStorage::clear_all_accounts();
    cx.update(|cx| SaiWebSocketManager::shared(cx).update(cx, |manager, cx| manager.reset(cx)));
    cx.run_until_parked();
}

fn install_refresh_account_fixtures(auth: &Entity<AuthServicesStore>, cx: &TestAppContext) {
    reset_refresh_account_fixtures(auth, cx);
    for (user_id, access_token, refresh_token, session_id) in [
        ("refresh-account-a", "access-a-v1", "refresh-a-v1", "session-a"),
        ("refresh-account-b", "access-b-v1", "refresh-b-v1", "session-b"),
    ] {
        let response = auth_register_fixture(user_id, access_token, refresh_token, session_id);
        cx.update(|cx| auth.update(cx, |auth, cx| auth.handle_register_success(response, cx)));
        cx.run_until_parked();
    }
    switch_account(auth, "refresh-account-a", cx);
}

fn switch_account(auth: &Entity<AuthServicesStore>, user_id: &str, cx: &TestAppContext) {
    cx.update(|cx| auth.update(cx, |auth, cx| auth.switch_account(user_id, cx)));
    cx.run_until_parked();
}

fn shared_auth(cx: &TestAppContext) -> Entity<AuthServicesStore> {
    cx.update(AuthServicesStore::shared)
}

fn active_account_id(auth: &Entity<AuthServicesStore>, cx: &TestAppContext) -> Option<String> {
    cx.read(|cx| auth.read(cx).active_account_id().map(ToOwned::to_owned))
}

fn is_authenticated(auth: &Entity<AuthServicesStore>, cx: &TestAppContext) -> bool {
    cx.read(|cx| auth.read(cx).is_authenticated())
}

fn access_token(auth: &Entity<AuthServicesStore>, cx: &TestAppContext) -> String {
    cx.read(|cx| auth.read(cx).tokens_and_other_data().access_token.clone())
}

fn refresh_token(auth: &Entity<AuthServicesStore>, cx: &TestAppContext) -> String {
    cx.read(|cx| auth.read(cx).tokens_and_other_data().refresh_token.clone())
}

fn stored_access_token(user_id: &str) -> Option<String> {
    AuthLocalStorage::load_account(user_id).map(|account| account.tokens.access_token)
}

fn stored_refresh_token(user_id: &str) -> Option<String> {
    AuthLocalStorage::load_account(user_id).map(|account| account.tokens.refresh_token)
}

fn hard_reset(reason: AccountStateResetReason, cx: &TestAppContext) {
    cx.update(|cx| AccountStateResetCoordinator::hard_reset(reason, true, false, cx));
    cx.run_until_parked();
}

fn prepare_for_activation(user_id: &str, cx: &TestAppContext) {
    cx.update(|cx| AccountStateResetCoordinator::prepare_for_activation(user_id, false, cx));
    cx.run_until_parked();
}

fn owner_root(owner: &str) -> Value {
    let mut root = Map::new();
    root.insert("owner".to_owned(), Value::from(owner));
    Value::Object(root)
}

fn install_root(root: Value, cache_key: &str, cx: &TestAppContext) {
    cx.update(|cx| {
        SaiWsResponseCache::install_optimistic_root(
            root,
            cache_key,
            true,
            true,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            cx,
        )
    })
    .expect("root installed");
}

fn current_payload(cache_key: &str, cx: &TestAppContext) -> Option<Value> {
    cx.update(|cx| {
        SaiWsResponseCache::current_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
    })
}

fn disk_only(cache_key: &str, cx: &TestAppContext) -> Option<Value> {
    cx.update(|cx| SaiWsResponseCache::disk_only(cache_key, cx))
}

fn feed_root(id: &str, is_liked: bool, is_reposted: bool) -> Value {
    let mut row = Map::new();
    row.insert(rise_key::post::ID.to_owned(), Value::from(id));
    row.insert(rise_key::post::IS_LIKED.to_owned(), Value::Bool(is_liked));
    row.insert(RiseLocalKey::OLD_IS_LIKED.to_owned(), Value::Bool(is_liked));
    row.insert(rise_key::post::IS_REPOSTED.to_owned(), Value::Bool(is_reposted));
    let mut root = Map::new();
    root.insert(rise_key::post::LIST.to_owned(), Value::Array(vec![Value::Object(row)]));
    root.insert(rise_key::post::IS_HAVE_MORE.to_owned(), Value::Bool(false));
    Value::Object(root)
}

fn assert_feed_row(root: Option<Value>, id: &str, is_liked: bool, is_reposted: bool) {
    let root = root.expect("feed root");
    let row = root.get(rise_key::post::LIST).and_then(|list| list.get(0)).expect("feed row");
    assert_eq!(row.get(rise_key::post::ID).and_then(Value::as_str), Some(id));
    assert_eq!(row.get(rise_key::post::IS_LIKED).and_then(Value::as_bool), Some(is_liked));
    assert_eq!(row.get(rise_key::post::IS_REPOSTED).and_then(Value::as_bool), Some(is_reposted));
}

fn feed_items(cx: &TestAppContext) -> Vec<GetPostFeedResponse> {
    cx.update(|cx| {
        let store = PostServicesStore::shared(cx);
        store.update(cx, |store, cx| store.feed_items(cx))
    })
}

fn write_music_file(track_id: &str, cx: &TestAppContext) -> std::path::PathBuf {
    let path = MusicPlaybackDiskCache::file_url(track_id, "mp3", None).expect("music file url");
    std::fs::write(&path, [0x49_u8, 0x44, 0x33, 0x04]).expect("music file written");
    let file_name = path.file_name().and_then(|name| name.to_str()).expect("music file name");
    MusicPlaybackDiskCache::register_saved_file(track_id, file_name, None);
    cx.run_until_parked();
    path
}

#[gpui::test]
async fn logout_success_still_finishes_with_local_reset(cx: &mut TestAppContext) {
    let sends = Rc::new(Cell::new(0));
    let resets = Rc::new(Cell::new(0));
    let (send_count, reset_count) = (Rc::clone(&sends), Rc::clone(&resets));
    let mut app = cx.to_async();

    AuthLogoutWorkflow::run(
        true,
        1_000_000,
        move |finish, cx| {
            send_count.set(send_count.get() + 1);
            finish(true, cx);
        },
        move |_| reset_count.set(reset_count.get() + 1),
        &mut app,
    )
    .await;

    assert_eq!(sends.get(), 1);
    assert_eq!(resets.get(), 1);
}

#[gpui::test]
async fn logout_server_failure_still_finishes_with_local_reset(cx: &mut TestAppContext) {
    let resets = Rc::new(Cell::new(0));
    let reset_count = Rc::clone(&resets);
    let mut app = cx.to_async();

    AuthLogoutWorkflow::run(
        true,
        1_000_000,
        |finish, cx| finish(false, cx),
        move |_| reset_count.set(reset_count.get() + 1),
        &mut app,
    )
    .await;

    assert_eq!(resets.get(), 1);
}

#[gpui::test]
async fn logout_timeout_still_finishes_with_local_reset(cx: &mut TestAppContext) {
    let resets = Rc::new(Cell::new(0));
    let reset_count = Rc::clone(&resets);
    let mut app = cx.to_async();

    let workflow = cx.foreground_executor().spawn(async move {
        AuthLogoutWorkflow::run(
            true,
            1_000_000,
            |_, _| {},
            move |_| reset_count.set(reset_count.get() + 1),
            &mut app,
        )
        .await;
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_nanos(1_000_000));
    workflow.await;

    assert_eq!(resets.get(), 1);
}

#[gpui::test]
async fn offline_logout_skips_transport_and_resets_immediately(cx: &mut TestAppContext) {
    let sends = Rc::new(Cell::new(0));
    let resets = Rc::new(Cell::new(0));
    let (send_count, reset_count) = (Rc::clone(&sends), Rc::clone(&resets));
    let mut app = cx.to_async();

    AuthLogoutWorkflow::run(
        false,
        AuthLogoutWorkflow::DEFAULT_TIMEOUT_NANOSECONDS,
        move |_, _| send_count.set(send_count.get() + 1),
        move |_| reset_count.set(reset_count.get() + 1),
        &mut app,
    )
    .await;

    assert_eq!(sends.get(), 0);
    assert_eq!(resets.get(), 1);
}

#[gpui::test]
fn hard_reset_clears_caches_slices_and_persisted_replay(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    hard_reset(AccountStateResetReason::ForcedLogout, cx);
    let cache_key = "account-reset-fixture";
    let slice = cx.update(SaiTypedSlice::<AccountResetFixtureDTO>::new);
    slice.mark_fulfilled(Some(&owner_root("account-a")), None);
    install_root(owner_root("account-a"), cache_key, cx);

    cx.update(|cx| {
        let debt = SaiWsResponseDebtCoordinator::shared(cx);
        debt.update(cx, |debt, _| debt.test_append_record(7_001, "message", "create_message"));
        debt.read(cx).test_persist_records_to_disk(cx);
        let deferred = SaiWsDeferredOutboundCoordinator::shared(cx);
        deferred.update(cx, |deferred, _| deferred.test_enqueue_without_continuation("comment", "create_comment"));
        deferred.read(cx).test_persist_records_to_disk(cx);

        assert_eq!(slice.status(), SaiWsSliceStatus::Fulfilled);
        assert_eq!(slice.data().map(|fixture| fixture.owner).as_deref(), Some("account-a"));
        assert_eq!(debt.read(cx).test_disk_record_count(), 1);
        assert_eq!(deferred.read(cx).test_persisted_disk_record_count(), 1);
    });

    hard_reset(AccountStateResetReason::ForcedLogout, cx);

    assert_eq!(slice.status(), SaiWsSliceStatus::Idle);
    assert_eq!(slice.data(), None);
    assert_eq!(current_payload(cache_key, cx), None);
    cx.update(|cx| {
        let debt = SaiWsResponseDebtCoordinator::shared(cx);
        assert_eq!(debt.read(cx).test_record_count(), 0);
        assert_eq!(debt.read(cx).test_disk_record_count(), 0);
        let deferred = SaiWsDeferredOutboundCoordinator::shared(cx);
        assert_eq!(deferred.read(cx).test_fifo_count(), 0);
        assert_eq!(deferred.read(cx).test_persisted_disk_record_count(), 0);
    });
}

#[gpui::test]
fn activating_different_owner_drops_previous_server_state(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    hard_reset(AccountStateResetReason::ForcedLogout, cx);
    prepare_for_activation("account-a", cx);
    let account_a_cache_key = SaiCache::new(["account-switch-fixture"]).key;
    install_root(owner_root("account-a"), &account_a_cache_key, cx);
    prepare_for_activation("account-b", cx);
    let account_b_cache_key = SaiCache::new(["account-switch-fixture"]).key;

    assert_ne!(account_a_cache_key, account_b_cache_key);
    assert_eq!(current_payload(&account_b_cache_key, cx), None);
    assert!(disk_only(&account_a_cache_key, cx).is_some());
    hard_reset(AccountStateResetReason::ForcedLogout, cx);
}

#[gpui::test]
fn feed_reaction_and_repost_flags_stay_inside_their_account_partition(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    hard_reset(AccountStateResetReason::ForcedLogout, cx);

    prepare_for_activation("feed-account-a", cx);
    let account_a_key = PostServicesStore::feed_cache_key();
    install_root(feed_root("post-a", true, true), &account_a_key, cx);

    assert_feed_row(current_payload(&account_a_key, cx), "post-a", true, true);

    prepare_for_activation("feed-account-b", cx);
    let account_b_key = PostServicesStore::feed_cache_key();

    assert_ne!(account_a_key, account_b_key);
    assert!(feed_items(cx).is_empty());
    assert_eq!(current_payload(&account_b_key, cx), None);

    install_root(feed_root("post-b", false, false), &account_b_key, cx);

    assert_feed_row(current_payload(&account_b_key, cx), "post-b", false, false);

    prepare_for_activation("feed-account-a", cx);

    assert_eq!(PostServicesStore::feed_cache_key(), account_a_key);
    assert_feed_row(disk_only(&account_a_key, cx), "post-a", true, true);
    hard_reset(AccountStateResetReason::ForcedLogout, cx);
}

#[gpui::test]
fn logout_cleanup_removes_only_selected_account_disk_cache(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    hard_reset(AccountStateResetReason::ForcedLogout, cx);
    prepare_for_activation("account-a", cx);
    let account_a_key = SaiCache::new(["profile"]).key;
    install_root(owner_root("account-a"), &account_a_key, cx);
    let account_a_music = write_music_file("logout-private-track", cx);

    prepare_for_activation("account-b", cx);
    let account_b_key = SaiCache::new(["profile"]).key;
    install_root(owner_root("account-b"), &account_b_key, cx);
    let account_b_music = write_music_file("logout-private-track", cx);

    cx.update(|cx| AccountStateResetCoordinator::remove_persisted_cache("account-a", cx));
    cx.run_until_parked();

    assert_eq!(disk_only(&account_a_key, cx), None);
    assert!(disk_only(&account_b_key, cx).is_some());
    assert!(!account_a_music.exists());
    assert!(account_b_music.exists());
    MusicPlaybackDiskCache::remove_persisted_account("account-a");
    MusicPlaybackDiskCache::remove_persisted_account("account-b");
    hard_reset(AccountStateResetReason::ForcedLogout, cx);
}

#[gpui::test]
fn stale_wire_response_cannot_populate_new_account_state(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    hard_reset(AccountStateResetReason::ForcedLogout, cx);
    prepare_for_activation("account-a", cx);
    let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: "post".to_owned(),
        method: "get_feed".to_owned(),
        ..Default::default()
    });
    let slice = cx.update(SaiTypedSlice::<AccountResetFixtureDTO>::new);
    let pending_slice: Rc<dyn SaiSliceState> = slice.clone();
    let did_call_success = Rc::new(Cell::new(false));
    let success_flag = Rc::clone(&did_call_success);

    cx.update(|cx| {
        SaiWsPendingRegistry::shared(cx).update(cx, |registry, cx| {
            registry.register(
                9_001,
                Some(SaiWsPendingCacheBinding::new(&options)),
                None,
                Some(pending_slice),
                options.account_user_id.clone(),
                options.account_epoch.clone(),
                None,
                Some(Box::new(move |_: Option<Box<dyn Any>>, _: &mut App| success_flag.set(true))),
                None,
                cx,
            );
        });
    });

    SaiAccountCacheScope::activate("account-b");
    let mut frame = Map::new();
    frame.insert(rise_key::bot::REQUEST_ID.to_owned(), Value::from("9001"));
    frame.insert(wire_key::DATA.to_owned(), owner_root("account-a"));
    let handled = cx.update(|cx| {
        SaiWsPendingRegistry::shared(cx).update(cx, |registry, cx| registry.handle_incoming_json_object(&frame, cx))
    });

    assert!(handled);
    assert!(!did_call_success.get());
    assert_eq!(slice.status(), SaiWsSliceStatus::Idle);
    assert_eq!(slice.data(), None);
    hard_reset(AccountStateResetReason::ForcedLogout, cx);
}

#[gpui::test]
fn account_reset_cancels_debounced_mutations(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let did_fire = Rc::new(Cell::new(false));
    let fire_flag = Rc::clone(&did_fire);
    cx.update(|cx| {
        let _ = SaiDebouncer::debounced_action_delay_milliseconds(
            "post-1",
            Box::new(move |_: &mut App| fire_flag.set(true)),
            60_000,
            "post_toggle_like",
            SaiDebouncerOptions::default().max_cache_data,
            None,
            cx,
        );
    });

    assert_eq!(cx.read(|cx| SaiDebouncer::active_count("post_toggle_like", cx)), 1);

    hard_reset(AccountStateResetReason::AccountChanged, cx);

    assert_eq!(cx.read(|cx| SaiDebouncer::active_count("post_toggle_like", cx)), 0);
    assert!(!did_fire.get());
}

#[gpui::test]
fn account_reset_clears_private_transient_ui_and_playback_state(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    hard_reset(AccountStateResetReason::ForcedLogout, cx);

    let loader_cancellation_count = Rc::new(Cell::new(0));
    let cancellation_count = Rc::clone(&loader_cancellation_count);
    cx.update(|cx| {
        BotCreateFlowState::shared(cx).update(cx, |flow, _| {
            flow.name = "Old account bot".to_owned();
            flow.tag = "oldaccountbot".to_owned();
            flow.description = "private draft".to_owned();
            flow.can_join_groups = false;
            flow.privacy_mode = false;
            flow.tag_availability = BotTagAvailabilityState::Available;
            flow.one_time_token = Some("secret-token".to_owned());
            flow.token_presented = true;
        });

        MessageInteractionsStore::shared(cx).update(cx, |store, _| {
            store.last_typing_sent_by_chat.insert("private-chat".to_owned(), 42.0);
        });
        StoryInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.load_stories_state(&["old-account-user".to_owned()], cx));

        BlockingLoaderStore::shared(cx).update(cx, |store, cx| {
            store.show(move |_| cancellation_count.set(cancellation_count.get() + 1), cx);
        });

        ChatVoiceMessagePlayer::shared(cx).update(cx, |player, cx| {
            player.play(
                "old-account-voice".to_owned(),
                "private-chat".to_owned(),
                "Private".to_owned(),
                None,
                MessageMediaItem::new(
                    "old-account-voice-media".to_owned(),
                    MessageMediaKind::Voice,
                    "https://127.0.0.1:1/private-voice.ogg".to_owned(),
                ),
                cx,
            );
        });

        let voice_player = ChatVoiceMessagePlayer::shared(cx);
        assert_eq!(voice_player.read(cx).playing_message_id(), Some("old-account-voice"));
        assert!(voice_player.read(cx).is_loading());
        assert!(BlockingLoaderStore::shared(cx).read(cx).is_visible());
        assert_eq!(SaiDebouncer::active_count("story", cx), 1);
    });

    hard_reset(AccountStateResetReason::AccountChanged, cx);

    cx.update(|cx| {
        let flow = BotCreateFlowState::shared(cx);
        let flow = flow.read(cx);
        assert!(flow.name.is_empty());
        assert!(flow.tag.is_empty());
        assert!(flow.description.is_empty());
        assert!(flow.can_join_groups);
        assert!(flow.privacy_mode);
        assert_eq!(flow.tag_availability, BotTagAvailabilityState::Idle);
        assert!(flow.created_bot.is_none());
        assert_eq!(flow.one_time_token, None);
        assert!(!flow.token_presented);

        assert!(MessageInteractionsStore::shared(cx).read(cx).last_typing_sent_by_chat.is_empty());
        assert!(!BlockingLoaderStore::shared(cx).read(cx).is_visible());
        assert_eq!(SaiDebouncer::active_count("story", cx), 0);
        let voice_player = ChatVoiceMessagePlayer::shared(cx);
        assert_eq!(voice_player.read(cx).playing_message_id(), None);
        assert!(!voice_player.read(cx).is_playing());
        assert!(!voice_player.read(cx).is_loading());

        BlockingLoaderStore::shared(cx).update(cx, |store, cx| store.cancel(cx));
    });
    cx.run_until_parked();
    assert_eq!(loader_cancellation_count.get(), 0);
}

#[gpui::test]
async fn restore_skips_expired_selected_account_and_activates_valid_fallback(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let auth = shared_auth(cx);
    reset_refresh_account_fixtures(&auth, cx);

    let account_a_access = account_reset_jwt("restore-expired-a", hour_ago());
    let account_a_refresh = account_reset_jwt("restore-expired-a", hour_ago());
    let account_b_access = account_reset_jwt("restore-valid-b", hour_ahead());
    let account_b_refresh = account_reset_jwt("restore-valid-b", hour_ahead());
    persist_restore_account_fixture("restore-expired-a", &account_a_access, &account_a_refresh, 200);
    persist_restore_account_fixture("restore-valid-b", &account_b_access, &account_b_refresh, 100);
    AuthLocalStorage::set_active_account("restore-expired-a", 200);

    let restore = cx.update(|cx| AuthServicesStore::restore_persisted_session(auth.clone(), None, cx));
    restore.await;
    cx.run_until_parked();

    assert!(is_authenticated(&auth, cx));
    assert_eq!(active_account_id(&auth, cx).as_deref(), Some("restore-valid-b"));
    assert_eq!(access_token(&auth, cx), account_b_access);
    assert!(AuthLocalStorage::load_account("restore-expired-a").is_none());
    assert!(AuthLocalStorage::load_account("restore-valid-b").is_some());
    reset_refresh_account_fixtures(&auth, cx);
}

#[gpui::test]
async fn restore_removes_every_account_with_expired_access_and_refresh_tokens(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let auth = shared_auth(cx);
    reset_refresh_account_fixtures(&auth, cx);

    for (index, user_id) in ["restore-expired-a", "restore-expired-b", "restore-expired-c"].iter().enumerate() {
        persist_restore_account_fixture(
            user_id,
            &account_reset_jwt(user_id, hour_ago()),
            &account_reset_jwt(user_id, hour_ago()),
            300 - index as i64 * 100,
        );
    }
    AuthLocalStorage::set_active_account("restore-expired-a", 300);

    let restore = cx.update(|cx| AuthServicesStore::restore_persisted_session(auth.clone(), None, cx));
    restore.await;
    cx.run_until_parked();

    assert!(!is_authenticated(&auth, cx));
    assert_eq!(active_account_id(&auth, cx), None);
    assert!(cx.read(|cx| auth.read(cx).accounts().is_empty()));
    assert!(AuthLocalStorage::load_accounts().is_empty());
    reset_refresh_account_fixtures(&auth, cx);
}

#[gpui::test]
async fn restore_refreshes_fallback_after_removing_expired_selected_account(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let auth = shared_auth(cx);
    reset_refresh_account_fixtures(&auth, cx);

    let account_b_refresh = account_reset_jwt("restore-refresh-b", hour_ahead());
    let account_b_rotated_access = account_reset_jwt("restore-refresh-b", hour_ahead());
    let account_b_rotated_refresh = account_reset_jwt("restore-refresh-b", hour_ahead());
    persist_restore_account_fixture(
        "restore-expired-a",
        &account_reset_jwt("restore-expired-a", hour_ago()),
        &account_reset_jwt("restore-expired-a", hour_ago()),
        200,
    );
    persist_restore_account_fixture(
        "restore-refresh-b",
        &account_reset_jwt("restore-refresh-b", hour_ago()),
        &account_b_refresh,
        100,
    );
    AuthLocalStorage::set_active_account("restore-expired-a", 200);

    let expected_refresh = account_b_refresh.clone();
    let rotated_access = account_b_rotated_access.clone();
    let rotated_refresh = account_b_rotated_refresh.clone();
    let request: AuthTokenRefreshRequest = Rc::new(
        move |refresh_token: String, _: &mut App| -> Task<anyhow::Result<AuthRefreshResponse>> {
            assert_eq!(refresh_token, expected_refresh);
            Task::ready(Ok(auth_refresh_fixture(&rotated_access, &rotated_refresh, "session-restore-refresh-b-v2")))
        },
    );
    let restore = cx.update(|cx| AuthServicesStore::restore_persisted_session(auth.clone(), Some(request), cx));
    restore.await;
    cx.run_until_parked();

    assert!(is_authenticated(&auth, cx));
    assert_eq!(active_account_id(&auth, cx).as_deref(), Some("restore-refresh-b"));
    assert_eq!(access_token(&auth, cx), account_b_rotated_access);
    assert_eq!(refresh_token(&auth, cx), account_b_rotated_refresh);
    assert_eq!(stored_access_token("restore-refresh-b"), Some(account_b_rotated_access));
    assert_eq!(stored_refresh_token("restore-refresh-b"), Some(account_b_rotated_refresh));
    assert!(AuthLocalStorage::load_account("restore-expired-a").is_none());
    reset_refresh_account_fixtures(&auth, cx);
}

#[gpui::test]
async fn late_refresh_response_cannot_overwrite_new_active_account(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let auth = shared_auth(cx);
    install_refresh_account_fixtures(&auth, cx);

    let switching_auth = auth.clone();
    let request: AuthTokenRefreshRequest = Rc::new(
        move |refresh_token: String, cx: &mut App| -> Task<anyhow::Result<AuthRefreshResponse>> {
            assert_eq!(refresh_token, "refresh-a-v1");
            switching_auth.update(cx, |auth, cx| auth.switch_account("refresh-account-b", cx));
            Task::ready(Ok(auth_refresh_fixture("access-a-v2", "refresh-a-v2", "session-a")))
        },
    );
    let refresh = cx.update(|cx| auth.update(cx, |auth, cx| auth.perform_token_refresh(Some(request), cx)));
    refresh.await.expect("token refresh");
    cx.run_until_parked();

    assert_eq!(active_account_id(&auth, cx).as_deref(), Some("refresh-account-b"));
    assert_eq!(access_token(&auth, cx), "access-b-v1");
    assert_eq!(refresh_token(&auth, cx), "refresh-b-v1");
    assert_eq!(stored_access_token("refresh-account-a").as_deref(), Some("access-a-v2"));
    assert_eq!(stored_refresh_token("refresh-account-a").as_deref(), Some("refresh-a-v2"));
    assert_eq!(stored_access_token("refresh-account-b").as_deref(), Some("access-b-v1"));
    assert_eq!(stored_refresh_token("refresh-account-b").as_deref(), Some("refresh-b-v1"));
    reset_refresh_account_fixtures(&auth, cx);
}

#[gpui::test]
async fn refresh_dedup_starts_new_account_without_waiting_for_old_account(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let auth = shared_auth(cx);
    install_refresh_account_fixtures(&auth, cx);

    let account_b_request: AuthTokenRefreshRequest = Rc::new(
        move |refresh_token: String, _: &mut App| -> Task<anyhow::Result<AuthRefreshResponse>> {
            assert_eq!(refresh_token, "refresh-b-v1");
            Task::ready(Ok(auth_refresh_fixture("access-b-v2", "refresh-b-v2", "session-b")))
        },
    );
    let switching_auth = auth.clone();
    let account_a_request: AuthTokenRefreshRequest = Rc::new(
        move |refresh_token: String, cx: &mut App| -> Task<anyhow::Result<AuthRefreshResponse>> {
            assert_eq!(refresh_token, "refresh-a-v1");
            switching_auth.update(cx, |auth, cx| auth.switch_account("refresh-account-b", cx));
            let auth = switching_auth.clone();
            let account_b_request = Rc::clone(&account_b_request);
            cx.spawn(async move |cx| {
                let account_b_refresh =
                    cx.update(|cx| AuthTokenRefresh::refresh_access_token(Some(account_b_request), cx));
                account_b_refresh.await?;
                let access = cx.update(|cx| auth.read(cx).tokens_and_other_data().access_token.clone());
                assert_eq!(access, "access-b-v2");
                Ok(auth_refresh_fixture("access-a-v2", "refresh-a-v2", "session-a"))
            })
        },
    );
    let refresh = cx.update(|cx| AuthTokenRefresh::refresh_access_token(Some(account_a_request), cx));
    refresh.await.expect("token refresh");
    cx.run_until_parked();

    assert_eq!(active_account_id(&auth, cx).as_deref(), Some("refresh-account-b"));
    assert_eq!(access_token(&auth, cx), "access-b-v2");
    assert_eq!(refresh_token(&auth, cx), "refresh-b-v2");
    assert_eq!(stored_access_token("refresh-account-a").as_deref(), Some("access-a-v2"));
    assert_eq!(stored_refresh_token("refresh-account-a").as_deref(), Some("refresh-a-v2"));
    assert_eq!(stored_access_token("refresh-account-b").as_deref(), Some("access-b-v2"));
    assert_eq!(stored_refresh_token("refresh-account-b").as_deref(), Some("refresh-b-v2"));
    reset_refresh_account_fixtures(&auth, cx);
}

#[gpui::test]
async fn late_refresh_response_cannot_resurrect_removed_account(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let auth = shared_auth(cx);
    install_refresh_account_fixtures(&auth, cx);

    let clearing_auth = auth.clone();
    let request: AuthTokenRefreshRequest = Rc::new(
        move |refresh_token: String, cx: &mut App| -> Task<anyhow::Result<AuthRefreshResponse>> {
            assert_eq!(refresh_token, "refresh-a-v1");
            clearing_auth.update(cx, |auth, cx| auth.full_clear(AccountStateResetReason::UserLogout, cx));
            Task::ready(Ok(auth_refresh_fixture("access-a-v2", "refresh-a-v2", "session-a")))
        },
    );
    let refresh = cx.update(|cx| auth.update(cx, |auth, cx| auth.perform_token_refresh(Some(request), cx)));
    refresh.await.expect("token refresh");
    cx.run_until_parked();

    assert_eq!(active_account_id(&auth, cx).as_deref(), Some("refresh-account-b"));
    assert_eq!(access_token(&auth, cx), "access-b-v1");
    assert_eq!(refresh_token(&auth, cx), "refresh-b-v1");
    assert!(AuthLocalStorage::load_account("refresh-account-a").is_none());
    assert!(AuthLocalStorage::load_account("refresh-account-b").is_some());
    reset_refresh_account_fixtures(&auth, cx);
}

#[gpui::test]
async fn stale_auth_recovery_cannot_suppress_or_logout_new_account(cx: &mut TestAppContext) {
    let _serial = prepare_test(cx);
    let auth = shared_auth(cx);
    install_refresh_account_fixtures(&auth, cx);

    let account_b_recovery_count = Rc::new(Cell::new(0));
    let recovery_count = Rc::clone(&account_b_recovery_count);
    let switching_auth = auth.clone();
    let account_a_recovery: AuthSessionGuardRecovery =
        Box::new(move |cx: &mut App| -> Task<anyhow::Result<()>> {
            switching_auth.update(cx, |auth, cx| auth.switch_account("refresh-account-b", cx));
            cx.spawn(async move |cx| {
                let account_b_recovery: AuthSessionGuardRecovery =
                    Box::new(move |_: &mut App| -> Task<anyhow::Result<()>> {
                        recovery_count.set(recovery_count.get() + 1);
                        Task::ready(Ok(()))
                    });
                let account_b_task =
                    cx.update(|cx| AuthSessionGuard::handle_auth_error(Some(account_b_recovery), cx));
                if let Some(account_b_task) = account_b_task {
                    account_b_task.await;
                }
                Err(anyhow::anyhow!("user authentication required"))
            })
        });
    let account_a_task = cx.update(|cx| AuthSessionGuard::handle_auth_error(Some(account_a_recovery), cx));
    if let Some(account_a_task) = account_a_task {
        account_a_task.await;
    }
    cx.run_until_parked();

    assert_eq!(account_b_recovery_count.get(), 1);
    assert!(is_authenticated(&auth, cx));
    assert_eq!(active_account_id(&auth, cx).as_deref(), Some("refresh-account-b"));
    assert_eq!(access_token(&auth, cx), "access-b-v1");
    assert!(AuthLocalStorage::load_account("refresh-account-b").is_some());
    reset_refresh_account_fixtures(&auth, cx);
}
