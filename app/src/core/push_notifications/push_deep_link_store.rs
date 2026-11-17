use std::time::Duration;

use gpui::{App, Context, Entity};
use rise_canon::rise_key::guide_screen::camel as guide_screen_keys;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{
    SaiActiveScreen, SaiNavigationBridge,
};

const CHAT_SCREEN_ID: &str = "Chat";
const PINNED_MESSAGES_SCREEN_ID: &str = "PinnedMessages";
const TOPIC_MODE_PARAM: &str = "topicMode";
const TOPIC_MODE_ALL: &str = "all";
const COMPOSER_FOCUS_GRACE: Duration = Duration::from_secs(3);

/// conversation a toast without a reply field sent the person to, to type there
#[derive(Clone, Debug, PartialEq, Eq)]
struct ChatComposerFocusRequest {
    chat_id: String,
    topic_id: Option<String>,
}

impl ChatComposerFocusRequest {
    fn for_link(link: &PushDeepLink) -> Option<Self> {
        let (chat_id, topic_id) = match link {
            PushDeepLink::Chat { chat_id } => (chat_id, None),
            PushDeepLink::ChatTopic { chat_id, topic_id } => (chat_id, Some(topic_id)),
            PushDeepLink::ChatMessage { chat_id, topic_id, .. } => (chat_id, topic_id.as_ref()),
            _ => return None,
        };
        Some(Self {
            chat_id: chat_id.clone(),
            topic_id: topic_id.filter(|topic_id| !topic_id.is_empty()).cloned(),
        })
    }

    fn is_for_screen(&self, chat_ids: &[&str], topic_id: Option<&str>, shows_all_topics: bool) -> bool {
        chat_ids.contains(&self.chat_id.as_str())
            && (shows_all_topics
                || topic_id.filter(|topic_id| !topic_id.is_empty()) == self.topic_id.as_deref())
    }
}

pub struct PushDeepLinkStore {
    pending_route: Option<PushDeepLink>,
    composer_focus_request: Option<ChatComposerFocusRequest>,
    composer_focus_deadline: Option<Duration>,
}

