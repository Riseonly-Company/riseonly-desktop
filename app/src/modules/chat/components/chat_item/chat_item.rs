use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, RenderOnce, Size, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_i18n::language_store::Locale;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::native_bottom_sheet_presentation_host::NativeBottomSheetPresentationHost;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::bubble_scroll_passthrough::BubbleScrollPassthroughModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::environment::{EnvKey, env};
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::stacks::hstack;

use crate::core::lib::canon_format::{CalendarBand, RiseFormat};
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerLiveState;
use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::{
    HoldContextMenuContainer, HoldContextMenuControllerPreviewFactory,
};
use crate::modules::chat::components::chat_item::chat_context_preview::ChatContextPreview;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::ChatWallpaperSheet;
use crate::modules::chat::components::chat::chat_logo::ChatLogo;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftPreview;
use crate::modules::chat::components::chat_item::chat_list_preview_row::{
    ChatListPreviewFactory, ChatListPreviewRow, ChatListRowMetrics,
};
use crate::modules::chat::components::chat_item::chat_list_row_activity::{ChatListRowActivity, ChatListRowActivityKey};
use crate::modules::chat::pages::chats::embedded_forum_transition::{
    EmbeddedForumRailAvatarMotion, EmbeddedForumRailContext, EmbeddedForumRailContextKey,
};
use crate::modules::chat::shared::chat_presentation_policy::ChatPresentationPolicy;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::notify::components::notifications_hub_sheet_store::NotificationsHubSheetStore;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const AUTO_DELETE_ONE_DAY: i64 = 86_400;
const AUTO_DELETE_ONE_WEEK: i64 = 604_800;
const AUTO_DELETE_ONE_MONTH: i64 = 2_592_000;

/// carrier of the suppression flag, not the flag itself
/// it is read in the tap handler and nothing draws from it
/// a bool in the environment would rerender on every folder swipe
pub struct ChatsListRowActionsSuppressorKey;

impl EnvKey for ChatsListRowActionsSuppressorKey {
    type Value = Option<Entity<AnimatedTabsPagerLiveState>>;

    fn default_value() -> Self::Value {
        None
    }
}

