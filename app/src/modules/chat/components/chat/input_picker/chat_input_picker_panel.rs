use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, Bounds, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, Pixels, Point, Render,
    RenderOnce, SharedString, Stateful, StyleRefinement, Subscription, Task, Window, black, canvas, white,
};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore};
use rise_ui::IconUi;
use rise_ui::animated_media_playback_scope::AnimatedMediaSettleProbe;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig};
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::HoldContextMenuGlass;
use rise_ui::liquid_chrome::LiquidChrome;
use rise_ui::scroll_edge_blur_ui::ScrollEdgeBlurFalloff;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition, UnitPoint};

use crate::app::router::navigation_fullscreen_pop::NavigationInteractivePopGuard;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::{AnimatedTabsPager, AnimatedTabsPagerLiveState};
use crate::modules::chat::components::chat::input_picker::chat_input_emoji_tab::ChatInputEmojiTab;
use crate::modules::chat::components::chat::input_picker::chat_input_gif_tab::ChatInputGifTab;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::{
    ChatInputPickerBarReserveKey, ChatInputPickerMetrics, ChatInputSegmentedTabs,
};
use crate::modules::chat::components::chat::input_picker::chat_input_stickers_tab::ChatInputStickersTab;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::StickerHoldPreviewPresenter;
use crate::modules::stickers::engine::rise_sticker_models::RiseStickerPickerTab;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{GifItem, PackKind, StickerItem};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;

type SelectEmoji = Rc<dyn Fn(String, &mut Window, &mut App)>;
type SelectSticker = Rc<dyn Fn(StickerItem, &mut Window, &mut App)>;
type SendGif = Rc<dyn Fn(GifItem, &mut Window, &mut App)>;
type PanelAction = Rc<dyn Fn(&mut Window, &mut App)>;
type OpenPackSettings = Rc<dyn Fn(PackKind, &mut Window, &mut App)>;
type TabShown = Rc<dyn Fn(Option<RiseStickerPickerTab>, &mut App)>;

const TAB_GIF: i64 = 0;
const TAB_STICKERS: i64 = 1;
const TAB_EMOJI: i64 = 2;
const PANEL_CORNER_RADIUS: f32 = 28.0;
const NEIGHBOUR_MOUNT_DELAY: Duration = Duration::from_millis(350);
const BAR_SPRING: (f64, f64) = (0.35, 0.86);
const BAR_SCRIM_LEAD: f32 = 36.0;
const BAR_SHADOW_PEAK: f32 = 0.92;

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn stickers_ordinal() -> i64 {
    i64::from(RiseStickerPickerTab::Stickers.ordinal())
}

/// panel reopens on the tab it was closed on, in every composer that hosts it
enum ChatInputPickerLastTab {}

static CHAT_INPUT_PICKER_LAST_TAB: AtomicI64 = AtomicI64::new(TAB_STICKERS);

impl ChatInputPickerLastTab {
    fn value() -> i64 {
        CHAT_INPUT_PICKER_LAST_TAB.load(Ordering::Relaxed)
    }

    fn set_value(value: i64) {
        CHAT_INPUT_PICKER_LAST_TAB.store(value, Ordering::Relaxed);
    }
}

fn picker_tab(value: i64) -> RiseStickerPickerTab {
    usize::try_from(value)
        .ok()
        .and_then(|index| RiseStickerPickerTab::ALL.get(index).copied())
        .unwrap_or(RiseStickerPickerTab::Stickers)
}

fn bar_transition() -> Transition {
    Transition::scale_default().combined(Transition::opacity())
}

fn tabs(places_on_media: bool) -> Vec<TabHeaderConfig> {
    let all = vec![
        TabHeaderConfig::new(TAB_GIF, tr("picker_tab_gif")),
        TabHeaderConfig::new(TAB_STICKERS, tr("picker_tab_stickers")),
        TabHeaderConfig::new(TAB_EMOJI, tr("picker_tab_emoji")),
    ];
    if places_on_media { all.into_iter().filter(|tab| tab.id != TAB_GIF).collect() } else { all }
}

fn first_tab_id(places_on_media: bool) -> i64 {
    tabs(places_on_media).first().map_or(TAB_GIF, |tab| tab.id)
}

/// every touch that starts inside the picker belongs to the picker
/// a held sticker dragged sideways must not start the back swipe of the chat underneath
#[derive(Clone)]
struct ChatInputPickerPopGuard {
    owner: Uuid,
    frame: Rc<Cell<Bounds<Pixels>>>,
}

impl ChatInputPickerPopGuard {
    fn new() -> Self {
        Self { owner: Uuid::new_v4(), frame: Rc::new(Cell::new(Bounds::default())) }
    }