impl PushDeepLinkStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { pending_route: None, composer_focus_request: None, composer_focus_deadline: None }
    }

    pub fn pending_route(&self) -> Option<&PushDeepLink> {
        self.pending_route.as_ref()
    }

    pub fn enqueue(&mut self, link: PushDeepLink, cx: &mut Context<Self>) -> bool {
        let active_screen = SaiNavigationBridge::current_screen(cx);
        self.enqueue_active_screen(link, active_screen.as_ref(), cx)
    }

    pub fn enqueue_active_screen(
        &mut self,
        link: PushDeepLink,
        active_screen: Option<&SaiActiveScreen>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.composer_focus_request = None;
        self.composer_focus_deadline = None;
        if link.matches(active_screen) {
            if self.pending_route.as_ref() == Some(&link) {
                self.pending_route = None;
                cx.notify();
            }
            return false;
        }

        self.pending_route = Some(link);
        cx.notify();
        true
    }

    pub fn enqueue_for_composer(&mut self, link: PushDeepLink, cx: &mut Context<Self>) -> bool {
        let active_screen = SaiNavigationBridge::current_screen(cx);
        self.enqueue_for_composer_active_screen(link, active_screen.as_ref(), cx)
    }

    pub fn enqueue_for_composer_active_screen(
        &mut self,
        link: PushDeepLink,
        active_screen: Option<&SaiActiveScreen>,
        cx: &mut Context<Self>,
    ) -> bool {
        let queued = self.enqueue_active_screen(link.clone(), active_screen, cx);
        self.composer_focus_request = ChatComposerFocusRequest::for_link(&link);
        self.composer_focus_deadline = (!queued).then(|| RiseClock::monotonic() + COMPOSER_FOCUS_GRACE);
        cx.notify();
        queued
    }

    /// one shot, true once for the shown chat screen the reply action named
    /// refused while the route is queued since the shown chat is the one being left
    pub fn take_composer_focus(
        &mut self,
        chat_ids: &[&str],
        topic_id: Option<&str>,
        shows_all_topics: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(request) = &self.composer_focus_request else {
            return false;
        };
        if self.pending_route.is_some() {
            return false;
        }
        let expired = self.composer_focus_deadline.is_some_and(|deadline| RiseClock::monotonic() > deadline);
        if !expired && !request.is_for_screen(chat_ids, topic_id, shows_all_topics) {
            return false;
        }
        self.composer_focus_request = None;
        self.composer_focus_deadline = None;
        cx.notify();
        !expired
    }

    pub fn consume(&mut self, cx: &mut Context<Self>) {
        self.pending_route = None;
        if self.composer_focus_request.is_some() && self.composer_focus_deadline.is_none() {
            self.composer_focus_deadline = Some(RiseClock::monotonic() + COMPOSER_FOCUS_GRACE);
        }
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        if SaiAccountCacheScope::active_user_id().is_none() {
            return;
        }
        self.composer_focus_request = None;
        self.composer_focus_deadline = None;
        self.consume(cx);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PushDeepLink {
    Chat { chat_id: String },
    ChatTopic { chat_id: String, topic_id: String },
    ChatMessage { chat_id: String, message_id: String, topic_id: Option<String> },
    BotMiniApp { tag: String, slug: String, start_param: Option<String> },
    BotChat { tag: String, start_param: Option<String> },
    Post { post_id: String },
    PostComment { post_id: String, comment_id: Option<String> },
    Profile { user_id: Option<String>, tag: Option<String> },
    PublicTag { tag: String },
    ContactJoinInbox,
    ContactPhotoSuggestions,
    Story { story_id: String },
    Highlight { highlight_id: String },
    Music,
    MusicTrack { track_id: String },
    PlaylistTrack { playlist_id: String, track_id: String },
    MusicReleaseTrack { track_id: String },
    MusicAlbum { album_id: String },
    JoinChat { token: String },
    MapInvite { token: String },
    Map { user_id: Option<String> },
    PlaylistInvite { token: String },
    Playlist { playlist_id: String },
    PlaylistJoinRequests { playlist_id: String },
    PlaylistSession { playlist_id: String },
    StickerPackJoinRequests { pack_id: String },
    StickerPackDetail { pack_id: String, pack_kind: Option<String> },
    AddStickerPack { link: String },
    StickerPackInvite { token: String },
    AddTheme { theme_id: String },
    WebLogin { login_token: String },
    ManagedLogin { code: String },
    PremiumPromo { code: String },
    Announcement { announcement_id: String },
    ModerationReport { report_id: String },
    ModerationSanction { report_id: String },
    AppealVerdict { appeal_id: String },
    StudioTrack { track_id: String },
    MyAlbumRequests,
    MyArtistRequest,
    BugReport { report_id: String },
    Feedback { report_id: String },
    SparksWallet,
    AdminConsole { url: Url },
}

impl PushDeepLink {
    pub fn matches(&self, active_screen: Option<&SaiActiveScreen>) -> bool {
        let Some(active_screen) = active_screen else {
            return false;
        };
        if active_screen.id != CHAT_SCREEN_ID {
            return false;
        }

        match self {
            Self::Chat { chat_id } => {
                if !Self::matches_chat_id(chat_id, active_screen) {
                    return false;
                }
                if Self::topic_mode_is_all(active_screen) {
                    return true;
                }
                Self::normalized(active_screen.params.get(guide_screen_keys::TOPIC_ID)).is_none()
                    && Self::normalized(active_screen.params.get(guide_screen_keys::THREAD_ROOT_ID))
                        .is_none()
            }

            Self::ChatTopic { chat_id, topic_id } => {
                if !Self::matches_chat_id(chat_id, active_screen) {
                    return false;
                }
                let Some(expected_topic_id) = Self::normalized(Some(topic_id)) else {
                    return false;
                };
                if Self::topic_mode_is_all(active_screen) {
                    return true;
                }
                Self::normalized(active_screen.params.get(guide_screen_keys::TOPIC_ID))
                    == Some(expected_topic_id)
                    && Self::normalized(active_screen.params.get(guide_screen_keys::THREAD_ROOT_ID))
                        .is_none()
            }

            _ => false,
        }
    }

    /// pinned messages and the viewer above them still
    /// show the open chat so banners stay quiet there too
    /// navigation keeps matches so a tap from pinned still opens the chat
    pub fn silences_banner(&self, active_screen: Option<&SaiActiveScreen>) -> bool {
        if self.matches(active_screen) {
            return true;
        }
        let Some(active_screen) = active_screen else {
            return false;
        };
        if active_screen.id != PINNED_MESSAGES_SCREEN_ID {
            return false;
        }
        let chat_screen =
            SaiActiveScreen::new(CHAT_SCREEN_ID.to_owned(), active_screen.params.clone());
        self.matches(Some(&chat_screen))
    }

    fn topic_mode_is_all(active_screen: &SaiActiveScreen) -> bool {
        active_screen.params.get(TOPIC_MODE_PARAM).map(String::as_str) == Some(TOPIC_MODE_ALL)
    }

    fn matches_chat_id(chat_id: &str, active_screen: &SaiActiveScreen) -> bool {
        let Some(expected_chat_id) = Self::normalized(Some(chat_id)) else {
            return false;
        };
        Self::normalized(active_screen.params.get(guide_screen_keys::CHAT_ID))
            == Some(expected_chat_id)
    }

    fn normalized<S: AsRef<str> + ?Sized>(value: Option<&S>) -> Option<&str> {
        let result = trimming_whitespaces_and_newlines(value?.as_ref());
        if result.is_empty() { None } else { Some(result) }
    }
}