pub type ChatItemAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type ChatItemHoldAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;
pub type ChatItemReorderChanged = Rc<dyn Fn(f32, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ChatItem {
    id: ElementId,
    chat: ChatInfo,
    draft: Option<ChatDraftPreview>,
    is_active: bool,
    measurement_only: bool,
    guide_anchor: Option<RiseGuideAnchor>,
    on_press: ChatItemAction,
    on_hold_action: Option<ChatItemHoldAction>,
    custom_hold_actions: Option<Vec<HoldContextMenuAction>>,
    additional_hold_actions: Vec<HoldContextMenuAction>,
    disabled_hold_action_ids: HashSet<String>,
    archived: bool,
    supports_selection: bool,
    reorder_mode: bool,
    pinned_count: i64,
    is_selected: bool,
    on_toggle_select: ChatItemAction,
    on_reorder_changed: Option<ChatItemReorderChanged>,
    on_reorder_ended: Option<ChatItemAction>,
}

impl ChatItem {
    pub fn new(id: impl Into<ElementId>, chat: ChatInfo) -> Self {
        Self {
            id: id.into(),
            chat,
            draft: None,
            is_active: true,
            measurement_only: false,
            guide_anchor: None,
            on_press: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_hold_action: None,
            custom_hold_actions: None,
            additional_hold_actions: Vec::new(),
            disabled_hold_action_ids: HashSet::new(),
            archived: false,
            supports_selection: false,
            reorder_mode: false,
            pinned_count: 0,
            is_selected: false,
            on_toggle_select: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_reorder_changed: None,
            on_reorder_ended: None,
        }
    }

    pub fn draft(mut self, draft: Option<ChatDraftPreview>) -> Self {
        self.draft = draft;
        self
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    pub fn measurement_only(mut self, measurement_only: bool) -> Self {
        self.measurement_only = measurement_only;
        self
    }

    pub fn guide_anchor(mut self, guide_anchor: Option<RiseGuideAnchor>) -> Self {
        self.guide_anchor = guide_anchor;
        self
    }

    pub fn on_press(mut self, on_press: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Rc::new(on_press);
        self
    }

    pub fn on_hold_action(mut self, on_hold_action: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_hold_action = Some(Rc::new(on_hold_action));
        self
    }

    pub fn custom_hold_actions(mut self, custom_hold_actions: Option<Vec<HoldContextMenuAction>>) -> Self {
        self.custom_hold_actions = custom_hold_actions;
        self
    }

    pub fn additional_hold_actions(mut self, additional_hold_actions: Vec<HoldContextMenuAction>) -> Self {
        self.additional_hold_actions = additional_hold_actions;
        self
    }

    pub fn disabled_hold_action_ids(mut self, disabled_hold_action_ids: HashSet<String>) -> Self {
        self.disabled_hold_action_ids = disabled_hold_action_ids;
        self
    }

    pub fn archived(mut self, archived: bool) -> Self {
        self.archived = archived;
        self
    }

    pub fn supports_selection(mut self, supports_selection: bool) -> Self {
        self.supports_selection = supports_selection;
        self
    }

    pub fn reorder_mode(mut self, reorder_mode: bool) -> Self {
        self.reorder_mode = reorder_mode;
        self
    }

    pub fn pinned_count(mut self, pinned_count: i64) -> Self {
        self.pinned_count = pinned_count;
        self
    }

    pub fn is_selected(mut self, is_selected: bool) -> Self {
        self.is_selected = is_selected;
        self
    }

    pub fn on_toggle_select(mut self, on_toggle_select: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle_select = Rc::new(on_toggle_select);
        self
    }

    pub fn on_reorder_changed(mut self, on_reorder_changed: Option<ChatItemReorderChanged>) -> Self {
        self.on_reorder_changed = on_reorder_changed;
        self
    }

    pub fn on_reorder_ended(mut self, on_reorder_ended: Option<ChatItemAction>) -> Self {
        self.on_reorder_ended = on_reorder_ended;
        self
    }

    /// the measuring copy of a row is mounted next to the live one so its state keys differ
    fn child_id(&self, name: &str) -> ElementId {
        let kind = if self.measurement_only { "measure" } else { "row" };
        ElementId::NamedChild(Arc::new(self.id.clone()), format!("{kind}-{name}").into())
    }

    /// carrier comes from render, the environment stack is gone by the time a tap handler runs
    fn is_suppressed(suppressor: Option<&Entity<AnimatedTabsPagerLiveState>>, cx: &App) -> bool {
        suppressor.is_some_and(|suppressor| suppressor.read(cx).suppresses_tab_taps())
    }

    fn resolved_hold_actions(&self, mute_all_chats_enabled: bool, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let Some(custom_hold_actions) = self.custom_hold_actions.clone() else {
            return ChatListMenuPolicy::actions(
                &self.chat,
                self.pinned_count,
                mute_all_chats_enabled,
                self.supports_selection,
                self.archived,
                self.additional_hold_actions.clone(),
                &self.disabled_hold_action_ids,
            );
        };
        ChatListMenuPolicy::applying(
            self.additional_hold_actions.clone(),
            &self.disabled_hold_action_ids,
            custom_hold_actions,
        )
    }

    fn motion(&self, name: &str, target: bool, window: &mut Window, cx: &mut App) -> f32 {
        let target = if target { 1.0_f32 } else { 0.0_f32 };
        let motion = window.use_keyed_state(self.child_id(name), cx, move |_, _| Animated::new(target));
        let now = RiseClock::monotonic();
        motion.update(cx, |animated, cx| {
            animated.set_in(target, spring(0.34, 0.84), now, cx);
            animated.frame_value(now, window)
        })
    }

    fn chat_row(&self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let checkbox_progress = self.motion("reorder-checkbox", self.reorder_mode, window, cx);
        let handle_progress = self.motion("reorder-handle", self.reorder_mode && self.chat.is_pinned, window, cx);
        let presentation = ChatListPreviewFactory::chat(&self.chat, self.draft.as_ref(), true, cx);
        let avatar = self.chat_avatar(cx);
        let leading = hstack(0.0)
            .child(
                div()
                    .flex_none()
                    .w(px(38.0 * checkbox_progress))
                    .opacity(checkbox_progress)
                    .overflow_hidden()
                    .child(self.reorder_checkbox(theme)),
            )
            .child(EmbeddedForumRailAvatarMotion::new(
                ChatListRowMetrics::AVATAR_SIZE,
                ChatListRowMetrics::HORIZONTAL_PADDING,
                avatar,
            ));
        let row = ChatListPreviewRow::new(self.child_id("preview"), presentation)
            .leading(leading)
            .rail_avatar_size(ChatListRowMetrics::AVATAR_SIZE)
            .measurement_only(self.measurement_only)
            .typing_chat_id((self.chat.r#type != ChatType::Favourites).then(|| self.chat.id.clone()))
            .typing_shows_names(self.chat.r#type != ChatType::Direct)
            .is_active(self.is_active);

        let did_drag = window.use_keyed_state(self.child_id("reorder-did-drag"), cx, |_, _| Rc::new(Cell::new(false)));
        let did_drag = did_drag.read(cx).clone();
        let reset_did_drag = did_drag.clone();
        let tap_did_drag = did_drag.clone();
        let reorder_mode = self.reorder_mode;
        let on_toggle_select = self.on_toggle_select.clone();

        hstack(0.0)
            .id(self.child_id("row"))
            .w_full()
            .bg(theme.bg._100)
            .child(div().flex_1().min_w_0().child(row))
            .child(
                div()
                    .flex_none()
                    .w(px(44.0 * handle_progress))
                    .opacity(handle_progress)
                    .overflow_hidden()
                    .child(self.reorder_handle(theme, did_drag)),
            )
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _: &mut Window, _: &mut App| {
                reset_did_drag.set(false);
            })
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                if !reorder_mode || tap_did_drag.get() {
                    return;
                }
                on_toggle_select(window, cx);
            })
            .into_any_element()
    }

    fn chat_avatar(&self, cx: &mut App) -> AnyElement {
        if self.measurement_only {
            return div()
                .size(px(ChatListRowMetrics::AVATAR_SIZE))
                .flex_none()
                .into_any_element();
        }
        let logo = ChatLogo::new(self.child_id("logo"), self.chat.clone())
            .size(ChatListRowMetrics::AVATAR_SIZE)
            .icon_size(22.0)
            .shows_worn_frame(true)
            .letter_fallback(tr("chat_untitled"));
        if self.reorder_mode || self.chat.r#type != ChatType::Direct || self.chat.participant.id.is_empty() {
            return logo.into_any_element();
        }

        let user_id = self.chat.participant.id.clone();
        let on_press = self.on_press.clone();
        let open_user_id = user_id.clone();
        let suppressor = env::<ChatsListRowActionsSuppressorKey>(cx);
        let rail = env::<EmbeddedForumRailContextKey>(cx);
        let tappable = div()
            .id(self.child_id("avatar"))
            .relative()
            .rounded_full()
            .block_mouse_except_scroll()
            .child(logo)
            .bubble_scroll_passthrough(self.child_id("avatar-scroll-passthrough"))
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                Self::open_chat_avatar(&open_user_id, suppressor.as_ref(), rail.as_ref(), &on_press, window, cx);
            });
        ChatListStoryActivity {
            id: self.child_id("story-activity"),
            user_id,
            is_active: self.is_active,
            content: tappable.into_any_element(),
        }
        .into_any_element()
    }

    fn open_chat_avatar(
        user_id: &str,
        suppressor: Option<&Entity<AnimatedTabsPagerLiveState>>,
        rail: Option<&EmbeddedForumRailContext>,
        on_press: &ChatItemAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        if Self::is_suppressed(suppressor, cx) {
            return;
        }
        if rail.is_some_and(|rail| rail.progress(cx) > 0.0) {
            on_press(window, cx);
            return;
        }
        let has_active_stories = StoryServicesStore::shared(cx).read(cx).has_active_stories(Some(user_id), cx);
        if has_active_stories {
            let _ = selection_changed();
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.open_stories_for_user_id(user_id, None, cx));
            return;
        }
        on_press(window, cx);
    }

    fn reorder_checkbox(&self, theme: &AppTheme) -> AnyElement {
        let stroke = if self.is_selected { theme.primary._100 } else { theme.text.secondary.opacity(0.5) };
        div()
            .flex_none()
            .w(px(26.0))
            .h(px(50.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .relative()
                    .size(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .rounded_full()
                            .bg(theme.primary._100)
                            .opacity(if self.is_selected { 1.0 } else { 0.0 }),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(-0.75))
                            .top(px(-0.75))
                            .size(px(23.5))
                            .rounded_full()
                            .border(px(1.5))
                            .border_color(stroke),
                    )
                    .child(
                        div()
                            .opacity(if self.is_selected { 1.0 } else { 0.0 })
                            .children(IconUi::sized("checkmark", px(12.0), theme.content_on(theme.primary._100), false)),
                    ),
            )
            .into_any_element()
    }

    fn reorder_handle(&self, theme: &AppTheme, did_drag: Rc<Cell<bool>>) -> AnyElement {
        let icon = div()
            .flex_none()
            .w(px(32.0))
            .h(px(50.0))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("line.3.horizontal", px(18.0), theme.text.secondary, false));
        if !self.reorder_mode || !self.chat.is_pinned {
            return icon.into_any_element();
        }
        let on_changed = self.on_reorder_changed.clone();
        let on_ended = self.on_reorder_ended.clone();
        icon.gesture(
            self.child_id("reorder-drag"),
            DragGesture::new(2.0)
                .on_changed(move |value: &DragGestureValue, window: &mut Window, cx: &mut App| {
                    if !did_drag.get() {
                        tracing::warn!(target: "riseonly::X2051", has_handler = on_changed.is_some(), "reorder handle drag");
                    }
                    did_drag.set(true);
                    if let Some(on_changed) = on_changed.as_ref() {
                        on_changed(f32::from(value.translation.height), window, cx);
                    }
                })
                .on_ended(move |_: &DragGestureValue, window: &mut Window, cx: &mut App| {
                    if let Some(on_ended) = on_ended.as_ref() {
                        on_ended(window, cx);
                    }
                }),
        )
        .into_any_element()
    }

    fn on_action(
        chat: &ChatInfo,
        on_hold_action: Option<&ChatItemHoldAction>,
        show_wallpaper: &Entity<bool>,
        id: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        if id == "notifications" {
            let target_user_id = (chat.r#type == ChatType::Direct && !chat.is_bot_chat())
                .then(|| chat.participant.id.clone());
            let chat_scopes = chat.notification_scopes();
            NotificationsHubSheetStore::shared(cx).update(cx, |store, cx| {
                store.present(Some(chat.id.as_str()), Some(chat_scopes), target_user_id.as_deref(), cx);
            });
        } else if id == "wallpaper" {
            show_wallpaper.update(cx, |show_wallpaper, cx| {
                *show_wallpaper = true;
                cx.notify();
            });
        } else if let Some(on_hold_action) = on_hold_action {
            on_hold_action(&id, window, cx);
        }
    }

    fn wallpaper_sheet(&self, show_wallpaper: Entity<bool>) -> NativeBottomSheetPresentationHost {
        let reader = show_wallpaper.clone();
        let chat = self.chat.clone();
        NativeBottomSheetPresentationHost::new(
            self.child_id("wallpaper-sheet"),
            Binding::new(
                move |cx| *reader.read(cx),
                move |shown, _, cx| {
                    show_wallpaper.update(cx, |show_wallpaper, cx| {
                        *show_wallpaper = shown;
                        cx.notify();
                    });
                },
            ),
            move |_, _| ChatWallpaperSheet::new(chat.id.clone(), chat.clone()),
        )
        .presentation_detents([])
        .presentation_drag_indicator(true)
    }
}

