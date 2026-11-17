use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::mem;
use std::rc::Rc;
use std::slice;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{
    App, AppContext as _, AsyncApp, Context, Entity, Global, IntoElement, ParentElement as _, RenderOnce, Styled as _,
    Task, WeakEntity, Window, black, div,
};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;

use rise_ui::screen_capture_protected_ui::ScreenCaptureProtectedUi;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::media_viewer::media_viewer_countdown_badge::MediaViewerCountdown;
use crate::core::media_viewer::media_viewer_presentation_state::MediaViewerPresentationState;
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerGalleryItemKind,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileMediaEntry, ChatProfileSource,
};
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind, MessageMediaViewLimit,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

pub enum ChatMediaGalleryBuilder {}

impl ChatMediaGalleryBuilder {
    /// generic over the element so the chat screen can pass its Arc snapshot without copying every message
    pub fn items_messages<M: Borrow<GetMessageMessage>>(
        chat: &ChatInfo,
        source_messages: Option<&[M]>,
        originating: &GetMessageMessage,
        tapped_index: usize,
        cx: &mut App,
    ) -> Vec<MediaViewerGalleryItem> {
        let tapped: Vec<&MessageMediaItem> =
            originating.grid_media_items().into_iter().filter(|item| !item.is_cancelled).collect();
        let tapped_is_animation = tapped.get(tapped_index).is_some_and(|item| Self::is_animation(item.kind));
        if tapped_is_animation {
            let mut out = Self::gallery_items(chat, slice::from_ref(originating), &originating.id, cx);
            out.retain(|item| item.kind == MediaViewerGalleryItemKind::Animation);
            return out;
        }
        let mut out = match source_messages {
            Some(messages) => Self::gallery_items(chat, messages, &originating.id, cx),
            None => Self::gallery_items(chat, slice::from_ref(originating), &originating.id, cx),
        };
        out.retain(|item| item.kind != MediaViewerGalleryItemKind::Animation);
        out
    }

