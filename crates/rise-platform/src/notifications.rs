//! System notifications raised locally from socket events.
//!
//! The policy fails open at every step — an unrecognised scope, an absent scope
//! set or a missing mute state all present — so an event kind the server adds
//! after this build ships still reaches the user.
//!
//! Two deviations from a phone: the visible screen is a *set* of route keys,
//! and losing window focus suppresses nothing.

use serde::Deserialize;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// The wire's `notification_scope`: which class of chat event a notification
/// belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NotificationScope {
    Messages,
    Mentions,
    Reactions,
    Events,
}

impl NotificationScope {
    /// Trimmed and case-folded: the value arrives as free text from the server.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "messages" => Some(Self::Messages),
            "mentions" => Some(Self::Mentions),
            "reactions" => Some(Self::Reactions),
            "events" => Some(Self::Events),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Messages => "messages",
            Self::Mentions => "mentions",
            Self::Reactions => "reactions",
            Self::Events => "events",
        }
    }

    pub const fn is_enabled_in(self, scopes: &MuteScopes) -> bool {
        match self {
            Self::Messages => scopes.messages,
            Self::Mentions => scopes.mentions,
            Self::Reactions => scopes.reactions,
            Self::Events => scopes.events,
        }
    }
}

/// A conversation's per-scope notification switches, as stored in the
/// `mute_scopes` JSON column. Every field defaults to enabled, so a value built
/// with `..Default::default()` un-mutes rather than mutes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Deserialize)]
pub struct MuteScopes {
    pub messages: bool,
    pub mentions: bool,
    pub reactions: bool,
    pub events: bool,
}

impl MuteScopes {
    pub const ALL_ENABLED: Self = Self {
        messages: true,
        mentions: true,
        reactions: true,
        events: true,
    };

    pub const MUTED: Self = Self {
        messages: false,
        mentions: false,
        reactions: false,
        events: false,
    };

    pub const fn is_all_enabled(&self) -> bool {
        self.messages && self.mentions && self.reactions && self.events
    }

    pub const fn is_fully_muted(&self) -> bool {
        !(self.messages || self.mentions || self.reactions || self.events)
    }

    /// All four keys or nothing: half-applying a partial payload would silently
    /// mute scopes the server never spoke about.
    pub fn parse(raw: &str) -> Option<Self> {
        serde_json::from_str(raw).ok()
    }

    /// The column as the chat row carries it; anything unparseable means all on.
    pub fn from_stored(raw: Option<&str>) -> Self {
        raw.and_then(Self::parse).unwrap_or(Self::ALL_ENABLED)
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"messages\":{},\"mentions\":{},\"reactions\":{},\"events\":{}}}",
            self.messages, self.mentions, self.reactions, self.events
        )
    }
}

impl Default for MuteScopes {
    fn default() -> Self {
        Self::ALL_ENABLED
    }
}

/// Fails open twice over: a scope this build does not recognise is one the
/// server added later, and an absent scope set means never configured — which
/// is not the same as configured to silence.
pub fn scope_allows(scope: Option<&str>, scopes: Option<&MuteScopes>) -> bool {
    let (Some(raw), Some(scopes)) = (scope, scopes) else {
        return true;
    };

    match NotificationScope::parse(raw) {
        Some(scope) => scope.is_enabled_in(scopes),
        None => true,
    }
}

/// Above this a timestamp must be milliseconds: as seconds it would be 33658.
const MILLISECOND_FLOOR: i64 = 1_000_000_000_000;

/// `mute_until` arrives in seconds on some paths and milliseconds on others,
/// and nothing on the wire says which; magnitude disambiguates, and the result
/// is always milliseconds.
pub const fn mute_until_millis(value: i64) -> i64 {
    if value > MILLISECOND_FLOOR {
        value
    } else {
        // A corrupt row near i64::MIN would overflow; saturating keeps it past.
        value.saturating_mul(1_000)
    }
}

pub const fn mute_is_active(mute_until: Option<i64>, now_ms: i64) -> bool {
    match mute_until {
        Some(value) => mute_until_millis(value) > now_ms,
        None => false,
    }
}

/// What the caller knows about the conversation this notification belongs to.
/// Both fields absent is the ordinary shape for a notification that is not
/// about a conversation at all, and it presents.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MuteState {
    pub mute_until: Option<i64>,
    pub scopes: Option<MuteScopes>,
}

