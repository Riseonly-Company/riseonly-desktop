use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use rise_core::foundation::foundation_date::RiseClock;
use gpui::{
    AnyElement, AnyView, AnyWeakView, App, AppContext, Context, ElementId, Entity, IntoElement, ParentElement, Pixels,
    Render, RenderOnce, Size, Styled, Subscription, Window, div,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::hold_context_menu_ui::hold_context_menu_controller_preview::{
    HoldContextMenuControllerPreview, HoldContextMenuPreviewAction,
};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::motion::{MotionSpec, spring};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_host::HoldContextMenuWindowPresenter;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseListRemovalDiag;
use crate::modules::chat::components::chat_item::chat_context_preview_hooks::chat_context_preview_hooks;
use crate::modules::chat::pages::chats::embedded_forum_transition::EmbeddedForumPresentationStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const COMMIT_COVER_MINIMUM: Duration = Duration::from_millis(480);
const COMMIT_COVER_LIMIT: Duration = Duration::from_millis(2_400);
const COMMIT_COVER_POLL: Duration = Duration::from_millis(32);

pub struct ChatContextPreviewSession {
    is_preview: bool,
    uses_preview_layout: bool,
    uses_preview_layout_animation: Option<MotionSpec>,
    is_transferring: bool,
    route: RootRoute,
    navigation: Entity<NavigationStore>,
    account_epoch: Option<String>,
    navigation_epoch: Option<Uuid>,
    pub opening_loaded_scope: Option<String>,
    has_initial_frame: bool,
    controller: Option<AnyWeakView>,
}

impl ChatContextPreviewSession {
    pub fn new(chat_id: String, all_topics: bool, navigation: Entity<NavigationStore>, topic_id: Option<String>) -> Self {
        let route = match topic_id.filter(|topic_id| !topic_id.is_empty()) {
            Some(topic_id) => RootRoute::ChatTopic { chat_id: Some(chat_id), topic_id: Some(topic_id) },
            None if all_topics => RootRoute::ForumAll { chat_id: Some(chat_id) },
            None => RootRoute::Chat { chat_id: Some(chat_id) },
        };
        Self {
            is_preview: true,
            uses_preview_layout: true,
            uses_preview_layout_animation: None,
            is_transferring: false,
            route,
            navigation,
            account_epoch: SaiAccountCacheScope::active_epoch(),
            navigation_epoch: None,
            opening_loaded_scope: None,
            has_initial_frame: false,
            controller: None,
        }
    }

    pub fn controller(&self) -> Option<AnyView> {
        self.controller.as_ref().and_then(AnyWeakView::upgrade)
    }

    pub fn is_preview(&self) -> bool {
        self.is_preview
    }

    pub fn uses_preview_layout(&self) -> bool {
        self.uses_preview_layout
    }

    /// animation spec of the expand, the screen reading uses_preview_layout animates by it
    /// none when reduce motion is on
    pub fn uses_preview_layout_animation(&self) -> Option<MotionSpec> {
        self.uses_preview_layout_animation
    }

    pub fn is_transferring(&self) -> bool {
        self.is_transferring
    }

    pub fn is_current_account(&self) -> bool {
        self.account_epoch == SaiAccountCacheScope::active_epoch()
    }

    pub fn navigation_epoch(&self) -> Option<Uuid> {
        self.navigation_epoch
    }

    pub fn expand(&mut self, cx: &mut Context<Self>) {
        if !self.is_preview || !self.is_current_account() {
            return;
        }
        self.uses_preview_layout_animation = if cx.reduce_motion() { None } else { Some(spring(0.46, 0.88)) };
        self.uses_preview_layout = false;
        cx.notify();
    }

    /// the preview page isnt moved into the navigation stack any more
    /// the menu closes and the chat is pushed as an ordinary route
    pub fn commit(&mut self, completion: HoldContextMenuPreviewAction, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_preview || !self.is_current_account() {
            window.defer(cx, move |window, cx| completion(window, cx));
            return;
        }
        HoldContextMenuWindowPresenter::restore_source_key_window(window, cx);
        let route = self.route.clone();
        let navigation = self.navigation.clone();
        let pushed = route.clone();
        let epoch = navigation.update(cx, |navigation, cx| {
            navigation.push(pushed, NavigationStoreSettlingPush::Drop, cx);
            navigation.chat_presentation_epoch_for(&route)
        });
        let Some(epoch) = epoch else {
            window.defer(cx, move |window, cx| completion(window, cx));
            return;
        };
        window
            .spawn(cx, async move |cx| {
                let started = RiseClock::monotonic();
                let mut reason = "limit";
                while (RiseClock::monotonic().saturating_sub(started)) < COMMIT_COVER_LIMIT {
                    let state = cx.update(|_, cx| {
                        let navigation = navigation.read(cx);
                        (navigation.path.last() == Some(&route), navigation.chat_first_frame_ready_epoch())
                    });
                    let Ok((on_top, ready_epoch)) = state else {
                        return;
                    };
                    if !on_top {
                        reason = "popped";
                        break;
                    }
                    if ready_epoch == Some(epoch) && (RiseClock::monotonic().saturating_sub(started)) >= COMMIT_COVER_MINIMUM {
                        reason = "first-frame";
                        break;
                    }
                    cx.background_executor().timer(COMMIT_COVER_POLL).await;
                }
                let elapsed_ms = (RiseClock::monotonic().saturating_sub(started)).as_millis();
                let _ = cx.update(|window, cx| {
                    RiseListRemovalDiag::log(
                        || format!("[d8f4] commit cover end reason={reason} ms={elapsed_ms}"),
                        false,
                        cx,
                    );
                    completion(window, cx);
                });
            })
            .detach();
    }

    /// the adopted page already drew its history in the preview
    /// the chat keeps that list mounted instead of swapping it for loading while the opening settles
    pub fn presents_adopted_feed(&self) -> bool {
        self.has_initial_frame && !self.is_preview
    }

    pub fn initial_frame_ready(&mut self, cx: &mut Context<Self>) {
        self.has_initial_frame = true;
        self.destination_ready(cx);
    }

    pub fn destination_ready(&mut self, cx: &mut Context<Self>) {
        if !self.has_initial_frame || self.is_preview || !self.is_route_on_top(cx) || !self.is_current_account() {
            return;
        }
        let route = self.route.clone();
        let epoch = self.navigation_epoch;
        self.navigation.update(cx, |navigation, cx| navigation.complete_chat_first_frame_transition(&route, epoch, cx));
    }

    fn is_route_on_top(&self, cx: &App) -> bool {
        self.navigation.read(cx).path.last() == Some(&self.route)
    }
}

struct ChatContextPreviewController {
    chat_id: String,
    topic_id: Option<String>,
    initial_all_view: bool,
    navigation: Entity<NavigationStore>,
    session: Entity<ChatContextPreviewSession>,
}

impl Render for ChatContextPreviewController {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = chat_context_preview_hooks().map(|hooks| {
            (hooks.chat_screen_context_preview)(
                Some(self.chat_id.clone()),
                self.topic_id.clone(),
                self.initial_all_view,
                self.session.clone(),
                window,
                cx,
            )
        });
        env_scope::<NavigationStoreKey>(Some(self.navigation.clone())).child(div().size_full().children(screen))
    }
}