impl RenderOnce for ChatItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        if self.measurement_only {
            return self.chat_row(&theme, window, cx);
        }

        let mute_all = window.use_keyed_state(self.child_id("mute-all"), cx, |_, cx| ChatItemMuteAllState::new(cx));
        let mute_all_chats_enabled = mute_all.read(cx).enabled;

        let show_wallpaper = window.use_keyed_state(self.child_id("show-wallpaper"), cx, |_, _| false);
        let sheet = self.wallpaper_sheet(show_wallpaper.clone());
        let on_press = self.on_press.clone();
        let reorder_mode = self.reorder_mode;
        let source_identity = self.chat.id.clone();
        let container_id = self.child_id("hold");
        let allows_menu = ChatPresentationPolicy::allows_list_context_menu(self.chat.r#type);
        let resolved_hold_actions = self.resolved_hold_actions(mute_all_chats_enabled, cx);
        let suppressor = env::<ChatsListRowActionsSuppressorKey>(cx);
        let preview_chat_id = self.chat.id.clone();
        let peeks = !reorder_mode && !resolved_hold_actions.is_empty();
        let controller_preview = env::<NavigationStoreKey>(cx).filter(|_| peeks).map(|navigation| {
            let factory: HoldContextMenuControllerPreviewFactory =
                Rc::new(move |size: Size<Pixels>, _: &mut Window, cx: &mut App| {
                    ChatContextPreview::make_with_chat_id(&preview_chat_id, None, size, navigation.clone(), cx)
                });
            factory
        });
        let this = Rc::new(self);

        let body = if allows_menu {
            let actions = if reorder_mode {
                Vec::new()
            } else {
                resolved_hold_actions
            };
            let row = this.clone();
            let action = this.clone();
            HoldContextMenuContainer::new(
                container_id,
                HoldContextMenuAlignment::Leading,
                Rc::new(move |_: &App| actions.clone()),
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let theme = rise_ui::theme(cx).clone();
                    row.chat_row(&theme, window, cx).guide_anchor(row.guide_anchor).into_any_element()
                }),
            )
            .show_emoji_list(false)
            .disabled(reorder_mode)
            .source_identity(Some(source_identity))
            .controller_preview(controller_preview)
            .on_action(Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
                Self::on_action(
                    &action.chat,
                    action.on_hold_action.as_ref(),
                    &show_wallpaper,
                    id,
                    window,
                    cx,
                );
            }))
            .on_tap(Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| {
                if Self::is_suppressed(suppressor.as_ref(), cx) {
                    return;
                }
                on_press(window, cx);
            }))
            .into_any_element()
        } else {
            div()
                .id(this.child_id("plain"))
                .w_full()
                .child(this.chat_row(&theme, window, cx).guide_anchor(this.guide_anchor))
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    if reorder_mode || Self::is_suppressed(suppressor.as_ref(), cx) {
                        return;
                    }
                    on_press(window, cx);
                })
                .into_any_element()
        };
        div().w_full().child(body).child(sheet).into_any_element()
    }
}