    /// server names an erased sender with an english literal
    /// the chat own flag is what title row and list read so the viewer header says the same
    pub fn viewer_sender_name(name: &str, is_own: bool, chat: &ChatInfo) -> String {
        if chat.r#type != ChatType::Direct || !chat.participant.is_deleted || is_own {
            return name.to_owned();
        }
        tr("chat_deleted_account")
    }

    fn is_animation(kind: MessageMediaKind) -> bool {
        matches!(kind, MessageMediaKind::Animation | MessageMediaKind::Gif)
    }

    pub fn items_profile_entries(
        chat: &ChatInfo,
        profile_entries: &[ChatProfileMediaEntry],
        cx: &mut App,
    ) -> Vec<MediaViewerGalleryItem> {
        profile_entries.iter().map(|entry| Self::gallery_item_entry(chat, entry, cx)).collect()
    }

    pub fn open_index(items: &[MediaViewerGalleryItem], frame_registry_id: &str) -> Option<usize> {
        items.iter().position(|item| item.frame_registry_id == frame_registry_id)
    }

    fn gallery_items<M: Borrow<GetMessageMessage>>(
        chat: &ChatInfo,
        messages: &[M],
        opening_message_id: &str,
        cx: &mut App,
    ) -> Vec<MediaViewerGalleryItem> {
        let mut out = Vec::new();
        for message in messages {
            let message: &GetMessageMessage = message.borrow();
            let media: Vec<&MessageMediaItem> =
                message.grid_media_items().into_iter().filter(|item| !item.is_cancelled).collect();
            if media.is_empty() {
                continue;
            }
            for (index, item) in media.iter().enumerate() {
                if item.view_limit.is_some() && message.id != opening_message_id {
                    continue;
                }
                let Some(entry) = Self::gallery_item(chat, message, item, index, media.len(), cx) else {
                    continue;
                };
                out.push(entry);
            }
        }
        out
    }

    /// absent session reads as not the current user
    /// a viewer row must never start the session restore the store accessor would trigger
    pub(super) fn is_own(sender_id: &str, chat: &ChatInfo, cx: &App) -> bool {
        AuthServicesStore::try_read(cx).is_some_and(|store| store.is_current_user(sender_id))
            && chat.r#type != ChatType::Channel
    }

    /// the single place chat viewer rows are built
    /// the paged window rebuilds the same row
    /// so a second copy would diverge on hero key, caption or permissions
    pub fn gallery_item(
        chat: &ChatInfo,
        message: &GetMessageMessage,
        item: &MessageMediaItem,
        index_in_message: usize,
        media_count: usize,
        cx: &mut App,
    ) -> Option<MediaViewerGalleryItem> {
        let kind = match item.kind {
            MessageMediaKind::Image => MediaViewerGalleryItemKind::Image,
            MessageMediaKind::Video => MediaViewerGalleryItemKind::Video,
            MessageMediaKind::Animation | MessageMediaKind::Gif => MediaViewerGalleryItemKind::Animation,
            _ => return None,
        };
        let is_own = Self::is_own(&message.sender_id, chat, cx);
        let permissions = MessagePermissions::new(message, chat, is_own);
        let stable_render_id = item.stable_render_id(&message.id);
        let frame_registry_id = MediaViewerFrameKeys::chat_bubble(&stable_render_id);
        Some(MediaViewerGalleryItem {
            id: stable_render_id,
            kind,
            url: item.url.clone(),
            thumbnail_url: item.thumbnail_url.clone(),
            width: item.width,
            height: item.height,
            duration: item.duration,
            sender_id: message.sender_id.clone(),
            sender_name: Self::viewer_sender_name(&message.sender_name, is_own, chat),
            sender_logo: message.sender_logo.clone(),
            created_at: message.created_at,
            caption_title: String::new(),
            caption: message.text.clone(),
            message_id: message.id.clone(),
            index_in_message,
            media_item_id: item.id.clone(),
            uses_fit_letterbox: media_count == 1,
            frame_registry_id,
            is_own,
            can_delete: permissions.can_delete_for_me(cx),
            can_save: !permissions.protects_content() && item.view_limit.is_none(),
            can_forward: permissions.can_forward(cx) && item.view_limit.is_none(),
            can_reply: permissions.can_reply(cx),
            can_show_in_context: true,
            can_delete_for_me: permissions.can_delete_for_me(cx),
            can_delete_for_everyone: permissions.can_delete_own(cx)
                || permissions.can_delete_private_peer(cx)
                || permissions.can_moderate_others(cx),
            can_set_as_main: false,
            is_album_row: false,
            deletes_from_favourites: chat.r#type == ChatType::Favourites,
            display_source_override: Some(item.display_source()),
            persist_media_to_disk: item.view_limit.is_none(),
            post_download: None,
            image_cache_policy: if item.view_limit.is_none() {
                CleverImageCachePolicy::Standard
            } else {
                CleverImageCachePolicy::PrivateEphemeral
            },
        })
    }

    fn gallery_item_entry(chat: &ChatInfo, entry: &ChatProfileMediaEntry, cx: &mut App) -> MediaViewerGalleryItem {
        let item = &entry.item;
        let kind = match item.kind {
            MessageMediaKind::Image => MediaViewerGalleryItemKind::Image,
            MessageMediaKind::Video => MediaViewerGalleryItemKind::Video,
            MessageMediaKind::Animation | MessageMediaKind::Gif => MediaViewerGalleryItemKind::Animation,
            _ => MediaViewerGalleryItemKind::Image,
        };
        let is_own = Self::is_own(&entry.sender_id, chat, cx);
        let source_message = ChatProfileSource::forward_message(entry, &chat.id, cx);
        let permissions = MessagePermissions::new(&source_message, chat, is_own);
        MediaViewerGalleryItem {
            id: item.stable_render_id(&entry.message_id),
            kind,
            url: item.url.clone(),
            thumbnail_url: item.thumbnail_url.clone(),
            width: item.width,
            height: item.height,
            duration: item.duration,
            sender_id: entry.sender_id.clone(),
            sender_name: Self::viewer_sender_name(&entry.sender_name, is_own, chat),
            sender_logo: None,
            created_at: entry.created_at,
            caption_title: String::new(),
            caption: String::new(),
            message_id: entry.message_id.clone(),
            index_in_message: 0,
            media_item_id: item.id.clone(),
            uses_fit_letterbox: false,
            frame_registry_id: MediaViewerFrameKeys::profile_grid(&entry.id),
            is_own,
            can_delete: permissions.can_delete_for_me(cx),
            can_save: !permissions.protects_content(),
            can_forward: permissions.can_forward(cx),
            can_reply: permissions.can_reply(cx),
            can_show_in_context: true,
            can_delete_for_me: permissions.can_delete_for_me(cx),
            can_delete_for_everyone: permissions.can_delete_own(cx)
                || permissions.can_delete_private_peer(cx)
                || permissions.can_moderate_others(cx),
            can_set_as_main: false,
            is_album_row: false,
            deletes_from_favourites: chat.r#type == ChatType::Favourites,
            display_source_override: Some(item.display_source()),
            persist_media_to_disk: true,
            post_download: None,
            image_cache_policy: CleverImageCachePolicy::Standard,
        }
    }
}