impl MuteState {
    /// Nothing known about this conversation, which presents.
    pub const NONE: Self = Self {
        mute_until: None,
        scopes: None,
    };

    pub const fn muted_until(mute_until: i64) -> Self {
        Self {
            mute_until: Some(mute_until),
            scopes: None,
        }
    }

    pub const fn with_scopes(scopes: MuteScopes) -> Self {
        Self {
            mute_until: None,
            scopes: Some(scopes),
        }
    }
}

/// One socket event, reduced to the three things the policy looks at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotificationRequest<'a> {
    /// The route this notification would open, in the caller's own vocabulary.
    /// Compared against the visible set by equality and never parsed.
    pub route_key: &'a str,
    /// The wire's `notification_scope`, unparsed. Unknown and absent both pass.
    pub scope: Option<&'a str>,
    /// Who caused the event.
    pub actor_id: Option<&'a str>,
}

impl<'a> NotificationRequest<'a> {
    pub const fn new(route_key: &'a str) -> Self {
        Self {
            route_key,
            scope: None,
            actor_id: None,
        }
    }

    pub const fn with_scope(mut self, scope: &'a str) -> Self {
        self.scope = Some(scope);
        self
    }

    pub const fn with_actor(mut self, actor_id: &'a str) -> Self {
        self.actor_id = Some(actor_id);
        self
    }
}

/// Everything about the running client that can change the answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PresentationContext<'a> {
    /// Every route the window is currently displaying — a set, because a window
    /// shows several at once.
    pub visible_route_keys: &'a [String],
    /// Whether the window has keyboard focus. On its own this suppresses
    /// nothing — see [`decide`].
    pub window_focused: bool,
    pub current_user_id: Option<&'a str>,
    pub mute: MuteState,
    pub now_ms: i64,
}

const NO_VISIBLE_ROUTES: &[String] = &[];

impl<'a> PresentationContext<'a> {
    pub const fn new(now_ms: i64) -> Self {
        Self {
            visible_route_keys: NO_VISIBLE_ROUTES,
            window_focused: false,
            current_user_id: None,
            mute: MuteState::NONE,
            now_ms,
        }
    }

    pub const fn showing(mut self, visible_route_keys: &'a [String]) -> Self {
        self.visible_route_keys = visible_route_keys;
        self
    }

    pub const fn with_focus(mut self, window_focused: bool) -> Self {
        self.window_focused = window_focused;
        self
    }

    pub const fn as_user(mut self, current_user_id: &'a str) -> Self {
        self.current_user_id = Some(current_user_id);
        self
    }

    pub const fn with_mute(mut self, mute: MuteState) -> Self {
        self.mute = mute;
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotificationDecision {
    Present,
    Suppressed(SuppressionReason),
}

impl NotificationDecision {
    pub const fn is_present(self) -> bool {
        matches!(self, Self::Present)
    }

    pub const fn reason(self) -> Option<SuppressionReason> {
        match self {
            Self::Present => None,
            Self::Suppressed(reason) => Some(reason),
        }
    }
}

/// Why nothing appeared.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SuppressionReason {
    /// The event was caused by this account.
    SelfAuthored,
    /// Inside the conversation's mute window.
    ConversationMuted,
    /// The conversation has this class of event switched off.
    ScopeDisabled,
    /// The route is on screen in a focused window.
    AlreadyOnScreen,
}

/// The whole policy, as a pure function.
///
/// The order is part of the contract: self-authored first, then the user's
/// standing preferences, then the window — so when two reasons apply the
/// durable one is reported. Focus alone suppresses nothing; only a focused
/// window showing exactly this route does.
pub fn decide(
    request: &NotificationRequest<'_>,
    context: &PresentationContext<'_>,
) -> NotificationDecision {
    if is_self_authored(request.actor_id, context.current_user_id) {
        return NotificationDecision::Suppressed(SuppressionReason::SelfAuthored);
    }

    if mute_is_active(context.mute.mute_until, context.now_ms) {
        return NotificationDecision::Suppressed(SuppressionReason::ConversationMuted);
    }

    if !scope_allows(request.scope, context.mute.scopes.as_ref()) {
        return NotificationDecision::Suppressed(SuppressionReason::ScopeDisabled);
    }

    if context.window_focused && is_visible(context.visible_route_keys, request.route_key) {
        return NotificationDecision::Suppressed(SuppressionReason::AlreadyOnScreen);
    }

    NotificationDecision::Present
}

/// Every session of an account receives that account's own events, so a like
/// sent from the phone arrives here. Ids are trimmed and an empty one is
/// nobody: two blank ids must not compare equal and silence a real event.
fn is_self_authored(actor_id: Option<&str>, current_user_id: Option<&str>) -> bool {
    let (Some(actor), Some(viewer)) = (actor_id, current_user_id) else {
        return false;
    };

    let actor = actor.trim();
    !actor.is_empty() && actor == viewer.trim()
}

fn is_visible(visible_route_keys: &[String], route_key: &str) -> bool {
    visible_route_keys
        .iter()
        .any(|visible| visible == route_key)
}

const CONVERSATION_TAG_PREFIX: &str = "conv:";
const UNIQUE_TAG_PREFIX: &str = "one:";

/// The identity gpui replaces on: posting a notification whose tag matches a
/// live one replaces it. The two constructors are namespaced against each
/// other, so a route key and an event id can be the same string.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NotificationTag(String);

impl NotificationTag {
    /// One tag per conversation: the newest message replaces the previous one.
    pub fn conversation(route_key: &str) -> Self {
        Self(format!("{CONVERSATION_TAG_PREFIX}{route_key}"))
    }