struct ChatItemMuteAllState {
    enabled: bool,
    _observer: Subscription,
}

impl ChatItemMuteAllState {
    fn new(cx: &mut Context<Self>) -> Self {
        let users = UserServicesStore::shared(cx);
        let observer = cx.observe(&users, |this, users, cx| {
            let enabled = users.read(cx).mute_all_chats_enabled();
            if this.enabled != enabled {
                this.enabled = enabled;
                cx.notify();
            }
        });
        let enabled = users.read(cx).mute_all_chats_enabled();
        Self { enabled, _observer: observer }
    }
}

struct ChatListStoryActivityGate {
    activity_active: bool,
    rendered_active: bool,
    _observer: Option<Subscription>,
}

impl ChatListStoryActivityGate {
    fn new(row_activity: Option<Entity<ChatListRowActivity>>, cx: &mut Context<Self>) -> Self {
        let observer = row_activity.map(|activity| {
            let (initial, this) = (activity.clone(), cx.weak_entity());
            cx.defer(move |cx| {
                let active = initial.read(cx).is_active();
                let _ = this.update(cx, |this, cx| {
                    if this.activity_active != active {
                        this.activity_active = active;
                        if active && !this.rendered_active {
                            cx.notify();
                        }
                    }
                });
            });
            cx.observe(&activity, |this, activity, cx| {
                let active = activity.read(cx).is_active();
                if this.activity_active == active {
                    return;
                }
                this.activity_active = active;
                if active && !this.rendered_active {
                    cx.notify();
                }
            })
        });
        Self { activity_active: true, rendered_active: true, _observer: observer }
    }
}