/// keeps an open view once or timed file inside its rules
/// viewer closes when timer ends or app goes to background
/// desktop cant see captures, linux says unsupported
pub enum ChatViewLimitedViewerGuard {}

/// the one running watch, replacing it drops (cancels) the old one
#[derive(Default)]
struct ChatViewLimitedViewerGuardTask(Option<Task<()>>);

impl Global for ChatViewLimitedViewerGuardTask {}

struct ChatViewLimitedWatchEnd {
    cx: AsyncApp,
    presenter: WeakEntity<MediaViewerPresentationState>,
    chat_id: String,
    message_id: String,
    is_timed: bool,
    ring: Option<MediaViewerCountdown>,
    cover: Option<Entity<ChatViewLimitedPrivacyCover>>,
}

impl Drop for ChatViewLimitedWatchEnd {
    fn drop(&mut self) {
        let presenter = self.presenter.clone();
        let chat_id = mem::take(&mut self.chat_id);
        let message_id = mem::take(&mut self.message_id);
        let is_timed = self.is_timed;
        let ring = self.ring.take();
        let cover = self.cover.take();
        let cx = self.cx.clone();
        self.cx
            .foreground_executor()
            .spawn(async move {
                cx.update(|cx| {
                    if let Some(cover) = cover {
                        cover.update(cx, |cover, cx| cover.stop(cx));
                    }
                    if let Some(ring) = ring {
                        let _ = presenter.update(cx, |presenter, cx| {
                            if presenter.countdown() == Some(&ring) {
                                presenter.set_countdown(None, cx);
                            }
                        });
                    }
                    ChatViewLimitedOpenings::shared(cx).update(cx, |openings, cx| openings.finish(&message_id, cx));
                    if !is_timed {
                        MessageInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.mark_media_viewed(&chat_id, &message_id, true, cx))
                            .detach();
                    }
                });
            })
            .detach();
    }
}

impl ChatViewLimitedViewerGuard {
    const PRESENT_GRACE_MS: i64 = 15000;
    const TICK: Duration = Duration::from_millis(250);
    const CLOSING_WAIT_MS: i64 = 1500;