    /// One banner per key, for notifications that must not swallow each other.
    /// The caller supplies something already unique, usually the event id.
    pub fn unique(key: &str) -> Self {
        Self(format!("{UNIQUE_TAG_PREFIX}{key}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Closes the loop for a click: gpui hands the tag back in the response,
    /// and a conversation tag still carries the route it was built from.
    pub fn conversation_route(&self) -> Option<&str> {
        self.0.strip_prefix(CONVERSATION_TAG_PREFIX)
    }
}

/// A button on a notification. gpui's action carries a label and an id and
/// nothing else, so [`NotificationAction::reply`] is a button that brings the
/// conversation forward, not a place to type.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

impl NotificationAction {
    /// Identical to the iOS identifiers, so one response handler serves both.
    pub const REPLY_ID: &str = "REPLY_ACTION";
    pub const MARK_READ_ID: &str = "MARK_READ_ACTION";

    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }

    /// Labels arrive already translated; this crate has no locale.
    pub fn reply(label: impl Into<String>) -> Self {
        Self::new(Self::REPLY_ID, label)
    }

    pub fn mark_read(label: impl Into<String>) -> Self {
        Self::new(Self::MARK_READ_ID, label)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LocalNotification {
    pub tag: NotificationTag,
    pub title: String,
    pub body: String,
    pub actions: Vec<NotificationAction>,
}

impl LocalNotification {
    pub fn new(tag: NotificationTag, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            tag,
            title: title.into(),
            body: body.into(),
            actions: Vec::new(),
        }
    }

    pub fn with_action(mut self, action: NotificationAction) -> Self {
        self.actions.push(action);
        self
    }
}

impl From<NotificationAction> for gpui::SystemNotificationAction {
    fn from(action: NotificationAction) -> Self {
        Self {
            id: action.id.into(),
            label: action.label.into(),
        }
    }
}

impl From<LocalNotification> for gpui::SystemNotification {
    fn from(notification: LocalNotification) -> Self {
        Self {
            tag: notification.tag.0.into(),
            title: notification.title.into(),
            body: notification.body.into(),
            actions: notification.actions.into_iter().map(Into::into).collect(),
        }
    }
}

/// What each backend needs before gpui will post anything at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeliveryPrecondition {
    /// `UNUserNotificationCenter` aborts the process outside a bundle, so gpui
    /// disables notifications when the main bundle has no identifier — which a
    /// bare `cargo run` binary is.
    AppBundle,
    /// An unpackaged Windows process has no AUMID to post under, so gpui drops
    /// the toast unless `App::set_app_identity` ran during startup.
    AppIdentity,
    /// The XDG path needs an `org.freedesktop.Notifications` owner on the
    /// session bus; without one gpui logs and drops.
    NotificationServer,
}

pub const fn delivery_precondition(host: HostOs) -> DeliveryPrecondition {
    match host {
        HostOs::MacOs => DeliveryPrecondition::AppBundle,
        HostOs::Windows => DeliveryPrecondition::AppIdentity,
        HostOs::Linux => DeliveryPrecondition::NotificationServer,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotificationAvailability {
    /// gpui will hand the post to the OS. Not a promise that the user sees it:
    /// authorization can be denied and gpui reports neither that nor a failure.
    Available,
    /// The precondition is unmet, so every post would be a silent no-op.
    Blocked(DeliveryPrecondition),
}

impl NotificationAvailability {
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

pub const fn availability_for(host: HostOs, precondition_met: bool) -> NotificationAvailability {
    if precondition_met {
        NotificationAvailability::Available
    } else {
        NotificationAvailability::Blocked(delivery_precondition(host))
    }
}

/// Whether posting over a live notification with the same tag replaces it.
/// gpui never sets the XDG replaces-id, so on Linux a busy conversation fills
/// the shade and a caller wanting one banner must throttle by tag itself.
pub const fn tag_replaces_previous(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs | HostOs::Windows => PlatformSupport::Performed,
        HostOs::Linux => PlatformSupport::Unsupported,
    }
}

/// Whether a notification can be retracted once posted. gpui's Linux handle is
/// consumed waiting for the user's action, so its `dismiss` is a no-op and
/// stale notifications age out of the shade on their own.
pub const fn supports_dismissal(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs | HostOs::Windows => PlatformSupport::Performed,
        HostOs::Linux => PlatformSupport::Unsupported,
    }
}

/// Posting and retracting. Not `Send + Sync`: the real implementation borrows
/// the gpui `App`, and a notification is posted from the thread that owns it.
pub trait NotificationPresenter {
    /// `Performed` means the post reached gpui, not that a banner appeared.
    fn present(&self, notification: &LocalNotification) -> PlatformSupport;
    fn dismiss(&self, tag: &NotificationTag) -> PlatformSupport;
    fn availability(&self) -> NotificationAvailability;
}

/// The real one. Built per call site from a borrowed `App`.
pub struct GpuiNotificationPresenter<'a> {
    app: &'a gpui::App,
}

impl<'a> GpuiNotificationPresenter<'a> {
    pub fn new(app: &'a gpui::App) -> Self {
        Self { app }
    }
}

impl NotificationPresenter for GpuiNotificationPresenter<'_> {
    fn present(&self, notification: &LocalNotification) -> PlatformSupport {
        if !self.availability().is_available() {
            return PlatformSupport::Unsupported;
        }

        self.app
            .show_system_notification(notification.clone().into());
        PlatformSupport::Performed
    }

    fn dismiss(&self, tag: &NotificationTag) -> PlatformSupport {
        if !supports_dismissal(HostOs::current()).is_supported() {
            return PlatformSupport::Unsupported;
        }

        self.app.dismiss_system_notification(tag.as_str());
        PlatformSupport::Performed
    }

    fn availability(&self) -> NotificationAvailability {
        availability_for(HostOs::current(), host_precondition_met())
    }
}

/// The same probe gpui makes before it builds a notification centre at all.
/// `App::app_path` is not a substitute: `mainBundle` is non-nil for a bare
/// binary too.
#[cfg(target_os = "macos")]
fn host_precondition_met() -> bool {
    objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .is_some()
}

/// Neither the Windows AUMID registration nor an XDG notification server is
/// observable from this process, so this reports available and lets gpui log.
#[cfg(not(target_os = "macos"))]
fn host_precondition_met() -> bool {
    true
}

/// Records instead of posting. Not a fallback for a host that cannot notify: a
/// caller that ends up here believes it notified the user and did not.
pub struct InMemoryNotificationPresenter {
    availability: NotificationAvailability,
    presented: parking_lot::Mutex<Vec<LocalNotification>>,
    dismissed: parking_lot::Mutex<Vec<NotificationTag>>,
}

impl InMemoryNotificationPresenter {
    pub fn new() -> Self {
        Self {
            availability: NotificationAvailability::Available,
            presented: parking_lot::Mutex::new(Vec::new()),
            dismissed: parking_lot::Mutex::new(Vec::new()),
        }
    }