    fn claim(&self, cx: &mut App) {
        let frame = Rc::downgrade(&self.frame);
        NavigationInteractivePopGuard::set_profile_tab_paging_back(
            true,
            self.owner,
            Some(Rc::new(move |point: Point<Pixels>| frame.upgrade().is_some_and(|frame| frame.get().contains(&point)))),
            cx,
        );
    }

    fn release(&self, cx: &mut App) {
        NavigationInteractivePopGuard::set_profile_tab_paging_back(false, self.owner, None, cx);
    }
}

struct ChatInputPickerPanelState {
    active_tab: i64,
    page_width: f32,
    mounted_tabs: HashSet<i64>,
    pager_live_state: Entity<AnimatedTabsPagerLiveState>,
    neighbour_mount_task: Option<Task<()>>,
    gear_presence: Presence<()>,
    backspace_presence: Presence<()>,
    pop_guard: ChatInputPickerPopGuard,
    places_on_media: bool,
    on_tab_shown: Option<TabShown>,
    page_callbacks: Rc<RefCell<Option<PickerPageCallbacks>>>,
    hold_was_active: bool,
    _hold_presenter_observation: Subscription,
}

impl ChatInputPickerPanelState {
    fn new(places_on_media: bool, cx: &mut Context<Self>) -> Self {
        let remembered = if places_on_media { stickers_ordinal() } else { ChatInputPickerLastTab::value() };
        let hold_presenter = StickerHoldPreviewPresenter::shared(cx);
        Self {
            active_tab: remembered,
            page_width: 0.0,
            mounted_tabs: HashSet::from([remembered]),
            pager_live_state: cx.new(|_| AnimatedTabsPagerLiveState::new()),
            neighbour_mount_task: None,
            gear_presence: Presence::new(Self::shows_gear(remembered, places_on_media).then_some(()))
                .with_transition(bar_transition()),
            backspace_presence: Presence::new(Self::shows_backspace(remembered, places_on_media).then_some(()))
                .with_transition(bar_transition()),
            pop_guard: ChatInputPickerPopGuard::new(),
            places_on_media,
            on_tab_shown: None,
            page_callbacks: Rc::new(RefCell::new(None)),
            hold_was_active: hold_presenter.read(cx).is_active(),
            _hold_presenter_observation: cx.observe(&hold_presenter, |this, presenter, cx| {
                let is_active = presenter.read(cx).is_active();
                if is_active != this.hold_was_active {
                    this.hold_was_active = is_active;
                    cx.notify();
                }
            }),
        }
    }

    fn shows_gear(tab: i64, places_on_media: bool) -> bool {
        !places_on_media && (tab == TAB_STICKERS || tab == TAB_EMOJI)
    }

    fn shows_backspace(tab: i64, places_on_media: bool) -> bool {
        !places_on_media && tab == TAB_EMOJI
    }

    fn tab_shown(state: &Entity<Self>, tab: Option<RiseStickerPickerTab>, cx: &mut App) {
        if let Some(on_tab_shown) = state.read(cx).on_tab_shown.clone() {
            on_tab_shown(tab, cx);
        }
    }

    /// mounting a neighbour tab builds a full sticker or emoji grid
    /// doing it from the drag callback hit the first swipe frame so it waits until the page settles
    fn schedule_neighbour_mount(&mut self, cx: &mut Context<Self>) {
        self.neighbour_mount_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NEIGHBOUR_MOUNT_DELAY).await;
            let _ = this.update(cx, |state, cx| {
                state.neighbour_mount_task = None;
                for candidate in [state.active_tab - 1, state.active_tab + 1] {
                    if tabs(state.places_on_media).iter().any(|tab| tab.id == candidate) {
                        state.mounted_tabs.insert(candidate);
                    }
                }
                cx.notify();
            });
        }));
    }

    /// pager binding and segmented tabs both write through here so one write runs it once
    fn set_active_tab(state: &Entity<Self>, value: i64, cx: &mut App) {
        let changed = state.update(cx, |state, cx| {
            if state.active_tab == value {
                return false;
            }
            state.active_tab = value;
            if !state.places_on_media {
                ChatInputPickerLastTab::set_value(value);
            }
            state.mounted_tabs.insert(value);
            state.schedule_neighbour_mount(cx);
            let now = RiseClock::monotonic();
            let bar_spring = spring(BAR_SPRING.0, BAR_SPRING.1);
            let places_on_media = state.places_on_media;
            state.gear_presence.set_in(Self::shows_gear(value, places_on_media).then_some(()), bar_spring, now, cx);
            state.backspace_presence.set_in(
                Self::shows_backspace(value, places_on_media).then_some(()),
                bar_spring,
                now,
                cx,
            );
            cx.notify();
            true
        });
        if changed {
            StickersInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.picker_did_select(picker_tab(value), cx));
            Self::tab_shown(state, Some(picker_tab(value)), cx);
            GuideFacts::set(RiseGuideSignal::StickerPickerOpen, value == TAB_STICKERS, cx);
        }
    }

    fn did_appear(state: &Entity<Self>, cx: &mut App) {
        state.read(cx).pop_guard.clone().claim(cx);
        let active_tab = state.update(cx, |state, cx| {
            state.mounted_tabs.insert(state.active_tab);
            state.schedule_neighbour_mount(cx);
            cx.notify();
            state.active_tab
        });
        StickersInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.picker_did_open(picker_tab(active_tab), cx));
        Self::tab_shown(state, Some(picker_tab(active_tab)), cx);
        GuideFacts::set(RiseGuideSignal::StickerPickerOpen, active_tab == TAB_STICKERS, cx);
    }

    fn did_disappear(state: &Entity<Self>, cx: &mut App) {
        state.read(cx).pop_guard.clone().release(cx);
        GuideFacts::set(RiseGuideSignal::StickerPickerOpen, false, cx);
        state.update(cx, |state, _| {
            state.neighbour_mount_task = None;
            state.mounted_tabs = HashSet::from([stickers_ordinal()]);
        });
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.picker_did_close(cx));
        Self::tab_shown(state, None, cx);
    }
}

