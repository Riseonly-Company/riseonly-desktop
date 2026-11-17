use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use gpui::{App, Context, Entity, Subscription};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::chat::components::chat::bubble::chat_audio_track_adapter::ChatAudioTrackAdapter;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileMediaEntry, ChatProfileSource,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::shared::chat_music_hooks::ChatMusicHooks;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::music::shared::utils::music::music_models::TrackModel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatAudioQueueContinuationOrder {
    Chronological,
    TabOrder,
}

struct Session {
    chat_id: String,
    scope_id: String,
    order: ChatAudioQueueContinuationOrder,
    account_id: Option<String>,
    account_epoch: Option<String>,
    base_tracks: Vec<TrackModel>,
    applied_ids: Vec<String>,
    entries_signature: Option<u64>,
    tracks: Vec<TrackModel>,
    requested_older: Option<(String, u64)>,
}

const MUSIC_TAB_CACHE_ID: &str = rise_canon::rise_wire_methods::message::GET_MUSIC_MESSAGES;
const OLDER_EDGE_DISTANCE: usize = 2;

/// grows a chat music queue from the tracks at hand to the whole music list of the chat
/// list comes from the music tab pages and the queue is swapped while the same track plays
pub struct ChatAudioQueueContinuation {
    session: Option<Session>,
    generation: u64,
    sync_scheduled: bool,
    subscriptions: Vec<Subscription>,
}