    pub fn blocked(precondition: DeliveryPrecondition) -> Self {
        Self {
            availability: NotificationAvailability::Blocked(precondition),
            presented: parking_lot::Mutex::new(Vec::new()),
            dismissed: parking_lot::Mutex::new(Vec::new()),
        }
    }

    pub fn presented(&self) -> Vec<LocalNotification> {
        self.presented.lock().clone()
    }

    pub fn dismissed(&self) -> Vec<NotificationTag> {
        self.dismissed.lock().clone()
    }
}

impl Default for InMemoryNotificationPresenter {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationPresenter for InMemoryNotificationPresenter {
    fn present(&self, notification: &LocalNotification) -> PlatformSupport {
        if !self.availability.is_available() {
            return PlatformSupport::Unsupported;
        }

        self.presented.lock().push(notification.clone());
        PlatformSupport::Performed
    }

    fn dismiss(&self, tag: &NotificationTag) -> PlatformSupport {
        if !self.availability.is_available() {
            return PlatformSupport::Unsupported;
        }

        self.dismissed.lock().push(tag.clone());
        PlatformSupport::Performed
    }

    fn availability(&self) -> NotificationAvailability {
        self.availability
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_760_000_000_000;

    fn keys(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| String::from(*value)).collect()
    }

    #[test]
    fn every_wire_scope_name_parses_to_its_switch() {
        let scopes = MuteScopes {
            messages: true,
            mentions: false,
            reactions: true,
            events: false,
        };

        for (raw, expected) in [
            ("messages", true),
            ("mentions", false),
            ("reactions", true),
            ("events", false),
        ] {
            let scope = NotificationScope::parse(raw).unwrap_or_else(|| panic!("{raw}"));
            assert_eq!(scope.is_enabled_in(&scopes), expected, "{raw}");
            assert_eq!(scope.as_str(), raw);
        }
    }