/// the picker panel above the composer
/// it leaves the keyboard with the text field so typing is never interrupted
/// a tab changes by click or trackpad swipe
#[derive(IntoElement, Clone)]
pub struct ChatInputPickerPanel {
    id: ElementId,
    panel_height: f32,
    safe_area_bottom: f32,
    on_select_emoji: SelectEmoji,
    on_select_custom_emoji: SelectSticker,
    on_backspace: PanelAction,
    on_send_sticker: SelectSticker,
    on_send_gif: SendGif,
    on_open_gif_search: PanelAction,
    on_open_pack_settings: OpenPackSettings,
    gif_search_namespace: Option<ElementId>,
    gif_search_active: bool,
    uses_flat_chrome: bool,
    stickers_restricted_text: Option<String>,
    gif_restricted_text: Option<String>,
    on_tab_shown: Option<TabShown>,
    places_on_media: bool,
    surface_fill: Option<Hsla>,
    over_backdrop: bool,
}

impl ChatInputPickerPanel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        panel_height: f32,
        on_select_emoji: impl Fn(String, &mut Window, &mut App) + 'static,
        on_select_custom_emoji: impl Fn(StickerItem, &mut Window, &mut App) + 'static,
        on_backspace: impl Fn(&mut Window, &mut App) + 'static,
        on_send_sticker: impl Fn(StickerItem, &mut Window, &mut App) + 'static,
        on_send_gif: impl Fn(GifItem, &mut Window, &mut App) + 'static,
        on_open_gif_search: impl Fn(&mut Window, &mut App) + 'static,
        on_open_pack_settings: impl Fn(PackKind, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            panel_height,
            safe_area_bottom: 0.0,
            on_select_emoji: Rc::new(on_select_emoji),
            on_select_custom_emoji: Rc::new(on_select_custom_emoji),
            on_backspace: Rc::new(on_backspace),
            on_send_sticker: Rc::new(on_send_sticker),
            on_send_gif: Rc::new(on_send_gif),
            on_open_gif_search: Rc::new(on_open_gif_search),
            on_open_pack_settings: Rc::new(on_open_pack_settings),
            gif_search_namespace: None,
            gif_search_active: false,
            uses_flat_chrome: false,
            stickers_restricted_text: None,
            gif_restricted_text: None,
            on_tab_shown: None,
            places_on_media: false,
            surface_fill: None,
            over_backdrop: false,
        }
    }

    pub const BACKDROP_BLUR_RADIUS: f32 = 24.0;

    pub fn backdrop_tint_peak(dark: bool) -> f32 {
        if dark { 0.74 } else { 0.68 }
    }

    pub fn over_backdrop(mut self, over_backdrop: bool) -> Self {
        self.over_backdrop = over_backdrop;
        self
    }

    pub fn surface_fill(mut self, surface_fill: Option<Hsla>) -> Self {
        self.surface_fill = surface_fill;
        self
    }

    pub fn safe_area_bottom(mut self, safe_area_bottom: f32) -> Self {
        self.safe_area_bottom = safe_area_bottom;
        self
    }

    pub fn gif_search_namespace(mut self, gif_search_namespace: Option<ElementId>) -> Self {
        self.gif_search_namespace = gif_search_namespace;
        self
    }

    pub fn gif_search_active(mut self, gif_search_active: bool) -> Self {
        self.gif_search_active = gif_search_active;
        self
    }

    pub fn uses_flat_chrome(mut self, uses_flat_chrome: bool) -> Self {
        self.uses_flat_chrome = uses_flat_chrome;
        self
    }

    pub fn stickers_restricted_text(mut self, stickers_restricted_text: Option<String>) -> Self {
        self.stickers_restricted_text = stickers_restricted_text;
        self
    }

    pub fn gif_restricted_text(mut self, gif_restricted_text: Option<String>) -> Self {
        self.gif_restricted_text = gif_restricted_text;
        self
    }

    pub fn on_tab_shown(mut self, on_tab_shown: impl Fn(Option<RiseStickerPickerTab>, &mut App) + 'static) -> Self {
        self.on_tab_shown = Some(Rc::new(on_tab_shown));
        self
    }

    pub fn places_on_media(mut self, places_on_media: bool) -> Self {
        self.places_on_media = places_on_media;
        self
    }

    /// panel inside its own cached view
    /// salt carries what the callbacks capture beyond the props (the chat they act on)
    /// so a new chat renders the panel fresh
    pub fn hosted(self, salt: &impl Hash) -> ChatInputPickerPanelHost {
        let revision = page_revision(&(
            salt,
            self.panel_height.to_bits(),
            self.safe_area_bottom.to_bits(),
            &self.gif_search_namespace,
            self.gif_search_active,
            self.uses_flat_chrome,
            &self.stickers_restricted_text,
            &self.gif_restricted_text,
            self.on_tab_shown.is_some(),
            self.places_on_media,
            self.surface_fill.map(|fill| [fill.h, fill.s, fill.l, fill.a].map(f32::to_bits)),
            self.over_backdrop,
        ));
        ChatInputPickerPanelHost { panel: self, revision }
    }

    fn bottom_bar_reserve(&self) -> f32 {
        ChatInputPickerMetrics::SEGMENT_HEIGHT
            + ChatInputPickerMetrics::BOTTOM_BAR_VERTICAL_PADDING * 2.0
            + self.safe_area_bottom
    }

    fn restricted_tab(text: String, flat_chrome: bool, theme: &AppTheme) -> AnyElement {
        let color = if flat_chrome { theme.text.secondary } else { white().opacity(0.6) };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .px(px(32.0))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .swiftui_font(Font::system_weight(15.0, FontWeight::NORMAL))
                    .text_color(color)
                    .text_center()
                    .child(text),
            )
            .into_any_element()
    }

    fn page(&self, name: &'static str, content: Option<PageContent>, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        let page = div().id(child_id(&self.id, name)).size_full();
        match content {
            None => page,
            Some(PageContent::Plain(element)) => page.child(element),
            Some(PageContent::Cached(revision, build)) => {
                let (host, slot) = {
                    let holder = window
                        .use_keyed_state(child_id(&self.id, format!("{name}-host")), cx, |_, cx| {
                            let build = Rc::new(RefCell::new(None));
                            ChatInputPickerPageHolder {
                                view: cx.new(|_| ChatInputPickerPageView { build: build.clone() }),
                                build,
                            }
                        })
                        .read(cx);
                    (holder.view.clone(), holder.build.clone())
                };
                slot.replace(Some(build));
                page.child(
                    env_scope::<ChatInputPickerPageRevisionKey>(revision)
                        .child(AnyView::from(host).cached(StyleRefinement::default().size_full())),
                )
            }
        }
    }

    fn pager(
        &self,
        state: &Entity<ChatInputPickerPanelState>,
        active_tab: Binding<i64>,
        flat_chrome: bool,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnimatedTabsPager {
        let callbacks = state.read(cx).page_callbacks.clone();
        *callbacks.borrow_mut() = Some(PickerPageCallbacks {
            on_select_emoji: self.on_select_emoji.clone(),
            on_select_custom_emoji: self.on_select_custom_emoji.clone(),
            on_backspace: self.on_backspace.clone(),
            on_send_sticker: self.on_send_sticker.clone(),
            on_send_gif: self.on_send_gif.clone(),
            on_open_gif_search: self.on_open_gif_search.clone(),
            on_open_pack_settings: self.on_open_pack_settings.clone(),
        });
        let bar_reserve = self.bottom_bar_reserve();
        let panel = state.read(cx);
        let current_tab = panel.active_tab;
        let width_reader = state.clone();
        let width_writer = state.clone();
        let page_width = Binding::new(
            move |cx: &App| width_reader.read(cx).page_width,
            move |width, _, cx| {
                width_writer.update(cx, |state, _| state.page_width = width);
            },
        );
        let live_state = panel.pager_live_state.clone();
        let suppressed_state = live_state.clone();
        let hold_is_active =
            rise_ui::shared::try_read::<StickerHoldPreviewPresenter>(cx).is_some_and(|presenter| presenter.is_active());

        let gif_page = if self.places_on_media {
            None
        } else if let Some(text) = self.gif_restricted_text.clone() {
            Some(PageContent::Plain(Self::restricted_tab(text, flat_chrome, theme)))
        } else if panel.mounted_tabs.contains(&TAB_GIF) {
            let id = child_id(&self.id, "gif-tab");
            let is_active = current_tab == TAB_GIF && !self.gif_search_active;
            let namespace = self.gif_search_namespace.clone();
            let search_active = self.gif_search_active;
            let revision = page_revision(&(TAB_GIF, &id, is_active, &namespace, search_active, flat_chrome, bar_reserve.to_bits()));
            let callbacks = callbacks.clone();
            Some(PageContent::Cached(
                revision,
                Rc::new(move |_: &mut Window, _: &mut App| {
                    let send = callbacks.clone();
                    let search = callbacks.clone();
                    ChatInputGifTab::new(
                        id.clone(),
                        move |gif, window, cx| {
                            if let Some(on_send_gif) = PickerPageCallbacks::latest(&send).map(|all| all.on_send_gif) {
                                on_send_gif(gif, window, cx);
                            }
                        },
                        move |window, cx| {
                            if let Some(open) = PickerPageCallbacks::latest(&search).map(|all| all.on_open_gif_search) {
                                open(window, cx);
                            }
                        },
                    )
                    .is_active(is_active)
                    .gif_search_namespace(namespace.clone())
                    .gif_search_active(search_active)
                    .uses_flat_chrome(flat_chrome)
                    .into_any_element()
                }),
            ))
        } else {
            None
        };

        let stickers_page = if let Some(text) = self.stickers_restricted_text.clone() {
            Some(PageContent::Plain(Self::restricted_tab(text, flat_chrome, theme)))
        } else if panel.mounted_tabs.contains(&TAB_STICKERS) {
            let id = child_id(&self.id, "stickers-tab");
            let is_active = current_tab == TAB_STICKERS;
            let revision = page_revision(&(TAB_STICKERS, &id, is_active, flat_chrome, bar_reserve.to_bits()));
            let callbacks = callbacks.clone();
            Some(PageContent::Cached(
                revision,
                Rc::new(move |_: &mut Window, _: &mut App| {
                    let send = callbacks.clone();
                    let settings = callbacks.clone();
                    ChatInputStickersTab::new(id.clone(), move |item, window, cx| {
                        if let Some(on_send_sticker) = PickerPageCallbacks::latest(&send).map(|all| all.on_send_sticker) {
                            on_send_sticker(item, window, cx);
                        }
                    })
                    .is_active(is_active)
                    .uses_flat_chrome(flat_chrome)
                    .on_open_pack_settings(move |window, cx| {
                        if let Some(open) = PickerPageCallbacks::latest(&settings).map(|all| all.on_open_pack_settings) {
                            open(PackKind::Sticker, window, cx);
                        }
                    })
                    .into_any_element()
                }),
            ))
        } else {
            None
        };

        let emoji_page = if panel.mounted_tabs.contains(&TAB_EMOJI) {
            let id = child_id(&self.id, "emoji-tab");
            let is_active = current_tab == TAB_EMOJI;
            let revision = page_revision(&(TAB_EMOJI, &id, is_active, flat_chrome, bar_reserve.to_bits()));
            let callbacks = callbacks.clone();
            Some(PageContent::Cached(
                revision,
                Rc::new(move |_: &mut Window, _: &mut App| {
                    let emoji = callbacks.clone();
                    let custom = callbacks.clone();
                    let backspace = callbacks.clone();
                    ChatInputEmojiTab::new(
                        id.clone(),
                        move |value, window, cx| {
                            if let Some(select) = PickerPageCallbacks::latest(&emoji).map(|all| all.on_select_emoji) {
                                select(value, window, cx);
                            }
                        },
                        move |item, window, cx| {
                            if let Some(select) = PickerPageCallbacks::latest(&custom).map(|all| all.on_select_custom_emoji)
                            {
                                select(item, window, cx);
                            }
                        },
                        move |window, cx| {
                            if let Some(erase) = PickerPageCallbacks::latest(&backspace).map(|all| all.on_backspace) {
                                erase(window, cx);
                            }
                        },
                    )
                    .is_active(is_active)
                    .reserves_picker_bottom_bar(true)
                    .uses_flat_chrome(flat_chrome)
                    .defers_loading_to_parent(true)
                    .shows_backspace(false)
                    .into_any_element()
                }),
            ))
        } else {
            None
        };

        let page_count = tabs(self.places_on_media).len() as i64;
        let pager = AnimatedTabsPager::new(
            child_id(&self.id, "pager"),
            page_count,
            active_tab,
            Binding::constant(0.0),
            page_width,
        )
        .live_drag_state(Some(live_state))
        .drag_locked(hold_is_active)
        .on_siblings_tap_suppressed(Some(Rc::new(move |suppressed, _, cx| {
            suppressed_state.update(cx, |live, cx| {
                live.set_tab_taps_suppressed(suppressed, AnimatedTabsMetrics::TAB_TAP_RESTORE_DELAY, cx);
            });
        })));
        let pager = if self.places_on_media { pager } else { pager.child(self.page("page-gif", gif_page, window, cx)) };
        pager
            .child(self.page("page-stickers", stickers_page, window, cx))
            .child(self.page("page-emoji", emoji_page, window, cx))
    }

    #[allow(clippy::too_many_arguments)]
    fn bar_button(
        &self,
        name: &'static str,
        system_image: &'static str,
        font_size: f32,
        label: String,
        outer_edge_is_leading: bool,
        flat_chrome: bool,
        theme: &AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        let outer = 14.0;
        let inner = 10.0;
        let hit_top = ChatInputPickerMetrics::BOTTOM_BAR_VERTICAL_PADDING;
        let hit_bottom = ChatInputPickerMetrics::BOTTOM_BAR_VERTICAL_PADDING + self.safe_area_bottom;
        let hit_leading = if outer_edge_is_leading { outer } else { inner };
        let hit_trailing = if outer_edge_is_leading { inner } else { outer };
        let side = ChatInputPickerMetrics::SEGMENT_HEIGHT;
        let foreground: Hsla = if flat_chrome { theme.text.primary } else { white().opacity(0.65) };
        let mut face = zstack(Alignment::Center).frame_size(side, side).clip_shape(Shape::Circle);
        face = face.bg(LiquidChrome::surface_fill(theme, 1));
        face = face.children(IconUi::sized(system_image, px(font_size), foreground, false));
        face = if flat_chrome {
            face.child(
                centred_stroke(
                    theme.text.primary.opacity(ChatInputPickerMetrics::FLAT_BAR_BUTTON_STROKE_ALPHA),
                    ChatInputPickerMetrics::FLAT_BAR_BUTTON_STROKE_WIDTH,
                    Shape::Circle,
                )
                .absolute()
                .inset_0(),
            )
        } else {
            face.child(centred_stroke(white().opacity(0.12), 0.6, Shape::Circle).absolute().inset_0())
        };
        div()
            .id(child_id(&self.id, format!("{name}-hover")))
            .occlude()
            .capture_any_mouse_down(|_, window, _| window.prevent_default())
            .tooltip_label(label)
            .mt(px(-hit_top))
            .mb(px(-hit_bottom))
            .ml(px(-hit_leading))
            .mr(px(-hit_trailing))
            .child(
                plain(child_id(&self.id, name))
                    .on_press(action)
                    .pt(px(hit_top))
                    .pb(px(hit_bottom))
                    .pl(px(hit_leading))
                    .pr(px(hit_trailing))
                    .child(face),
            )
    }

    fn bottom_bar(
        &self,
        state: &Entity<ChatInputPickerPanelState>,
        active_tab: Binding<i64>,
        flat_chrome: bool,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> Div {
        let panel = state.read(cx);
        let current_tab = panel.active_tab;
        let live_state = panel.pager_live_state.clone();
        let tap_state = state.clone();
        let segmented = ChatInputPickerSegmentedBar {
            id: child_id(&self.id, "segments"),
            tabs: tabs(self.places_on_media),
            active_tab,
            live_drag_state: live_state,
            on_tap: Rc::new(move |id, _, cx| Self::select_tab(&tap_state, id, cx)),
            uses_flat_chrome: flat_chrome,
        };

        let now = RiseClock::monotonic();
        let on_open_pack_settings = self.on_open_pack_settings.clone();
        let gear = panel.gear_presence.render_transitioned(now, window, |_| {
            self.bar_button(
                "gear",
                "gearshape",
                15.0,
                tr("settings_page_title"),
                true,
                flat_chrome,
                theme,
                move |window, cx| {
                    let _ = selection_changed();
                    let kind = if current_tab == TAB_EMOJI { PackKind::Emoji } else { PackKind::Sticker };
                    on_open_pack_settings(kind, window, cx);
                },
            )
        });
        let on_backspace = self.on_backspace.clone();
        let backspace = panel.backspace_presence.render_transitioned(now, window, |_| {
            self.bar_button(
                "backspace",
                "delete.left",
                16.0,
                tr("delete_action"),
                false,
                flat_chrome,
                theme,
                move |window, cx| {
                    let _ = impact(ImpactStyle::Light);
                    on_backspace(window, cx);
                },
            )
        });

        let scrim_color = match self.surface_fill {
            Some(fill) => fill,
            None if flat_chrome || self.over_backdrop => theme.bg._100,
            None => black(),
        };
        let shadow_height = BAR_SCRIM_LEAD + self.bottom_bar_reserve();
        let shadow_solid_end =
            (self.safe_area_bottom + ChatInputPickerMetrics::BOTTOM_BAR_VERTICAL_PADDING) / shadow_height;
        let scrim = pass_through().absolute().left_0().right_0().bottom_0().top(px(-BAR_SCRIM_LEAD)).child(
            LinearGradient::new(
                ScrollEdgeBlurFalloff::mask_image(scrim_color.opacity(BAR_SHADOW_PEAK), shadow_solid_end),
                UnitPoint::BOTTOM,
                UnitPoint::TOP,
            ),
        );

        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .px(px(14.0))
            .pt(px(ChatInputPickerMetrics::BOTTOM_BAR_VERTICAL_PADDING))
            .pb(px(ChatInputPickerMetrics::BOTTOM_BAR_VERTICAL_PADDING + self.safe_area_bottom))
            .capture_any_mouse_down(|_, window, _| window.prevent_default())
            .child(scrim)
            .child(
                zstack(Alignment::Center)
                    .w_full()
                    .h(px(ChatInputPickerMetrics::SEGMENT_HEIGHT))
                    .child(segmented)
                    .child(hstack(0.0).w_full().h_full().children(gear).child(spacer()).children(backspace)),
            )
    }

    fn select_tab(state: &Entity<ChatInputPickerPanelState>, id: i64, cx: &mut App) {
        if state.read(cx).active_tab == id {
            return;
        }
        state.update(cx, |state, _| {
            state.mounted_tabs.insert(id);
        });
        let _ = selection_changed();
        ChatInputPickerPanelState::set_active_tab(state, id, cx);
    }
}

impl RenderOnce for ChatInputPickerPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx| {
            ChatInputPickerPanelState::new(self.places_on_media, cx)
        });
        let on_tab_shown = self.on_tab_shown.clone();
        state.update(cx, |state, _| state.on_tab_shown = on_tab_shown);
        let (theme, resolved_appearance) = {
            let store = ThemeStore::shared(cx).read(cx);
            (store.current_theme().clone(), store.resolved_appearance())
        };
        let flat_chrome =
            self.uses_flat_chrome || self.surface_fill.is_some() || resolved_appearance == RiseThemeAppearance::Light;

        let active_reader = state.clone();
        let active_writer = state.clone();
        let active_tab = Binding::new(
            move |cx: &App| active_reader.read(cx).active_tab,
            move |value: i64, _, cx| ChatInputPickerPanelState::set_active_tab(&active_writer, value, cx),
        );
        let first_tab = first_tab_id(self.places_on_media);
        let page_reader = state.clone();
        let page_writer = state.clone();
        let pager_page = Binding::new(
            move |cx: &App| page_reader.read(cx).active_tab - first_tab,
            move |page: i64, _, cx| ChatInputPickerPanelState::set_active_tab(&page_writer, page + first_tab, cx),
        );
        let frame = state.read(cx).pop_guard.frame.clone();

        let mut panel = div()
            .relative()
            .w_full()
            .h(px(self.panel_height))
            .rounded_t(px(PANEL_CORNER_RADIUS))
            .overflow_hidden()
            .on_any_mouse_down(|_, window, _| window.prevent_default());
        if self.over_backdrop {
        } else if flat_chrome {
            panel = panel.bg(self.surface_fill.unwrap_or(theme.bg._100));
        } else {
            panel = panel.child(
                div().absolute().top_0().left_0().right_0().bottom(px(-PANEL_CORNER_RADIUS)).child(
                    HoldContextMenuGlass::new(child_id(&self.id, "glass"))
                        .tint(black().opacity(0.30))
                        .corner_radius(PANEL_CORNER_RADIUS)
                        .opaque(true),
                ),
            );
        }
        panel = panel
            .child(AnimatedMediaSettleProbe::new(child_id(&self.id, "settle-probe")))
            .child(canvas(move |bounds, _, _| frame.set(bounds), |_, _, _, _| {}).absolute().size_full())
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .capture_any_mouse_down(|_, window, _| window.prevent_default())
                    .child(
                        env_scope::<ChatInputPickerBarReserveKey>(self.bottom_bar_reserve())
                            .child(self.pager(&state, pager_page, flat_chrome, &theme, window, cx)),
                    ),
            )
            .child(self.bottom_bar(&state, active_tab, flat_chrome, &theme, window, cx));
        if !flat_chrome {
            panel = panel.child(
                centred_stroke(white().opacity(0.16), 0.8, Shape::continuous(PANEL_CORNER_RADIUS))
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom(px(-PANEL_CORNER_RADIUS)),
            );
        }

        let appear_state = state.clone();
        let disappear_state = state;
        panel
            .on_appear(child_id(&self.id, "appear"), move |_, cx| {
                ChatInputPickerPanelState::did_appear(&appear_state, cx);
            })
            .on_disappear(child_id(&self.id, "disappear"), move |_, cx| {
                ChatInputPickerPanelState::did_disappear(&disappear_state, cx);
            })
    }
}