    /// call right before the viewer is asked to open the view limited file
    /// when the viewer is gone the media self destructs for this viewer and the row becomes a service line
    pub fn watch(
        presenter: &Entity<MediaViewerPresentationState>,
        limit: &MessageMediaViewLimit,
        chat_id: &str,
        message_id: &str,
        cx: &mut App,
    ) {
        let previous = cx.default_global::<ChatViewLimitedViewerGuardTask>().0.take();
        drop(previous);
        let previous_id = presenter.read(cx).presentation_id();
        let start_ms = Self::now_ms();
        let window_ms: Option<i64> =
            if limit.is_timed() { limit.seconds.map(|seconds| seconds * 1000) } else { None };
        let present_limit_ms = window_ms.unwrap_or(0) + Self::PRESENT_GRACE_MS;
        let presenter = presenter.clone();
        let limit = limit.clone();
        let chat_id = chat_id.to_owned();
        let message_id = message_id.to_owned();
        let task = cx.spawn(async move |cx: &mut AsyncApp| {
            let mut end = ChatViewLimitedWatchEnd {
                cx: cx.clone(),
                presenter: presenter.downgrade(),
                chat_id,
                message_id: message_id.clone(),
                is_timed: limit.is_timed(),
                ring: None,
                cover: None,
            };
            let mut presentation_id: Option<Uuid> = None;
            while presentation_id.is_none() {
                let found = presenter.read_with(cx, |presenter, _| {
                    presenter
                        .presentation_id()
                        .filter(|id| Some(*id) != previous_id && presenter.is_active_presentation(*id))
                });
                if found.is_some() {
                    presentation_id = found;
                } else if Self::now_ms() - start_ms > present_limit_ms {
                    return;
                } else {
                    cx.background_executor().timer(Self::TICK).await;
                }
            }
            let Some(presentation_id) = presentation_id else {
                return;
            };
            let deadline_ms: Option<i64> = window_ms.map(|window| Self::now_ms() + window);
            let ring = match (deadline_ms, limit.seconds) {
                (Some(deadline_ms), Some(seconds)) if seconds > 0 => Some(MediaViewerCountdown {
                    presentation_id,
                    deadline: Date::from_time_interval_since_1970(deadline_ms as f64 / 1000.0),
                    total: seconds as f64,
                }),
                _ => None,
            };
            end.ring = ring;
            let finish_id = message_id.clone();
            let cover = cx.update(|cx| {
                presenter.update(cx, |presenter, cx| {
                    presenter.set_closes_without_flyback(true, cx);
                    presenter.set_on_close_begin(
                        Some(Rc::new(move |cx: &mut App| {
                            ChatViewLimitedOpenings::shared(cx).update(cx, |openings, cx| openings.finish(&finish_id, cx));
                        })),
                        cx,
                    );
                    if let Some(ring) = ring {
                        presenter.set_countdown(Some(ring), cx);
                    }
                });
                cx.new(ChatViewLimitedPrivacyCover::new)
            });
            end.cover = Some(cover.clone());
            loop {
                if !presenter.read_with(cx, |presenter, _| presenter.is_active_presentation(presentation_id)) {
                    return;
                }
                let timed_out = deadline_ms.is_some_and(|deadline_ms| Self::now_ms() >= deadline_ms);
                let went_to_background = cover.read_with(cx, |cover, _| cover.went_to_background());
                if timed_out || went_to_background {
                    presenter.update(cx, |presenter, cx| presenter.request_animated_dismissal(cx));
                    let closing_since = Self::now_ms();
                    while presenter.read_with(cx, |presenter, _| presenter.is_active_presentation(presentation_id))
                        && Self::now_ms() - closing_since < Self::CLOSING_WAIT_MS
                    {
                        Self::presentation_change(&presenter, Self::TICK, cx).await;
                    }
                    return;
                }
                Self::presentation_change(&presenter, Self::TICK, cx).await;
            }
        });
        cx.default_global::<ChatViewLimitedViewerGuardTask>().0 = Some(task);
    }

    /// recipient already opened this view limited media
    /// server started their window, they are in played_by or opened it in this process
    /// the tile shows opened and a tap does nothing
    pub fn is_opened_by_viewer(
        message: &GetMessageMessage,
        limit: &MessageMediaViewLimit,
        viewer_id: &str,
        cx: &mut App,
    ) -> bool {
        limit.expires_at_ms.is_some()
            || (!viewer_id.is_empty() && message.played_by.iter().any(|id| id == viewer_id))
            || ChatViewLimitedOpenings::shared(cx).read(cx).contains(&message.id)
    }