    #[test]
    fn scope_parsing_ignores_case_and_surrounding_whitespace() {
        assert_eq!(
            NotificationScope::parse("  Mentions\n"),
            Some(NotificationScope::Mentions),
            "the server sends this as free text and the reference folds it too"
        );
    }

    #[test]
    fn a_scope_this_build_has_never_heard_of_does_not_parse() {
        assert_eq!(NotificationScope::parse("typing"), None);
        assert_eq!(NotificationScope::parse(""), None);
    }

    #[test]
    fn an_unrecognised_scope_is_shown_even_when_everything_is_muted() {
        assert!(
            scope_allows(Some("polls"), Some(&MuteScopes::MUTED)),
            "a scope added server-side after this build must reach the user, \
             not vanish until the client updates"
        );
    }

    #[test]
    fn a_notification_carrying_no_scope_is_shown_even_when_everything_is_muted() {
        assert!(scope_allows(None, Some(&MuteScopes::MUTED)));
    }

    #[test]
    fn a_conversation_with_no_stored_scopes_shows_everything() {
        assert!(scope_allows(Some("messages"), None));
    }

    #[test]
    fn only_a_recognised_and_disabled_scope_stops_a_notification() {
        let scopes = MuteScopes {
            messages: false,
            ..MuteScopes::ALL_ENABLED
        };

        assert!(!scope_allows(Some("messages"), Some(&scopes)));
        assert!(scope_allows(Some("mentions"), Some(&scopes)));
    }

    #[test]
    fn every_mute_scope_defaults_to_enabled() {
        assert_eq!(MuteScopes::default(), MuteScopes::ALL_ENABLED);
        assert!(MuteScopes::default().is_all_enabled());
        assert!(!MuteScopes::default().is_fully_muted());
    }

    #[test]
    fn fully_muted_and_all_enabled_are_opposites() {
        assert!(MuteScopes::MUTED.is_fully_muted());
        assert!(!MuteScopes::MUTED.is_all_enabled());
        assert!(!MuteScopes::ALL_ENABLED.is_fully_muted());
    }

    #[test]
    fn one_disabled_scope_is_neither_all_enabled_nor_fully_muted() {
        let scopes = MuteScopes {
            reactions: false,
            ..MuteScopes::ALL_ENABLED
        };

        assert!(!scopes.is_all_enabled());
        assert!(!scopes.is_fully_muted());
    }

    #[test]
    fn mute_scopes_round_trip_through_the_stored_column() {
        let scopes = MuteScopes {
            messages: true,
            mentions: true,
            reactions: false,
            events: false,
        };

        let stored = scopes.to_json();
        assert!(stored.contains("\"reactions\":false"), "{stored}");
        assert_eq!(MuteScopes::parse(&stored), Some(scopes));
    }