pub enum ChatContextPreview {}

impl ChatContextPreview {
    pub fn make(
        chat: &ChatInfo,
        available_size: Size<Pixels>,
        navigation: Entity<NavigationStore>,
        cx: &mut App,
    ) -> HoldContextMenuControllerPreview {
        Self::make_with_chat_id(&chat.id, None, available_size, navigation, cx)
    }

    /// a text channel of a forum opens the same scrollable chat card on hold
    pub fn make_with_chat_id(
        chat_id: &str,
        topic_id: Option<String>,
        available_size: Size<Pixels>,
        navigation: Entity<NavigationStore>,
        cx: &mut App,
    ) -> HoldContextMenuControllerPreview {
        let is_topic = topic_id.as_deref().is_some_and(|topic_id| !topic_id.is_empty());
        let all_topics = !is_topic
            && ChatServicesStore::shared(cx).update(cx, |services, cx| services.topics_enabled(chat_id, cx));
        let topic_id = topic_id.filter(|_| is_topic);
        let session_navigation = navigation.clone();
        let session_topic = topic_id.clone();
        let session_chat = chat_id.to_owned();
        let session = cx.new(|_| {
            ChatContextPreviewSession::new(session_chat, all_topics, session_navigation, session_topic)
        });
        let controller_session = session.clone();
        let controller = cx.new(|_| ChatContextPreviewController {
            chat_id: chat_id.to_owned(),
            topic_id,
            initial_all_view: all_topics,
            navigation,
            session: controller_session,
        });
        let controller = AnyView::from(controller);
        session.update(cx, |session, _| session.controller = Some(controller.downgrade()));
        let expanding = session.clone();
        HoldContextMenuControllerPreview::new(
            controller,
            available_size,
            Rc::new(move |_: &mut Window, cx: &mut App| {
                expanding.update(cx, |session, cx| session.expand(cx));
            }),
            Rc::new(move |completion: HoldContextMenuPreviewAction, window: &mut Window, cx: &mut App| {
                session.update(cx, |session, cx| session.commit(completion, window, cx));
            }),
        )
    }
}

/// a pushed chat route, entering from the hold preview pushes this ordinary screen too
#[derive(IntoElement)]
pub struct ChatContextPreviewDestination {
    chat_id: Option<String>,
    all_topics: bool,
    navigation_epoch: Option<Uuid>,
}