#[derive(IntoElement)]
struct ChatListStoryActivity {
    id: ElementId,
    user_id: String,
    is_active: bool,
    content: AnyElement,
}

impl RenderOnce for ChatListStoryActivity {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let row_activity = env::<ChatListRowActivityKey>(cx);
        let observer_id = ElementId::NamedChild(Arc::new(self.id.clone()), "activity-observer".into());
        let gate = window.use_keyed_state(observer_id, cx, move |_, cx| ChatListStoryActivityGate::new(row_activity, cx));
        let activity_active = gate.update(cx, |gate, _| {
            gate.rendered_active = gate.activity_active;
            gate.activity_active
        });
        let active = self.is_active && activity_active;
        let user_id = self.user_id;
        let task_id = active.then(|| user_id.clone());
        use_task(self.id, task_id, window, cx, async move |cx| {
            if !active {
                return;
            }
            let _ = cx.update(|_, cx| {
                let is_known = StoryServicesStore::shared(cx).read(cx).has_known_stories_state(Some(&user_id), cx);
                if is_known {
                    return;
                }
                StoryInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_stories_state(std::slice::from_ref(&user_id), cx));
            });
        });
        self.content
    }
}

pub enum ChatListMenuPolicy {}

impl ChatListMenuPolicy {
    fn pin_action(chat: &ChatInfo) -> HoldContextMenuAction {
        HoldContextMenuAction::new("pin", if chat.is_pinned { tr("ctx_unpin") } else { tr("ctx_pin") })
            .system_image(if chat.is_pinned { "pin.slash" } else { "pin" })
    }