/// panel callbacks as of the last render
/// a cached page replays old listeners so they read these at call time instead of capturing
#[derive(Clone)]
struct PickerPageCallbacks {
    on_select_emoji: SelectEmoji,
    on_select_custom_emoji: SelectSticker,
    on_backspace: PanelAction,
    on_send_sticker: SelectSticker,
    on_send_gif: SendGif,
    on_open_gif_search: PanelAction,
    on_open_pack_settings: OpenPackSettings,
}

impl PickerPageCallbacks {
    fn latest(cell: &Rc<RefCell<Option<Self>>>) -> Option<Self> {
        cell.borrow().clone()
    }
}

type PageBuild = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// one pager page, a plain element or a grid built inside its own cached view
enum PageContent {
    Plain(AnyElement),
    Cached(u64, PageBuild),
}

fn page_revision(props: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    props.hash(&mut hasher);
    hasher.finish()
}

struct ChatInputPickerPageHolder {
    view: Entity<ChatInputPickerPageView>,
    build: Rc<RefCell<Option<PageBuild>>>,
}

/// gif, sticker or emoji grid as its own cached view
/// inline it rebuilt and laid out everything on every frame of the tab switch
struct ChatInputPickerPageView {
    build: Rc<RefCell<Option<PageBuild>>>,
}