    fn now_ms() -> i64 {
        (RiseClock::now().time_interval_since_1970() * 1000.0) as i64
    }

    /// returns once the viewer presentation changes or within passes, whichever is first
    /// a plain tick sleep kept the blurred tile up to a tick after the viewer closed
    async fn presentation_change(
        presenter: &Entity<MediaViewerPresentationState>,
        within: Duration,
        cx: &mut AsyncApp,
    ) {
        let wake = Rc::new(ChatViewLimitedWake::new());
        let on_change = wake.clone();
        let subscription = cx.update(|cx| cx.observe(presenter, move |_, _| on_change.fire()));
        let on_timer = wake.clone();
        let sleep = cx.background_executor().timer(within);
        let timer = cx.foreground_executor().spawn(async move {
            sleep.await;
            on_timer.fire();
        });
        wake.wait().await;
        drop(timer);
        drop(subscription);
    }
}

struct ChatViewLimitedWake {
    fired: Cell<bool>,
    continuation: RefCell<Option<oneshot::Sender<()>>>,
    receiver: RefCell<Option<oneshot::Receiver<()>>>,
}

impl ChatViewLimitedWake {
    fn new() -> Self {
        let (sender, receiver) = oneshot::channel();
        Self { fired: Cell::new(false), continuation: RefCell::new(Some(sender)), receiver: RefCell::new(Some(receiver)) }
    }

    async fn wait(&self) {
        if self.fired.get() {
            return;
        }
        let receiver = self.receiver.borrow_mut().take();
        if let Some(receiver) = receiver {
            let _ = receiver.await;
        }
    }

    fn fire(&self) {
        if self.fired.replace(true) {
            return;
        }
        if let Some(continuation) = self.continuation.borrow_mut().take() {
            let _ = continuation.send(());
        }
    }
}

/// view limited messages the recipient opened in this process
/// server window end reaches the row only on the next history read
/// so the bubble closes the tile from here
pub struct ChatViewLimitedOpenings {
    opened: HashSet<String>,
    closed: HashSet<String>,
    sender_expiries: HashMap<String, Task<()>>,
    sender_deadlines: HashMap<String, i64>,
}

impl ChatViewLimitedOpenings {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { opened: HashSet::new(), closed: HashSet::new(), sender_expiries: HashMap::new(), sender_deadlines: HashMap::new() }
    }

    pub fn opened(&self) -> &HashSet<String> {
        &self.opened
    }

    pub fn closed(&self) -> &HashSet<String> {
        &self.closed
    }

    pub fn contains(&self, message_id: &str) -> bool {
        self.opened.contains(message_id)
    }

    /// false when an open is already running or done, a second tap starts nothing
    pub fn begin(&mut self, message_id: &str, cx: &mut Context<Self>) -> bool {
        let inserted = self.opened.insert(message_id.to_owned());
        if inserted {
            cx.notify();
        }
        inserted
    }

    /// server refused the open, the tile may be tapped again
    pub fn cancel(&mut self, message_id: &str, cx: &mut Context<Self>) {
        if self.opened.remove(message_id) {
            cx.notify();
        }
    }

    pub fn finish(&mut self, message_id: &str, cx: &mut Context<Self>) {
        if !self.opened.contains(message_id) {
            return;
        }
        if self.closed.insert(message_id.to_owned()) {
            cx.notify();
        }
    }

    pub fn sender_deadline(&mut self, message_id: &str, server_ms: i64, seconds: i64) -> i64 {
        if let Some(known) = self.sender_deadlines.get(message_id) {
            return *known;
        }
        let now = Self::now_ms();
        let deadline = now + (server_ms - now).max(0).min(seconds * 1_000);
        self.sender_deadlines.insert(message_id.to_owned(), deadline);
        deadline
    }

    pub fn expire_sent(&mut self, message_id: &str, at_ms: i64, cx: &mut Context<Self>) {
        if self.closed.contains(message_id) || self.sender_expiries.contains_key(message_id) {
            return;
        }
        let wait_ms = at_ms - Self::now_ms();
        let id = message_id.to_owned();
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if wait_ms > 0 {
                cx.background_executor().timer(Duration::from_millis(wait_ms as u64)).await;
            }
            let _ = this.update(cx, |openings, cx| {
                openings.sender_expiries.remove(&id);
                if openings.closed.insert(id) {
                    cx.notify();
                }
            });
        });
        self.sender_expiries.insert(message_id.to_owned(), task);
    }

    fn now_ms() -> i64 {
        (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64
    }
}