    fn notifications_action(chat: &ChatInfo) -> HoldContextMenuAction {
        HoldContextMenuAction::new("notifications", tr("ctx_notifications"))
            .system_image(if chat.is_muted { "bell.slash" } else { "bell" })
    }

    fn wallpaper_action() -> HoldContextMenuAction {
        HoldContextMenuAction::new("wallpaper", tr("ctx_wallpaper")).system_image("photo.on.rectangle.angled")
    }

    fn mute_action(chat: &ChatInfo) -> HoldContextMenuAction {
        HoldContextMenuAction::new("mute", if chat.is_muted { tr("chat_unmute_exception") } else { tr("ctx_mute") })
            .system_image(if chat.is_muted { "bell" } else { "bell.slash" })
            .guide_anchor(RiseGuideAnchor::ChatMenuMuteException)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn actions(
        chat: &ChatInfo,
        pinned_count: i64,
        mute_all_chats_enabled: bool,
        supports_selection: bool,
        archived: bool,
        additional_actions: Vec<HoldContextMenuAction>,
        disabled_action_ids: &HashSet<String>,
    ) -> Vec<HoldContextMenuAction> {
        if chat.r#type == ChatType::Favourites {
            return Self::applying(
                additional_actions,
                disabled_action_ids,
                vec![
                    Self::pin_action(chat),
                    HoldContextMenuAction::new("delete", tr("chat_clear_favourites_action"))
                        .system_image("trash")
                        .destructive(true),
                ],
            );
        }

        if chat.is_official_bot_chat() {
            let mut items = vec![Self::notifications_action(chat), Self::wallpaper_action()];
            if mute_all_chats_enabled {
                items.push(Self::mute_action(chat));
            }
            items.push(
                HoldContextMenuAction::new("delete", tr("chat_clear_history_action"))
                    .system_image("trash")
                    .destructive(true),
            );
            return Self::applying(Vec::new(), disabled_action_ids, items);
        }

        let mut items = Vec::new();
        let auto_delete = chat.auto_delete_messages_after.unwrap_or(0);
        if chat.unread_count > 0
            || chat.unread_mentions_count > 0
            || chat.unread_replies_count > 0
            || chat.unread_reactions_count > 0
        {
            items.push(HoldContextMenuAction::new("mark_read", tr("ctx_mark_read")).system_image("checkmark.circle"));
        } else {
            items.push(HoldContextMenuAction::new("mark_unread", tr("ctx_mark_unread")).system_image("envelope.badge"));
        }
        if !archived {
            items.push(Self::pin_action(chat));
        }
        items.push(Self::notifications_action(chat));
        items.push(Self::wallpaper_action());
        items.push(
            HoldContextMenuAction::new("autodelete", tr("chat_autodelete_title")).system_image("timer").submenu(vec![
                HoldContextMenuAction::new("autodelete_off", tr("chat_autodelete_off")).checked(auto_delete <= 0),
                HoldContextMenuAction::new("autodelete_1d", tr("chat_autodelete_1d"))
                    .checked(auto_delete == AUTO_DELETE_ONE_DAY),
                HoldContextMenuAction::new("autodelete_1w", tr("chat_autodelete_1w"))
                    .checked(auto_delete == AUTO_DELETE_ONE_WEEK),
                HoldContextMenuAction::new("autodelete_1m", tr("chat_autodelete_1m"))
                    .checked(auto_delete == AUTO_DELETE_ONE_MONTH),
            ]),
        );
        items.push(if archived {
            HoldContextMenuAction::new("unarchive", tr("chat_archive_restore")).system_image("archivebox")
        } else {
            HoldContextMenuAction::new("archive", tr("ctx_archive")).system_image("archivebox")
        });
        if mute_all_chats_enabled {
            items.push(Self::mute_action(chat));
        }
        if chat.is_pinned && pinned_count > 1 {
            items.push(HoldContextMenuAction::new("reorder", tr("ctx_reorder")).system_image("arrow.up.arrow.down"));
        }
        if (chat.r#type == ChatType::Group || chat.r#type == ChatType::Channel) && !ChatProfilePermissions::is_owner(chat) {
            let title = if chat.r#type == ChatType::Channel { tr("chat_leave_channel") } else { tr("chat_leave_group") };
            items.push(
                HoldContextMenuAction::new("leave", title)
                    .system_image("rectangle.portrait.and.arrow.right")
                    .destructive(true),
            );
        } else {
            if chat.r#type == ChatType::Direct {
                items.push(
                    HoldContextMenuAction::new("history_clear", tr("chat_clear_history_action"))
                        .system_image("clear")
                        .destructive(true),
                );
            }
            items.push(HoldContextMenuAction::new("delete", tr("chat_delete_text")).system_image("trash").destructive(true));
        }
        if supports_selection {
            items.push(
                HoldContextMenuAction::new("select", tr("ctx_select")).system_image("checkmark.circle").section_break(true),
            );
        }
        Self::applying(additional_actions, disabled_action_ids, items)
    }

    pub fn applying(
        additional_actions: Vec<HoldContextMenuAction>,
        disabled_action_ids: &HashSet<String>,
        base_actions: Vec<HoldContextMenuAction>,
    ) -> Vec<HoldContextMenuAction> {
        base_actions
            .into_iter()
            .chain(additional_actions)
            .map(|mut item| {
                if disabled_action_ids.contains(&item.id) {
                    item.disabled = true;
                }
                item
            })
            .collect()
    }
}

pub enum ChatListTimestampFormatter {}

impl ChatListTimestampFormatter {
    pub fn string(timestamp: TimeInterval, locale: &Locale, calendar: &Calendar) -> String {
        let date = Date::from_time_interval_since_1970(timestamp);
        let now = Date::now();
        let locale = locale.identifier();

        match RiseFormat::calendar_band(&date, &now, calendar) {
            CalendarBand::Today => RiseFormat::clock_date(&date, calendar, RiseFormat::uses_twelve_hour_clock()),
            CalendarBand::Yesterday => RiseFormat::day_label(&date, &now, calendar, locale, true),
            CalendarBand::ThisYear | CalendarBand::Older => RiseFormat::day_label(&date, &now, calendar, locale, false),
        }
    }
}