impl Render for ChatInputPickerPageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let build = self.build.borrow().clone();
        div().size_full().children(build.map(|build| build(window, cx)))
    }
}

#[derive(IntoElement)]
pub struct ChatInputPickerPanelHost {
    panel: ChatInputPickerPanel,
    revision: u64,
}

struct ChatInputPickerPanelHostHolder {
    view: Entity<ChatInputPickerPanelHostView>,
    panel: Rc<RefCell<Option<ChatInputPickerPanel>>>,
}

struct ChatInputPickerPanelHostView {
    panel: Rc<RefCell<Option<ChatInputPickerPanel>>>,
}

impl Render for ChatInputPickerPanelHostView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(self.panel.borrow().clone())
    }
}

impl RenderOnce for ChatInputPickerPanelHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (view, slot) = {
            let holder = window
                .use_keyed_state(child_id(&self.panel.id, "panel-host"), cx, |_, cx| {
                    let panel = Rc::new(RefCell::new(None));
                    ChatInputPickerPanelHostHolder {
                        view: cx.new(|_| ChatInputPickerPanelHostView { panel: panel.clone() }),
                        panel,
                    }
                })
                .read(cx);
            (holder.view.clone(), holder.panel.clone())
        };
        slot.replace(Some(self.panel));
        div().size_full().child(
            env_scope::<ChatInputPickerPanelHostRevisionKey>(self.revision)
                .child(AnyView::from(view).cached(StyleRefinement::default().size_full())),
        )
    }
}