impl ChatContextPreviewDestination {
    pub fn new(chat_id: Option<String>, all_topics: bool, navigation_epoch: Option<Uuid>) -> Self {
        Self { chat_id, all_topics, navigation_epoch }
    }
}

impl RenderOnce for ChatContextPreviewDestination {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let content = chat_context_preview_hooks()
            .map(|hooks| (hooks.chat_screen)(self.chat_id.clone(), self.all_topics, self.navigation_epoch, window, cx))
            .unwrap_or_else(|| div().into_any_element());
        ConvertedForumRedirect { chat_id: if self.all_topics { None } else { self.chat_id }, content }
    }
}

fn element_id(name: &str, chat_id: Option<&str>, all_topics: bool) -> ElementId {
    ElementId::Name(format!("chat-context-preview-{name}-{}-{all_topics}", chat_id.unwrap_or_default()).into())
}

#[derive(IntoElement)]
struct ConvertedForumRedirect {
    chat_id: Option<String>,
    content: AnyElement,
}

impl ConvertedForumRedirect {
    /// none until the chat row is known so a forum still loading never reads as just converted
    fn topics_enabled(chat_id: Option<&str>, cx: &mut App) -> Option<bool> {
        let id = Self::resolved_chat_id(chat_id, cx)?;
        let services = ChatServicesStore::shared(cx);
        services.read(cx).chat_info(&id, cx)?;
        Some(services.update(cx, |services, cx| services.topics_enabled(&id, cx)))
    }

    fn resolved_chat_id(chat_id: Option<&str>, cx: &mut App) -> Option<String> {
        let chat_id = chat_id.filter(|id| !id.is_empty())?;
        let services = ChatServicesStore::shared(cx);
        Some(services.read(cx).real_chat_id(chat_id, cx).unwrap_or_else(|| chat_id.to_owned()))
    }

    fn open_forum(chat_id: Option<String>, nav_store: Option<Entity<NavigationStore>>, window: &mut Window, cx: &mut App) {
        let Some(nav_store) = nav_store else {
            return;
        };
        let route = RootRoute::Chat { chat_id: chat_id.clone() };
        if nav_store.read(cx).path.last() != Some(&route) {
            return;
        }
        let Some(id) = Self::resolved_chat_id(chat_id.as_deref(), cx) else {
            return;
        };
        let services = ChatServicesStore::shared(cx);
        let Some(chat) = services.read(cx).chat_info(&id, cx) else {
            return;
        };
        ScreenFocus::release_text_field(window, cx);
        let (current_tab, is_only_route) = {
            let store = nav_store.read(cx);
            (store.current_tab, store.path.len() == 1)
        };
        if current_tab == MainTab::Chats && is_only_route {
            EmbeddedForumPresentationStore::shared(cx).update(cx, |store, cx| store.present(chat, cx));
            nav_store.update(cx, |store, cx| store.pop(cx));
        } else {
            nav_store.update(cx, |store, cx| store.replace_top(RootRoute::ForumHome { chat_id: Some(id) }, cx));
        }
    }
}

impl RenderOnce for ConvertedForumRedirect {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let topics_enabled = Self::topics_enabled(self.chat_id.as_deref(), cx);

        let host = window.current_view();
        let observed_chat_id = self.chat_id.clone();
        let observed = Cell::new(topics_enabled);
        let _observer = window.use_keyed_state(
            element_id("redirect-observer", self.chat_id.as_deref(), false),
            cx,
            move |_, cx| -> Option<[Subscription; 2]> {
                observed_chat_id.as_ref()?;
                let services = ChatServicesStore::shared(cx);
                let observed = Rc::new(observed);
                let follow = {
                    let (chat_id, observed) = (observed_chat_id.clone(), observed.clone());
                    move |cx: &mut App| {
                        let value = Self::topics_enabled(chat_id.as_deref(), cx);
                        if observed.replace(value) != value {
                            cx.notify(host);
                        }
                    }
                };
                let on_info = follow.clone();
                Some([
                    App::observe(cx, &services, move |_, cx: &mut App| follow(cx)),
                    ChatServicesStore::subscribe_info_keys(observed_chat_id.clone(), cx, on_info),
                ])
            },
        );

        let chat_id = self.chat_id.clone();
        on_change(
            element_id("redirect", self.chat_id.as_deref(), false),
            topics_enabled,
            false,
            window,
            cx,
            move |was_enabled: &Option<bool>, is_enabled: &Option<bool>, window: &mut Window, cx: &mut App| {
                if *was_enabled != Some(false) || *is_enabled != Some(true) {
                    return;
                }
                Self::open_forum(chat_id, nav_store, window, cx);
            },
        );
        self.content
    }
}