    #[test]
    fn a_partial_mute_scopes_payload_is_refused_whole_rather_than_half_applied() {
        assert_eq!(MuteScopes::parse("{\"messages\":false}"), None);
        assert_eq!(
            MuteScopes::from_stored(Some("{\"messages\":false}")),
            MuteScopes::ALL_ENABLED,
            "half a payload must not silence scopes the server never mentioned"
        );
    }

    #[test]
    fn an_absent_or_unreadable_column_reads_as_all_enabled() {
        assert_eq!(MuteScopes::from_stored(None), MuteScopes::ALL_ENABLED);
        assert_eq!(MuteScopes::from_stored(Some("")), MuteScopes::ALL_ENABLED);
        assert_eq!(
            MuteScopes::from_stored(Some("not json")),
            MuteScopes::ALL_ENABLED
        );
    }

    #[test]
    fn a_mute_until_in_seconds_is_understood() {
        let seconds = NOW / 1_000 + 600;
        assert!(mute_is_active(Some(seconds), NOW));
        assert!(!mute_is_active(Some(NOW / 1_000 - 600), NOW));
    }

    #[test]
    fn a_mute_until_in_milliseconds_is_understood() {
        assert!(mute_is_active(Some(NOW + 600_000), NOW));
        assert!(!mute_is_active(Some(NOW - 600_000), NOW));
    }

    #[test]
    fn the_two_unit_shapes_of_one_instant_normalise_to_the_same_millisecond() {
        let seconds = 1_760_000_000;
        assert_eq!(mute_until_millis(seconds), mute_until_millis(NOW));
        assert_eq!(mute_until_millis(NOW), NOW);
    }

    #[test]
    fn no_mute_until_never_suppresses() {
        assert!(!mute_is_active(None, NOW));
    }

    #[test]
    fn a_corrupt_timestamp_cannot_overflow_the_normalisation() {
        assert!(!mute_is_active(Some(i64::MIN), NOW));
        assert!(!mute_is_active(Some(0), NOW));
    }

    #[test]
    fn a_notification_about_your_own_action_never_reaches_you() {
        let request = NotificationRequest::new("chat/42").with_actor("u1");
        let context = PresentationContext::new(NOW).as_user("u1");

        assert_eq!(
            decide(&request, &context),
            NotificationDecision::Suppressed(SuppressionReason::SelfAuthored),
            "the socket delivers this account's own events to every one of its \
             sessions, which APNs never did"
        );
    }

    #[test]
    fn someone_elses_action_in_the_same_conversation_still_reaches_you() {
        let request = NotificationRequest::new("chat/42").with_actor("u2");
        let context = PresentationContext::new(NOW).as_user("u1");

        assert_eq!(decide(&request, &context), NotificationDecision::Present);
    }

    #[test]
    fn an_absent_or_blank_actor_id_is_not_treated_as_yourself() {
        let context = PresentationContext::new(NOW).as_user("   ");

        for actor in [None, Some(""), Some("   ")] {
            let mut request = NotificationRequest::new("chat/42");
            request.actor_id = actor;

            assert_eq!(
                decide(&request, &context),
                NotificationDecision::Present,
                "two blank ids must not compare equal and swallow a real event"
            );
        }
    }

    #[test]
    fn a_muted_conversation_reports_why_rather_than_a_bare_refusal() {
        let mute = MuteState::muted_until(NOW + 60_000);
        let request = NotificationRequest::new("chat/42").with_scope("messages");
        let context = PresentationContext::new(NOW).with_mute(mute);

        assert_eq!(
            decide(&request, &context).reason(),
            Some(SuppressionReason::ConversationMuted)
        );
    }

    #[test]
    fn an_expired_mute_window_stops_suppressing() {
        let mute = MuteState::muted_until(NOW - 60_000);
        let request = NotificationRequest::new("chat/42").with_scope("messages");
        let context = PresentationContext::new(NOW).with_mute(mute);

        assert_eq!(decide(&request, &context), NotificationDecision::Present);
    }

    #[test]
    fn a_disabled_scope_is_reported_separately_from_a_mute_window() {
        let scopes = MuteScopes {
            reactions: false,
            ..MuteScopes::ALL_ENABLED
        };
        let request = NotificationRequest::new("chat/42").with_scope("reactions");
        let mute = MuteState::with_scopes(scopes);
        let context = PresentationContext::new(NOW).with_mute(mute);

        assert_eq!(
            decide(&request, &context).reason(),
            Some(SuppressionReason::ScopeDisabled)
        );
    }

