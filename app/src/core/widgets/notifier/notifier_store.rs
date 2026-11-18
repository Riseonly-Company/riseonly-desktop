use std::collections::HashMap;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Global, Hsla, Task};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::DESKTOP_NOTIFICATION_BANNER_POSITION;
use rise_theme::color;
use rise_ui::swiftui::motion::duration_from_seconds;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, UIApplication};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};
use crate::core::restriction::restriction_store::RestrictionStore;

const DEFAULT_SEND_DURATION: TimeInterval = 3.2;
const DEFAULT_REPLACEMENT_DELAY_NANOSECONDS: u64 = 200_000_000;
const MINIMUM_UNDO_WINDOW: TimeInterval = 0.5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotifierPosition {
    Top,
    #[default]
    Bottom,
}

/// where in app notifications show, the person picks it in the notification settings
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotifierBannerPosition {
    #[default]
    TopRight,
    TopCenter,
    TopLeft,
}

impl Global for NotifierBannerPosition {}

impl NotifierBannerPosition {
    pub const ALL: [Self; 3] = [Self::TopRight, Self::TopCenter, Self::TopLeft];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::TopRight => "top_right",
            Self::TopCenter => "top_center",
            Self::TopLeft => "top_left",
        }
    }

    pub const fn title_key(self) -> &'static str {
        match self {
            Self::TopRight => "desktop_notification_banner_right",
            Self::TopCenter => "desktop_notification_banner_center",
            Self::TopLeft => "desktop_notification_banner_left",
        }
    }

    pub fn current(cx: &App) -> Self {
        if let Some(position) = cx.try_global::<Self>() {
            return *position;
        }
        let stored = UserDefaults::standard().string(DESKTOP_NOTIFICATION_BANNER_POSITION);
        Self::ALL.into_iter().find(|position| Some(position.raw_value()) == stored.as_deref()).unwrap_or_default()
    }

    pub fn set(position: Self, cx: &mut App) {
        UserDefaults::standard().set(DESKTOP_NOTIFICATION_BANNER_POSITION, position.raw_value());
        cx.set_global(position);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotifierType {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl NotifierType {
    pub fn accent_color(&self) -> Hsla {
        match self {
            Self::Info => color("#3F8EF7"),
            Self::Success => color("#27AE60"),
            Self::Warning => color("#F2A53C"),
            Self::Error => color("#EB5757"),
        }
    }

    pub fn default_symbol(&self) -> &'static str {
        match self {
            Self::Info => "info.circle.fill",
            Self::Success => "checkmark.circle.fill",
            Self::Warning => "exclamationmark.triangle.fill",
            Self::Error => "xmark.octagon.fill",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NotifierIcon {
    #[default]
    None,
    Emoji(String),
    System(String),
    Lottie(String),
    Avatar { source: Option<String>, name: Option<String>, user_id: Option<String> },
}

/// author of a group or forum message
/// banner names the community first, then the person on their own line
/// avatar is pinned to the community one
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifierAuthor {
    pub name: String,
    pub tag: Option<String>,
    pub avatar_url: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifierAction {
    DeepLink(PushDeepLink),
}

#[derive(Clone, Debug, PartialEq)]
pub struct NotifierUndoPresentation {
    pub action_title: String,
    pub deadline: Date,
    pub shows_countdown: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifierUndoOutcome {
    Commit,
    Cancel,
}

/// undo window for a destructive action, the request is not sent while it runs
/// only the action button cancels, any other end of the window commits
pub enum NotifierUndoPolicy {}

impl NotifierUndoPolicy {
    pub const WINDOW_SECONDS: TimeInterval = 5.0;

    pub fn seconds_remaining(until: &Date, now: &Date) -> i64 {
        let remaining = until.time_interval_since(now);
        if remaining <= 0.0 {
            return 0;
        }
        (remaining.ceil() as i64).max(1)
    }

    pub fn ring_progress(until: &Date, now: &Date, window: TimeInterval) -> f64 {
        if window <= 0.0 {
            return 0.0;
        }
        let remaining = until.time_interval_since(now);
        (remaining / window).clamp(0.0, 1.0)
    }
}

impl NotifierConfig {
    /// a notification about someone or something, it opens a place when tapped
    /// it shows where the person put banners, feedback toasts stay at the top centre
    pub fn is_in_app_notification(&self) -> bool {
        self.action.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NotifierConfig {
    pub id: Uuid,
    pub title: String,
    pub content: Option<String>,
    pub r#type: NotifierType,
    pub position: NotifierPosition,
    pub offset: f32,
    pub icon: NotifierIcon,
    pub action: Option<NotifierAction>,
    pub duration: TimeInterval,
    pub undo: Option<NotifierUndoPresentation>,
    pub content_is_markup: bool,
    pub author: Option<NotifierAuthor>,
    pub replace_key: Option<String>,
}

/// default params of the notifier send call
#[derive(Clone, Debug, PartialEq)]
pub struct NotifierSendOptions {
    pub content: Option<String>,
    pub r#type: NotifierType,
    pub position: NotifierPosition,
    pub offset: f32,
    pub icon: NotifierIcon,
    pub action: Option<NotifierAction>,
    pub duration: TimeInterval,
    pub content_is_markup: bool,
    pub author: Option<NotifierAuthor>,
    pub replace_key: Option<String>,
}

impl Default for NotifierSendOptions {
    fn default() -> Self {
        Self {
            content: None,
            r#type: NotifierType::Info,
            position: NotifierPosition::Bottom,
            offset: 0.0,
            icon: NotifierIcon::None,
            action: None,
            duration: DEFAULT_SEND_DURATION,
            content_is_markup: false,
            author: None,
            replace_key: None,
        }
    }
}

/// default params of the notifier send undo call
#[derive(Clone, Debug, PartialEq)]
pub struct NotifierUndoOptions {
    pub content: Option<String>,
    pub r#type: NotifierType,
    pub position: NotifierPosition,
    pub offset: f32,
    pub icon: NotifierIcon,
    pub duration: TimeInterval,
    pub shows_countdown: bool,
}

impl Default for NotifierUndoOptions {
    fn default() -> Self {
        Self {
            content: None,
            r#type: NotifierType::Info,
            position: NotifierPosition::Bottom,
            offset: 0.0,
            icon: NotifierIcon::None,
            duration: NotifierUndoPolicy::WINDOW_SECONDS,
            shows_countdown: true,
        }
    }
}

/// default params of the notifier send retry call
#[derive(Clone, Debug, PartialEq)]
pub struct NotifierRetryOptions {
    pub content: Option<String>,
    pub position: NotifierPosition,
    pub duration: TimeInterval,
}

impl Default for NotifierRetryOptions {
    fn default() -> Self {
        Self { content: None, position: NotifierPosition::Bottom, duration: NotifierUndoPolicy::WINDOW_SECONDS }
    }
}

pub type NotifierUndoHandler = Box<dyn FnOnce(&mut App)>;

struct NotifierUndoHandlers {
    commit: NotifierUndoHandler,
    cancel: NotifierUndoHandler,
}

pub struct NotifierStore {
    items: Vec<NotifierConfig>,
    pending_latest: Option<NotifierConfig>,
    timers: HashMap<Uuid, Task<()>>,
    replacement_task: Option<Task<()>>,
    replacement_generation: u64,
    is_replacing: bool,
    undo_handlers: HashMap<Uuid, NotifierUndoHandlers>,
    replacement_delay_nanoseconds: u64,
    held: Option<Uuid>,
    awaits_return: Option<Uuid>,
}

impl NotifierStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |cx| Self::new(DEFAULT_REPLACEMENT_DELAY_NANOSECONDS, cx))
    }

    pub fn new(replacement_delay_nanoseconds: u64, cx: &mut Context<Self>) -> Self {
        let (returns, mut inbox) = mpsc::unbounded::<()>();
        RiseAppEvents::default_center()
            .observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, move |_| {
                let _ = returns.unbounded_send(());
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                if this.update(cx, |store, cx| store.app_returned(cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
        Self {
            items: Vec::new(),
            pending_latest: None,
            timers: HashMap::new(),
            replacement_task: None,
            replacement_generation: 0,
            is_replacing: false,
            undo_handlers: HashMap::new(),
            replacement_delay_nanoseconds,
            held: None,
            awaits_return: None,
        }
    }

    fn app_returned(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.awaits_return.take() else {
            return;
        };
        if let Some(duration) = self.items.first().filter(|first| first.id == id).map(|first| first.duration) {
            self.restart_timer(id, duration, cx);
        }
    }

    pub fn items(&self) -> &[NotifierConfig] {
        &self.items
    }

    pub fn pending_latest(&self) -> Option<&NotifierConfig> {
        self.pending_latest.as_ref()
    }

    pub fn send(&mut self, title: String, options: NotifierSendOptions, cx: &mut Context<Self>) -> Uuid {
        if let Some(replace_key) = options.replace_key.as_ref()
            && !self.is_replacing
            && let Some(current_id) = self
                .items
                .first()
                .filter(|current| current.replace_key.as_ref() == Some(replace_key) && current.undo.is_none())
                .map(|current| current.id)
        {
            let duration = options.duration;
            self.items = vec![NotifierConfig {
                id: current_id,
                title,
                content: options.content,
                r#type: options.r#type,
                position: options.position,
                offset: options.offset,
                icon: options.icon,
                action: options.action,
                duration,
                undo: None,
                content_is_markup: options.content_is_markup,
                author: options.author,
                replace_key: options.replace_key,
            }];
            self.restart_timer(current_id, duration, cx);
            cx.notify();
            return current_id;
        }
        let config = NotifierConfig {
            id: Uuid::new_v4(),
            title,
            content: options.content,
            r#type: options.r#type,
            position: options.position,
            offset: options.offset,
            icon: options.icon,
            action: options.action,
            duration: options.duration,
            undo: None,
            content_is_markup: options.content_is_markup,
            author: options.author,
            replace_key: options.replace_key,
        };
        let id = config.id;
        tracing::warn!(target: "riseonly::X1428", ?id, items = self.items.len(), replacing = self.is_replacing, "toast queued");

        if self.items.is_empty() && !self.is_replacing {
            self.present(config, cx);
        } else {
            self.replace_pending_latest(Some(config), cx);
            self.begin_replacement_if_needed(cx);
        }
        cx.notify();
        id
    }

    pub fn send_undo(
        &mut self,
        title: String,
        action_title: String,
        options: NotifierUndoOptions,
        on_commit: impl FnOnce(&mut App) + 'static,
        on_cancel: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Uuid {
        let window = options.duration.max(MINIMUM_UNDO_WINDOW);
        let config = NotifierConfig {
            id: Uuid::new_v4(),
            title,
            content: options.content,
            r#type: options.r#type,
            position: options.position,
            offset: options.offset,
            icon: options.icon,
            action: None,
            duration: window,
            undo: Some(NotifierUndoPresentation {
                action_title,
                deadline: Date::now().adding_time_interval(window),
                shows_countdown: options.shows_countdown,
            }),
            content_is_markup: false,
            author: None,
            replace_key: None,
        };
        let id = config.id;

        self.undo_handlers
            .insert(id, NotifierUndoHandlers { commit: Box::new(on_commit), cancel: Box::new(on_cancel) });

        if self.items.is_empty() && !self.is_replacing {
            self.present(config, cx);
        } else {
            self.replace_pending_latest(Some(config), cx);
            self.begin_replacement_if_needed(cx);
        }
        cx.notify();
        id
    }

    pub fn extend_undo(&mut self, id: Uuid, title: String, duration: TimeInterval, cx: &mut Context<Self>) -> bool {
        if !self.undo_handlers.contains_key(&id) {
            return false;
        }
        let window = duration.max(MINIMUM_UNDO_WINDOW);

        if let Some(pending) = self.pending_latest.as_mut().filter(|pending| pending.id == id) {
            pending.title = title;
            pending.duration = window;
            cx.notify();
            return true;
        }

        let Some(config) = self.items.iter_mut().find(|item| item.id == id) else {
            return false;
        };
        config.title = title;
        config.duration = window;
        if let Some(undo) = config.undo.as_mut() {
            undo.deadline = Date::now().adding_time_interval(window);
        }

        self.cancel_timer(id);
        self.schedule_dismiss(id, window, cx);
        cx.notify();
        true
    }

    /// desktop hover, pointer over an island stops its dismiss timer
    /// undo islands keep their deadline because the ring counts down to commit
    pub fn hold(&mut self, id: Uuid) {
        if self.items.first().is_none_or(|first| first.id != id || first.undo.is_some()) {
            return;
        }
        self.held = Some(id);
        self.cancel_timer(id);
    }

    pub fn release_hold(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.held != Some(id) {
            return;
        }
        self.held = None;
        let Some(duration) = self.items.first().filter(|first| first.id == id).map(|first| first.duration) else {
            return;
        };
        self.restart_timer(id, duration, cx);
    }

    pub fn dismiss(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.held == Some(id) {
            self.held = None;
        }
        self.resolve_undo(id, NotifierUndoOutcome::Commit, cx);
        self.cancel_timer(id);

        if self.pending_latest.as_ref().is_some_and(|pending| pending.id == id) {
            self.pending_latest = None;
            cx.notify();
        }

        if self.items.first().is_none_or(|first| first.id != id) {
            return;
        }
        self.items.clear();
        cx.notify();
    }

    pub fn undo(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if !self.resolve_undo(id, NotifierUndoOutcome::Cancel, cx) {
            tracing::warn!(target: "riseonly::X971", "undo {}: window already resolved", id.uuid_string());
            return;
        }
        tracing::warn!(target: "riseonly::X971", "undo {}: cancelled", id.uuid_string());
        self.cancel_timer(id);

        if self.pending_latest.as_ref().is_some_and(|pending| pending.id == id) {
            self.pending_latest = None;
            cx.notify();
        }

        if self.items.first().is_none_or(|first| first.id != id) {
            return;
        }
        self.items.clear();
        cx.notify();
    }

    pub fn activate(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(item) = self.items.iter().find(|item| item.id == id) else {
            return;
        };
        if item.undo.is_some() {
            return;
        }
        let action = item.action.clone();
        self.dismiss(id, cx);

        if let Some(NotifierAction::DeepLink(route)) = action {
            cx.defer(move |cx| {
                PushDeepLinkStore::shared(cx).update(cx, |store, cx| store.enqueue(route, cx));
            });
        }
    }

    pub fn commit_pending_undo_actions(&mut self, cx: &mut Context<Self>) {
        for commit in self.take_pending_undo_commits(cx) {
            Self::run_handler(commit, cx);
        }
    }

    fn take_pending_undo_commits(&mut self, cx: &mut Context<Self>) -> Vec<NotifierUndoHandler> {
        if self.undo_handlers.is_empty() {
            return Vec::new();
        }

        let outstanding = std::mem::take(&mut self.undo_handlers);
        let outstanding_ids: Vec<Uuid> = outstanding.keys().copied().collect();
        let commits: Vec<NotifierUndoHandler> = outstanding.into_values().map(|handlers| handlers.commit).collect();

        if let Some(current_id) = self.items.first().map(|current| current.id)
            && outstanding_ids.contains(&current_id)
        {
            self.cancel_timer(current_id);
            self.items.clear();
            cx.notify();
        }
        if self.pending_latest.as_ref().is_some_and(|next| outstanding_ids.contains(&next.id)) {
            self.pending_latest = None;
            cx.notify();
        }
        commits
    }

    pub fn dismiss_all(&mut self, cx: &mut Context<Self>) {
        self.commit_pending_undo_actions(cx);
        self.timers.clear();
        self.replacement_task = None;
        self.replacement_generation = self.replacement_generation.wrapping_add(1);
        self.is_replacing = false;
        self.pending_latest = None;
        self.items.clear();
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.dismiss_all(cx);
    }

    pub fn dismiss_errors(&mut self, cx: &mut Context<Self>) {
        if self.pending_latest.as_ref().is_some_and(|next| next.r#type == NotifierType::Error) {
            self.replace_pending_latest(None, cx);
        }
        if let Some(current_id) = self
            .items
            .first()
            .filter(|current| current.r#type == NotifierType::Error)
            .map(|current| current.id)
        {
            self.dismiss(current_id, cx);
        }
    }

    fn resolve_undo(&mut self, id: Uuid, outcome: NotifierUndoOutcome, cx: &mut Context<Self>) -> bool {
        let Some(handlers) = self.undo_handlers.remove(&id) else {
            return false;
        };

        match outcome {
            NotifierUndoOutcome::Commit => Self::run_handler(handlers.commit, cx),
            NotifierUndoOutcome::Cancel => Self::run_handler(handlers.cancel, cx),
        }
        true
    }

    fn run_handler(handler: NotifierUndoHandler, cx: &mut Context<Self>) {
        cx.defer(move |cx| handler(cx));
    }

    fn replace_pending_latest(&mut self, config: Option<NotifierConfig>, cx: &mut Context<Self>) {
        if let Some(previous_id) = self.pending_latest.as_ref().map(|previous| previous.id)
            && config.as_ref().is_none_or(|config| config.id != previous_id)
        {
            self.resolve_undo(previous_id, NotifierUndoOutcome::Commit, cx);
        }
        self.pending_latest = config;
        cx.notify();
    }

    fn present(&mut self, mut config: NotifierConfig, cx: &mut Context<Self>) {
        if let Some(undo) = config.undo.as_mut() {
            undo.deadline = Date::now().adding_time_interval(config.duration);
        }
        let id = config.id;
        let duration = config.duration;
        self.items = vec![config];
        cx.notify();

        self.restart_timer(id, duration, cx);
    }

    fn restart_timer(&mut self, id: Uuid, duration: TimeInterval, cx: &mut Context<Self>) {
        self.cancel_timer(id);
        if duration <= 0.0 || self.held == Some(id) {
            return;
        }
        let banner = self.items.first().is_some_and(|first| first.id == id && first.is_in_app_notification());
        if banner && RiseAppLifecycle::scene_phase(cx) != ScenePhase::Active {
            self.awaits_return = Some(id);
            return;
        }
        self.schedule_dismiss(id, duration, cx);
    }

    fn schedule_dismiss(&mut self, id: Uuid, seconds: TimeInterval, cx: &mut Context<Self>) {
        let delay = duration_from_seconds(seconds);
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |store, cx| store.dismiss(id, cx));
        });
        self.timers.insert(id, task);
    }

    fn begin_replacement_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.is_replacing {
            return;
        }

        self.is_replacing = true;
        if let Some(current_id) = self.items.first().map(|current| current.id) {
            self.resolve_undo(current_id, NotifierUndoOutcome::Commit, cx);
            self.cancel_timer(current_id);
        }
        self.items.clear();
        cx.notify();

        self.replacement_generation = self.replacement_generation.wrapping_add(1);
        let generation = self.replacement_generation;
        let delay = Duration::from_nanos(self.replacement_delay_nanoseconds);
        self.replacement_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |store, cx| store.finish_replacement(generation, cx));
        }));
    }

    fn finish_replacement(&mut self, generation: u64, cx: &mut Context<Self>) {
        if !self.is_replacing || self.replacement_generation != generation {
            return;
        }

        self.replacement_task = None;
        self.is_replacing = false;
        let Some(next) = self.pending_latest.take() else {
            return;
        };
        cx.notify();
        self.present(next, cx);
    }

    fn cancel_timer(&mut self, id: Uuid) {
        self.timers.remove(&id);
    }
}

/// app wide entry, every toast goes through here
pub enum Notifier {}

impl Notifier {
    pub fn send(title: impl Into<String>, options: NotifierSendOptions, cx: &mut App) -> Uuid {
        let title = title.into();
        if options.r#type == NotifierType::Error && RestrictionStore::shared(cx).read(cx).is_presented() {
            tracing::warn!(target: "riseonly::X1428", %title, "toast refused under the restriction modal");
            return Uuid::new_v4();
        }
        if matches!(options.r#type, NotifierType::Error | NotifierType::Warning) {
            let kind = if options.r#type == NotifierType::Error { "error" } else { "warning" };
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("toast {kind}: {title}"));
        }
        NotifierStore::shared(cx).update(cx, |store, cx| store.send(title, options, cx))
    }

    /// confirms a clipboard write
    pub fn copied(cx: &mut App) {
        let _ = notification(NotificationKind::Success);
        let options = NotifierSendOptions { r#type: NotifierType::Success, ..NotifierSendOptions::default() };
        Self::send(tr("copied"), options, cx);
    }

    pub fn send_undo(
        title: impl Into<String>,
        action_title: impl Into<String>,
        options: NotifierUndoOptions,
        on_commit: impl FnOnce(&mut App) + 'static,
        on_cancel: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) -> Uuid {
        let title = title.into();
        let action_title = action_title.into();
        NotifierStore::shared(cx)
            .update(cx, |store, cx| store.send_undo(title, action_title, options, on_commit, on_cancel, cx))
    }

    /// failure with one way out, button runs on_retry
    /// an unanswered toast just leaves and runs on_expire
    pub fn send_retry(
        title: impl Into<String>,
        options: NotifierRetryOptions,
        on_expire: impl FnOnce(&mut App) + 'static,
        on_retry: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) -> Uuid {
        let title = title.into();
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("toast error: {title}"));
        let undo_options = NotifierUndoOptions {
            content: options.content,
            r#type: NotifierType::Error,
            position: options.position,
            duration: options.duration,
            shows_countdown: false,
            ..NotifierUndoOptions::default()
        };
        NotifierStore::shared(cx)
            .update(cx, |store, cx| store.send_undo(title, tr("retry"), undo_options, on_expire, on_retry, cx))
    }

    pub fn extend_undo(id: Uuid, title: impl Into<String>, duration: TimeInterval, cx: &mut App) -> bool {
        let title = title.into();
        NotifierStore::shared(cx).update(cx, |store, cx| store.extend_undo(id, title, duration, cx))
    }

    pub fn dismiss(id: Uuid, cx: &mut App) {
        NotifierStore::shared(cx).update(cx, |store, cx| store.dismiss(id, cx));
    }

    pub fn commit_pending_undo(cx: &mut App) {
        NotifierStore::shared(cx).update(cx, |store, cx| store.commit_pending_undo_actions(cx));
    }

    pub fn commit_pending_undo_now(cx: &mut App) {
        let commits = NotifierStore::shared(cx).update(cx, |store, cx| store.take_pending_undo_commits(cx));
        for commit in commits {
            commit(cx);
        }
    }
}