/// strings read on the main thread and handed to the snapshot
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatViewLimitedDestructionStrings {
    pub photo_timed: String,
    pub photo_once: String,
    pub video_once: String,
}

impl ChatViewLimitedDestructionStrings {
    pub fn current() -> Self {
        Self {
            photo_timed: tr("media_view_destructed_photo_timed"),
            photo_once: tr("media_view_destructed_photo_once"),
            video_once: tr("media_view_destructed_video_once"),
        }
    }
}

/// self destruction of view once and timed media
/// after the recipient closes the viewer or sender learns it opened
/// bubble becomes a service line
pub enum ChatViewLimitedDestruction {}

impl ChatViewLimitedDestruction {
    pub fn label(
        message: &GetMessageMessage,
        viewer_user_id: Option<&str>,
        opened: &HashSet<String>,
        closed: &HashSet<String>,
        now: &Date,
        strings: &ChatViewLimitedDestructionStrings,
    ) -> Option<String> {
        if matches!(message.content_type, MessageContentType::System) {
            return None;
        }
        let media: Vec<&MessageMediaItem> =
            message.grid_media_items().into_iter().filter(|item| !item.is_cancelled).collect();
        let first = *media.first()?;
        let limit = first.view_limit.as_ref()?;
        if !media.iter().all(|item| item.view_limit.is_some()) {
            return None;
        }
        let viewer = viewer_user_id.unwrap_or_default();
        let is_own = !viewer.is_empty() && message.sender_id == viewer;
        let destructed = if media
            .iter()
            .any(|item| item.view_limit.as_ref().is_some_and(MessageMediaViewLimit::is_destroyed_on_server))
        {
            true
        } else if is_own && limit.is_timed() {
            closed.contains(&message.id)
                || limit
                    .sender_countdown_deadline_ms()
                    .is_some_and(|deadline| (now.time_interval_since_1970() * 1_000.0) as i64 >= deadline)
        } else if is_own {
            message.played_by.iter().any(|id| !id.is_empty() && id != viewer)
                || limit.state.as_deref() == Some(MessageMediaViewLimit::STATE_OPENED)
        } else if closed.contains(&message.id) {
            true
        } else if opened.contains(&message.id) {
            limit.is_timed() && limit.is_consumed(*now)
        } else {
            limit.expires_at_ms.is_some()
                || limit.is_consumed(*now)
                || (!viewer.is_empty() && message.played_by.iter().any(|id| id == viewer))
        };
        if !destructed {
            return None;
        }
        if matches!(first.kind, MessageMediaKind::Video) {
            return Some(strings.video_once.clone());
        }
        match limit.seconds {
            Some(seconds) if limit.is_timed() && seconds > 0 => {
                Some(strings.photo_timed.replace("{count}", &seconds.to_string()))
            }
            _ => Some(strings.photo_once.clone()),
        }
    }
}

#[derive(Clone, Copy)]
enum ChatViewLimitedPrivacySignal {
    WillResignActive,
    DidBecomeActive,
    DidEnterBackground,
}

/// how many privacy covers are up, root draws the overlay over every window while any is
#[derive(Default)]
struct ChatViewLimitedPrivacyCoverCount(usize);

impl Global for ChatViewLimitedPrivacyCoverCount {}

/// how many watches keep windows out of captures
/// root overlay claims the exclusion for its window while any does
#[derive(Default)]
struct ChatViewLimitedCaptureExclusionCount(usize);