    #[test]
    fn an_unfocused_window_still_notifies_for_the_conversation_it_is_showing() {
        let visible = keys(&["chat/42"]);
        let request = NotificationRequest::new("chat/42");
        let context = PresentationContext::new(NOW)
            .showing(&visible)
            .with_focus(false);

        assert_eq!(
            decide(&request, &context),
            NotificationDecision::Present,
            "a desktop keeps its socket behind another window; losing focus is \
             not the phone's scenePhase and must suppress nothing"
        );
    }

    #[test]
    fn a_focused_window_suppresses_only_the_route_actually_on_screen() {
        let visible = keys(&["chat/42", "profile/u9"]);
        let context = PresentationContext::new(NOW)
            .showing(&visible)
            .with_focus(true);

        assert_eq!(
            decide(&NotificationRequest::new("chat/42"), &context).reason(),
            Some(SuppressionReason::AlreadyOnScreen)
        );
        assert_eq!(
            decide(&NotificationRequest::new("chat/7"), &context),
            NotificationDecision::Present
        );
    }

    #[test]
    fn both_conversations_of_a_split_view_are_suppressed() {
        let visible = keys(&["chat/1", "chat/2"]);
        let context = PresentationContext::new(NOW)
            .showing(&visible)
            .with_focus(true);

        for route in ["chat/1", "chat/2"] {
            assert_eq!(
                decide(&NotificationRequest::new(route), &context).reason(),
                Some(SuppressionReason::AlreadyOnScreen),
                "a window shows more than one conversation, unlike a phone"
            );
        }
    }

    #[test]
    fn a_route_key_that_merely_shares_a_prefix_is_not_suppressed() {
        let visible = keys(&["chat/4"]);
        let context = PresentationContext::new(NOW)
            .showing(&visible)
            .with_focus(true);

        assert_eq!(
            decide(&NotificationRequest::new("chat/42"), &context),
            NotificationDecision::Present,
            "route keys are opaque to this crate and compared only by equality"
        );
    }

    #[test]
    fn a_standing_preference_outranks_the_transient_screen_state() {
        let visible = keys(&["chat/42"]);
        let request = NotificationRequest::new("chat/42").with_scope("messages");
        let context = PresentationContext::new(NOW)
            .showing(&visible)
            .with_focus(true)
            .with_mute(MuteState::muted_until(NOW + 60_000));

        assert_eq!(
            decide(&request, &context).reason(),
            Some(SuppressionReason::ConversationMuted),
            "mute explains tomorrow's silence too; being on screen does not"
        );
    }

    #[test]
    fn an_empty_context_presents() {
        let decision = decide(
            &NotificationRequest::new("chat/42"),
            &PresentationContext::new(NOW),
        );

        assert!(decision.is_present());
        assert_eq!(decision.reason(), None);
    }

    #[test]
    fn two_messages_in_one_conversation_share_a_tag() {
        assert_eq!(
            NotificationTag::conversation("chat/42"),
            NotificationTag::conversation("chat/42"),
            "forty messages must replace one banner, not stack forty"
        );
    }

    #[test]
    fn two_conversations_never_share_a_tag() {
        assert_ne!(
            NotificationTag::conversation("chat/42"),
            NotificationTag::conversation("chat/43")
        );
    }

    #[test]
    fn a_conversation_tag_cannot_collide_with_a_standalone_one() {
        assert_ne!(
            NotificationTag::conversation("x"),
            NotificationTag::unique("x")
        );
    }

    #[test]
    fn every_standalone_notification_keeps_its_own_banner() {
        assert_ne!(
            NotificationTag::unique("event-1"),
            NotificationTag::unique("event-2")
        );
    }

    #[test]
    fn a_conversation_tag_hands_its_route_back_for_the_click() {
        let tag = NotificationTag::conversation("chat/42");
        assert_eq!(tag.conversation_route(), Some("chat/42"));
        assert_eq!(
            NotificationTag::unique("chat/42").conversation_route(),
            None
        );
    }