impl ChatAudioQueueContinuation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            session: None,
            generation: 0,
            sync_scheduled: false,
            subscriptions: Vec::new(),
        }
    }

    pub fn begin(&mut self, chat_id: &str, order: ChatAudioQueueContinuationOrder, cx: &mut Context<Self>) {
        let chat = trimming_whitespaces_and_newlines(chat_id).to_owned();
        let scope_id = format!("chat:{chat}");
        self.generation = self.generation.wrapping_add(1);
        self.subscriptions.clear();
        if chat.is_empty() || ChatMusicHooks::playback_scope_id(cx).as_deref() != Some(scope_id.as_str()) {
            self.session = None;
            return;
        }
        let base_tracks = ChatMusicHooks::source_queue(cx);
        let applied_ids = base_tracks.iter().map(|track| track.id.clone()).collect();
        self.session = Some(Session {
            chat_id: chat.clone(),
            scope_id,
            order,
            account_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            base_tracks,
            applied_ids,
            entries_signature: None,
            tracks: Vec::new(),
            requested_older: None,
        });
        let expected = self.generation;
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            MessageInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.load_initial_music_tab(&chat, None, false, cx));
            let _ = weak.update(cx, |this, cx| this.sync(expected, cx));
        });
    }

    fn end(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.session = None;
        self.subscriptions.clear();
    }

    fn sync(&mut self, expected: u64, cx: &mut Context<Self>) {
        if expected != self.generation {
            return;
        }
        let Some(mut session) = self.session.take() else {
            return;
        };
        let source_ids: Vec<String> = ChatMusicHooks::source_queue(cx).iter().map(|track| track.id.clone()).collect();
        let still_ours = ChatMusicHooks::playback_scope_id(cx).as_deref() == Some(session.scope_id.as_str())
            && !ChatMusicHooks::has_listening_session(cx)
            && SaiAccountCacheScope::active_user_id() == session.account_id
            && SaiAccountCacheScope::active_epoch() == session.account_epoch
            && source_ids == session.applied_ids;
        let current = ChatMusicHooks::current_track(cx);
        let (true, Some(current)) = (still_ours, current) else {
            self.end();
            return;
        };

        let entries = ChatProfileSource::audio_items(&session.chat_id, cx);
        let signature = Self::signature(&entries);
        if session.entries_signature != Some(signature) {
            session.entries_signature = Some(signature);
            let listed: Vec<TrackModel> = Self::ordered(&entries, session.order)
                .into_iter()
                .map(|entry| {
                    ChatAudioTrackAdapter::track(
                        &entry.message_id,
                        Some(entry.message_id.as_str()),
                        &session.chat_id,
                        &entry.item,
                        "",
                    )
                })
                .collect();
            session.tracks = match session.order {
                ChatAudioQueueContinuationOrder::Chronological => Self::merged(listed, &session.base_tracks),
                ChatAudioQueueContinuationOrder::TabOrder => listed,
            };
        }

        let (page, tab_revision) = MessageServicesStore::shared(cx).update(cx, |store, cx| {
            (
                store.tab_pagination(MUSIC_TAB_CACHE_ID, &session.chat_id, cx),
                store.message_tab_presentation_revision(MUSIC_TAB_CACHE_ID, &session.chat_id, cx),
            )
        });
        let anchor = session
            .tracks
            .iter()
            .position(|track| ChatAudioTrackAdapter::is_same_audio(Some(&current), track));
        let loads_older_page = match anchor {
            Some(anchor) => {
                let mut tracks = session.tracks.clone();
                tracks[anchor] = current;
                let near_older_edge = match session.order {
                    ChatAudioQueueContinuationOrder::Chronological => anchor <= OLDER_EDGE_DISTANCE,
                    ChatAudioQueueContinuationOrder::TabOrder => anchor + 1 + OLDER_EDGE_DISTANCE >= tracks.len(),
                };
                let ids: Vec<String> = tracks.iter().map(|track| track.id.clone()).collect();
                if ids != session.applied_ids && ChatMusicHooks::replace_ad_hoc_queue(&session.scope_id, tracks, cx) {
                    session.applied_ids = ids;
                }
                near_older_edge && page.has_more
            }
            None => page.has_more,
        };
        let older_request = match (&page.relative_id, page.has_more) {
            (Some(cursor), true) => Some((cursor.clone(), tab_revision)),
            _ => None,
        };
        let fills_whole_chat = session.order == ChatAudioQueueContinuationOrder::Chronological
            && older_request.is_some()
            && older_request != session.requested_older;
        if fills_whole_chat {
            session.requested_older = older_request;
            tracing::warn!(
                target: "riseonly::X1964",
                "chat queue older page chat={} queued={} cursor={:?} revision={}",
                session.chat_id,
                session.tracks.len(),
                page.relative_id,
                tab_revision
            );
        }
        if loads_older_page || fills_whole_chat {
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_music_tab(&session.chat_id, cx));
        }

        self.session = Some(session);
        self.observe(expected, cx);
    }

    fn observe(&mut self, expected: u64, cx: &mut Context<Self>) {
        if !self.subscriptions.is_empty() {
            return;
        }
        let services = MessageServicesStore::shared(cx);
        self.subscriptions
            .push(cx.observe(&services, move |this, _, cx| this.schedule_sync(expected, cx)));
        let presentation = RiseConversationPresentation::shared(cx);
        self.subscriptions
            .push(cx.observe(&presentation, move |this, _, cx| this.schedule_sync(expected, cx)));
        let weak = cx.weak_entity();
        let playback = ChatMusicHooks::observe_playback(
            Box::new(move |cx: &mut App| {
                let _ = weak.update(cx, |this, cx| this.schedule_sync(expected, cx));
            }),
            cx,
        );
        self.subscriptions.extend(playback);
    }

    fn schedule_sync(&mut self, expected: u64, cx: &mut Context<Self>) {
        if self.sync_scheduled {
            return;
        }
        self.sync_scheduled = true;
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = weak.update(cx, |this, cx| {
                this.sync_scheduled = false;
                this.sync(expected, cx);
            });
        });
    }

    fn signature(entries: &[ChatProfileMediaEntry]) -> u64 {
        let mut hasher = DefaultHasher::new();
        entries.len().hash(&mut hasher);
        for entry in entries {
            entry.id.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// music tab is cache first and can miss mp3s just sent
    /// so the loaded run stays whole and the tab only adds before and after it
    fn merged(listed: Vec<TrackModel>, base: &[TrackModel]) -> Vec<TrackModel> {
        let matches_base =
            |track: &TrackModel| base.iter().any(|kept| ChatAudioTrackAdapter::is_same_audio(Some(kept), track));
        let (Some(first), Some(last)) = (listed.iter().position(matches_base), listed.iter().rposition(matches_base))
        else {
            return listed;
        };
        let mut merged = Vec::with_capacity(first + base.len() + listed.len() - last - 1);
        merged.extend_from_slice(&listed[..first]);
        merged.extend_from_slice(base);
        merged.extend_from_slice(&listed[last + 1..]);
        merged
    }

    fn ordered(
        entries: &[ChatProfileMediaEntry],
        order: ChatAudioQueueContinuationOrder,
    ) -> Vec<&ChatProfileMediaEntry> {
        match order {
            ChatAudioQueueContinuationOrder::TabOrder => entries.iter().collect(),
            ChatAudioQueueContinuationOrder::Chronological => {
                let mut indexed: Vec<(usize, &ChatProfileMediaEntry)> = entries.iter().enumerate().collect();
                indexed.sort_by(|left, right| {
                    left.1
                        .created_at
                        .partial_cmp(&right.1.created_at)
                        .unwrap_or(Ordering::Equal)
                        .then(right.0.cmp(&left.0))
                });
                indexed.into_iter().map(|(_, entry)| entry).collect()
            }
        }
    }
}