impl Global for ChatViewLimitedCaptureExclusionCount {}

/// covers every window of the app while inactive
/// so a view limited file never reaches a window switcher preview
pub struct ChatViewLimitedPrivacyCover {
    observers: Vec<RiseSubscription>,
    signals: Option<Task<()>>,
    covers: bool,
    excludes_capture: bool,
    went_to_background: bool,
}

impl ChatViewLimitedPrivacyCover {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<ChatViewLimitedPrivacySignal>();
        let center = RiseAppEvents::default_center();
        let mut observers = Vec::new();
        for (name, signal) in [
            (UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION, ChatViewLimitedPrivacySignal::WillResignActive),
            (UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, ChatViewLimitedPrivacySignal::DidBecomeActive),
            (UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, ChatViewLimitedPrivacySignal::DidEnterBackground),
        ] {
            let sender = sender.clone();
            observers.push(center.observe(name, move |_| {
                let _ = sender.unbounded_send(signal);
            }));
        }
        let signals = cx.spawn(async move |this, cx| {
            while let Some(signal) = inbox.next().await {
                let updated = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| match signal {
                    ChatViewLimitedPrivacySignal::WillResignActive => this.show(cx),
                    ChatViewLimitedPrivacySignal::DidBecomeActive => this.hide(cx),
                    ChatViewLimitedPrivacySignal::DidEnterBackground => {
                        this.went_to_background = true;
                        cx.notify();
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        });
        let mut cover =
            Self { observers, signals: Some(signals), covers: false, excludes_capture: true, went_to_background: false };
        cx.default_global::<ChatViewLimitedCaptureExclusionCount>().0 += 1;
        cx.refresh_windows();
        if RiseAppLifecycle::scene_phase(cx) != ScenePhase::Active {
            cover.show(cx);
        }
        cover
    }

    pub fn went_to_background(&self) -> bool {
        self.went_to_background
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.observers.clear();
        self.signals = None;
        self.hide(cx);
        if self.excludes_capture {
            self.excludes_capture = false;
            let count = cx.default_global::<ChatViewLimitedCaptureExclusionCount>();
            count.0 = count.0.saturating_sub(1);
            cx.refresh_windows();
        }
    }

    pub fn is_covering(cx: &App) -> bool {
        cx.try_global::<ChatViewLimitedPrivacyCoverCount>().is_some_and(|count| count.0 > 0)
    }

    pub fn is_excluding_capture(cx: &App) -> bool {
        cx.try_global::<ChatViewLimitedCaptureExclusionCount>().is_some_and(|count| count.0 > 0)
    }

    fn show(&mut self, cx: &mut Context<Self>) {
        if self.covers {
            return;
        }
        self.covers = true;
        cx.default_global::<ChatViewLimitedPrivacyCoverCount>().0 += 1;
        cx.refresh_windows();
        cx.notify();
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        if !self.covers {
            return;
        }
        self.covers = false;
        let count = cx.default_global::<ChatViewLimitedPrivacyCoverCount>();
        count.0 = count.0.saturating_sub(1);
        cx.refresh_windows();
        cx.notify();
    }
}

/// blur view added to each window, drawn by root over everything and never hit tested
/// while a watch runs it also leaves its window out of screen captures
#[derive(IntoElement, Default)]
pub struct ChatViewLimitedPrivacyCoverOverlay;

impl ChatViewLimitedPrivacyCoverOverlay {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for ChatViewLimitedPrivacyCoverOverlay {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let cover = if ChatViewLimitedPrivacyCover::is_covering(cx) {
            div().absolute().inset_0().bg(black().opacity(0.85)).into_any_element()
        } else {
            div().into_any_element()
        };
        if !ChatViewLimitedPrivacyCover::is_excluding_capture(cx) {
            return cover;
        }
        div()
            .absolute()
            .inset_0()
            .child(cover)
            .child(ScreenCaptureProtectedUi::new("chat-view-limited-capture", true, div()))
            .into_any_element()
    }
}