    #[test]
    fn a_local_notification_maps_onto_the_gpui_payload_field_for_field() {
        let tag = NotificationTag::conversation("chat/42");
        let notification = LocalNotification::new(tag, "Ayan", "привет")
            .with_action(NotificationAction::mark_read("Mark as read"));

        let posted: gpui::SystemNotification = notification.into();

        assert_eq!(posted.tag, "conv:chat/42");
        assert_eq!(posted.title, "Ayan");
        assert_eq!(posted.body, "привет");
        assert_eq!(posted.actions.len(), 1);
        assert_eq!(posted.actions[0].id, NotificationAction::MARK_READ_ID);
        assert_eq!(posted.actions[0].label, "Mark as read");
    }

    #[test]
    fn a_notification_without_buttons_maps_to_an_empty_action_list() {
        let notification = LocalNotification::new(NotificationTag::unique("e1"), "t", "b");
        let posted: gpui::SystemNotification = notification.into();

        assert!(posted.actions.is_empty());
    }

    #[test]
    fn every_host_names_its_own_delivery_precondition() {
        for host in HostOs::ALL {
            let shared = HostOs::ALL
                .iter()
                .filter(|other| delivery_precondition(**other) == delivery_precondition(host))
                .count();

            assert_eq!(
                shared, 1,
                "{host:?} cannot be told apart from another host's precondition"
            );
        }
    }

    #[test]
    fn no_host_is_inherently_unable_to_post() {
        for host in HostOs::ALL {
            assert_eq!(
                availability_for(host, true),
                NotificationAvailability::Available,
                "gpui implements show_system_notification on all three backends"
            );
        }
    }

    #[test]
    fn an_unmet_precondition_reports_which_one() {
        assert_eq!(
            availability_for(HostOs::MacOs, false),
            NotificationAvailability::Blocked(DeliveryPrecondition::AppBundle)
        );
        assert_eq!(
            availability_for(HostOs::Windows, false),
            NotificationAvailability::Blocked(DeliveryPrecondition::AppIdentity)
        );
        assert_eq!(
            availability_for(HostOs::Linux, false),
            NotificationAvailability::Blocked(DeliveryPrecondition::NotificationServer)
        );
    }

    #[test]
    fn only_linux_stacks_repeated_notifications_for_one_tag() {
        assert!(tag_replaces_previous(HostOs::MacOs).is_supported());
        assert!(tag_replaces_previous(HostOs::Windows).is_supported());
        assert!(
            !tag_replaces_previous(HostOs::Linux).is_supported(),
            "gpui never sets the XDG replaces-id, so a busy conversation fills \
             the shade unless the caller throttles"
        );
    }

    #[test]
    fn only_linux_cannot_retract_a_notification() {
        assert!(supports_dismissal(HostOs::MacOs).is_supported());
        assert!(supports_dismissal(HostOs::Windows).is_supported());
        assert!(!supports_dismissal(HostOs::Linux).is_supported());
    }

    #[test]
    fn the_recording_presenter_keeps_what_it_was_handed() {
        let presenter = InMemoryNotificationPresenter::new();
        let tag = NotificationTag::conversation("chat/42");
        let notification = LocalNotification::new(tag, "Ayan", "hi");

        assert_eq!(presenter.present(&notification), PlatformSupport::Performed);
        assert_eq!(presenter.presented(), vec![notification]);
    }

    #[test]
    fn dismissing_records_the_tag_it_was_asked_to_retract() {
        let presenter = InMemoryNotificationPresenter::new();
        let tag = NotificationTag::conversation("chat/42");

        assert_eq!(presenter.dismiss(&tag), PlatformSupport::Performed);
        assert_eq!(presenter.dismissed(), vec![tag]);
    }

    #[test]
    fn a_blocked_presenter_says_so_and_records_nothing() {
        let blocked = DeliveryPrecondition::AppBundle;
        let presenter = InMemoryNotificationPresenter::blocked(blocked);
        let notification = LocalNotification::new(NotificationTag::unique("e1"), "Ayan", "hi");

        assert_eq!(
            presenter.availability(),
            NotificationAvailability::Blocked(DeliveryPrecondition::AppBundle)
        );
        assert_eq!(
            presenter.present(&notification),
            PlatformSupport::Unsupported,
            "a caller must be able to fall back to in-app UI instead of \
             believing it told the user"
        );
        assert!(presenter.presented().is_empty());
    }
}