struct ChatInputPickerPanelHostRevisionKey;

impl EnvKey for ChatInputPickerPanelHostRevisionKey {
    type Value = u64;

    fn default_value() -> Self::Value {
        0
    }

    fn fingerprint(value: &Self::Value) -> Option<u64> {
        Some(*value)
    }
}

struct ChatInputPickerPageRevisionKey;

impl EnvKey for ChatInputPickerPageRevisionKey {
    type Value = u64;

    fn default_value() -> Self::Value {
        0
    }

    fn fingerprint(value: &Self::Value) -> Option<u64> {
        Some(*value)
    }
}

#[derive(IntoElement)]
struct ChatInputPickerSegmentedBar {
    id: ElementId,
    tabs: Vec<TabHeaderConfig>,
    active_tab: Binding<i64>,
    live_drag_state: Entity<AnimatedTabsPagerLiveState>,
    on_tap: Rc<dyn Fn(i64, &mut Window, &mut App)>,
    uses_flat_chrome: bool,
}

impl RenderOnce for ChatInputPickerSegmentedBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let progress_state = self.live_drag_state.clone();
        let suppress_state = self.live_drag_state;
        let on_tap = self.on_tap;
        ChatInputSegmentedTabs::new(
            self.id,
            self.tabs,
            self.active_tab,
            move |cx: &App| progress_state.read(cx).drag_progress(),
            move |id, window, cx| on_tap(id, window, cx),
        )
        .suppresses_tap(move |cx: &App| suppress_state.read(cx).suppresses_tab_taps())
        .uses_flat_chrome(self.uses_flat_chrome)
    }
}
