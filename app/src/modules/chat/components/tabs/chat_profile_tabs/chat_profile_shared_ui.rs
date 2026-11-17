use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, ScrollHandle, Styled, Task, Window,
};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::hold_context_menu_locks_scroll;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::environment::{EnvKey, env};
use rise_ui::swiftui::frame::{EdgeSet, SwiftUiAlignedFrame, SwiftUiFrame};
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::stacks::{Alignment, hstack, spacer, vstack};

use crate::core::lib::canon_format::{CalendarBand, RiseFormat};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui_hooks::ChatProfileSharedUiHooks;

/// key that says the tab sits inside the profile page scroll
pub struct ChatProfileUsesParentScrollKey;

impl EnvKey for ChatProfileUsesParentScrollKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// true when the tab is inside the profile page own scroll
pub fn chat_profile_uses_parent_scroll(cx: &App) -> bool {
    env::<ChatProfileUsesParentScrollKey>(cx)
}

/// the profile page scroll a tab sits in, so pinned rows find its real top
pub struct ChatProfilePageScrollKey;

impl EnvKey for ChatProfilePageScrollKey {
    type Value = Option<ScrollHandle>;

    fn default_value() -> Self::Value {
        None
    }
}

pub fn chat_profile_page_scroll(cx: &App) -> Option<ScrollHandle> {
    env::<ChatProfilePageScrollKey>(cx)
}

/// window y of the pinned tab bar bottom edge
/// tabs container writes it before its pages prepaint so headers sit right under the bar
pub type ChatProfilePinnedTabsBottom = Rc<Cell<Option<f32>>>;

pub struct ChatProfilePinnedTabsBottomKey;

impl EnvKey for ChatProfilePinnedTabsBottomKey {
    type Value = Option<ChatProfilePinnedTabsBottom>;

    fn default_value() -> Self::Value {
        None
    }
}

pub fn chat_profile_pinned_tabs_bottom(cx: &App) -> Option<ChatProfilePinnedTabsBottom> {
    env::<ChatProfilePinnedTabsBottomKey>(cx)
}

type ChatProfileTabContent = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;

pub(crate) type ChatProfileTabRefresh = Rc<dyn Fn(&mut Window, &mut App) -> Task<()>>;

/// retry callback for a tab, runs on main thread
pub(crate) type ChatProfileTabRetry = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ChatProfileTabScrollContainer {
    id: ElementId,
    scroll_enabled: bool,
    shows_indicators: bool,
    on_refresh: Option<ChatProfileTabRefresh>,
    content: ChatProfileTabContent,
}

impl ChatProfileTabScrollContainer {
    /// id keys the scroll position
    pub fn new<E: IntoElement>(
        id: impl Into<ElementId>,
        content: impl FnOnce(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            scroll_enabled: true,
            shows_indicators: true,
            on_refresh: None,
            content: Box::new(move |window, cx| content(window, cx).into_any_element()),
        }
    }

    pub fn scroll_enabled(mut self, scroll_enabled: bool) -> Self {
        self.scroll_enabled = scroll_enabled;
        self
    }

    pub fn shows_indicators(mut self, shows_indicators: bool) -> Self {
        self.shows_indicators = shows_indicators;
        self
    }

    pub fn on_refresh(mut self, on_refresh: impl Fn(&mut Window, &mut App) -> Task<()> + 'static) -> Self {
        self.on_refresh = Some(Rc::new(on_refresh));
        self
    }
}

impl RenderOnce for ChatProfileTabScrollContainer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let uses_parent_scroll = chat_profile_uses_parent_scroll(cx);
        let hold_scroll_lock = hold_context_menu_locks_scroll(cx);
        let content = (self.content)(window, cx);
        if uses_parent_scroll {
            return vstack(0.0).w_full().child(content).frame_max_width(Alignment::Top).into_any_element();
        }
        let scroll = scroll_view(self.id)
            .shows_indicators(self.shows_indicators)
            .scroll_disabled(!self.scroll_enabled || hold_scroll_lock)
            .size_full()
            .child(content);
        match self.on_refresh {
            Some(on_refresh) => scroll
                .refreshable(move |window, cx| on_refresh(window, cx))
                .into_any_element(),
            None => scroll.into_any_element(),
        }
    }
}

#[derive(IntoElement, Default)]
pub struct ChatProfileTabBottomSpacer;

impl ChatProfileTabBottomSpacer {
    pub(crate) const HEIGHT: f32 = 80.0;
}

impl RenderOnce for ChatProfileTabBottomSpacer {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let height = if chat_profile_uses_parent_scroll(cx) { 0.0 } else { Self::HEIGHT };
        gpui::div().w_full().frame_size(None, height)
    }
}

type ChatProfileTabStateBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// data render over one message tab slice of the chat
#[derive(IntoElement)]
pub struct ChatProfileTabDataRender {
    cache_id: String,
    scope: String,
    is_empty: bool,
    empty_icon: String,
    empty_text: String,
    message_height_percent: f32,
    no_data_height_percent: f32,
    retry: ChatProfileTabRetry,
    content: ChatProfileTabContent,
    loading: ChatProfileTabStateBuilder,
}

impl ChatProfileTabDataRender {
    pub(crate) const DEFAULT_MESSAGE_HEIGHT_PERCENT: f32 = 30.0;
    pub(crate) const DEFAULT_NO_DATA_HEIGHT_PERCENT: f32 = 20.0;

    #[allow(clippy::too_many_arguments)]
    pub fn new<C: IntoElement, L: IntoElement>(
        cache_id: impl Into<String>,
        scope: impl Into<String>,
        is_empty: bool,
        empty_icon: impl Into<String>,
        empty_text: impl Into<String>,
        retry: impl Fn(&mut Window, &mut App) + 'static,
        content: impl FnOnce(&mut Window, &mut App) -> C + 'static,
        loading: impl Fn(&mut Window, &mut App) -> L + 'static,
    ) -> Self {
        Self {
            cache_id: cache_id.into(),
            scope: scope.into(),
            is_empty,
            empty_icon: empty_icon.into(),
            empty_text: empty_text.into(),
            message_height_percent: Self::DEFAULT_MESSAGE_HEIGHT_PERCENT,
            no_data_height_percent: Self::DEFAULT_NO_DATA_HEIGHT_PERCENT,
            retry: Rc::new(retry),
            content: Box::new(move |window, cx| content(window, cx).into_any_element()),
            loading: Rc::new(move |window, cx| loading(window, cx).into_any_element()),
        }
    }

    pub(crate) fn message_height_percent(mut self, message_height_percent: f32) -> Self {
        self.message_height_percent = message_height_percent;
        self
    }

    pub(crate) fn no_data_height_percent(mut self, no_data_height_percent: f32) -> Self {
        self.no_data_height_percent = no_data_height_percent;
        self
    }
}

impl RenderOnce for ChatProfileTabDataRender {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let status = ChatProfileSharedUiHooks::message_tab_render_status_or_default(
            &self.cache_id,
            &self.scope,
            !self.is_empty,
            cx,
        );
        let error_text = ChatProfileSharedUiHooks::message_tab_error_or_default(&self.cache_id, &self.scope, cx);
        let render_id = ElementId::Name(format!("chat-profile-tab-data-{}-{}", self.cache_id, self.scope).into());
        let content = self.content;
        let loading = self.loading;
        let empty_icon = self.empty_icon;
        let empty_text = self.empty_text.clone();
        let error_state_text = error_text.clone().unwrap_or_else(|| tr("error_fetching_data"));
        let retry = self.retry;
        SaiDataRender::new(status, content)
            .id(render_id)
            .is_empty(self.is_empty)
            .no_data_text(self.empty_text)
            .error_text(error_text)
            .message_height_percent(self.message_height_percent)
            .no_data_height_percent(self.no_data_height_percent)
            .sai_data_render_loading(move |window, cx| loading(window, cx))
            .sai_data_render_empty(move |_, _| ChatProfileEmptyState::new(empty_icon.clone(), empty_text.clone()))
            .sai_data_render_error(move |_, _| {
                let retry = retry.clone();
                ChatProfileTabErrorState::new(error_state_text.clone(), move |window, cx| retry(window, cx))
            })
    }
}

#[derive(IntoElement)]
pub struct ChatProfileTabErrorState {
    text: String,
    retry: ChatProfileTabRetry,
}

impl ChatProfileTabErrorState {
    pub fn new(text: impl Into<String>, retry: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { text: text.into(), retry: Rc::new(retry) }
    }
}

impl RenderOnce for ChatProfileTabErrorState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let text_secondary = theme.text.secondary;
        let primary = theme.primary._100;
        let on_primary = theme.content_on(primary);
        let retry = self.retry;
        vstack(14.0)
            .child(
                MainText::new(self.text)
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(text_secondary),
            )
            .child(
                plain("chat-profile-tab-retry")
                    .on_press(move |window, cx| retry(window, cx))
                    .child(
                        gpui::div()
                            .padding_edges(EdgeSet::HORIZONTAL, 18.0)
                            .padding_edges(EdgeSet::VERTICAL, 9.0)
                            .bg(primary)
                            .rounded_full()
                            .child(
                                MainText::new(tr("common_retry"))
                                    .px(14.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(on_primary),
                            ),
                    ),
            )
            .padding_edges(EdgeSet::VERTICAL, 60.0)
            .padding_edges(EdgeSet::HORIZONTAL, 24.0)
            .frame_max_width(Alignment::Center)
    }
}

#[derive(IntoElement)]
pub struct ChatProfileSectionHeader {
    title: String,
}

impl ChatProfileSectionHeader {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into() }
    }
}

impl RenderOnce for ChatProfileSectionHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let text_secondary = theme.text.secondary;
        let background = theme.bg._100;
        hstack(None)
            .w_full()
            .child(
                MainText::new(self.title)
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(text_secondary),
            )
            .child(spacer())
            .padding_edges(EdgeSet::HORIZONTAL, 16.0)
            .padding_edges(EdgeSet::TOP, 14.0)
            .padding_edges(EdgeSet::BOTTOM, 6.0)
            .bg(background)
    }
}

#[derive(IntoElement)]
pub struct ChatProfileEmptyState {
    icon: String,
    text: String,
    vertical_padding: f32,
}

impl ChatProfileEmptyState {
    const ICON_SIZE: f32 = 38.0;
    const ICON_OPACITY: f32 = 0.5;
    const TEXT_MAX_WIDTH: f32 = 260.0;
    const VERTICAL_PADDING: f32 = 60.0;

    pub fn new(icon: impl Into<String>, text: impl Into<String>) -> Self {
        Self { icon: icon.into(), text: text.into(), vertical_padding: Self::VERTICAL_PADDING }
    }

    /// a page under a tall pinned header has little room above the mini player
    pub fn vertical_padding(mut self, padding: f32) -> Self {
        self.vertical_padding = padding;
        self
    }
}

impl RenderOnce for ChatProfileEmptyState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let text_secondary = theme.text.secondary;
        vstack(10.0)
            .children(IconUi::sized(
                &self.icon,
                gpui::px(Self::ICON_SIZE),
                text_secondary.opacity(Self::ICON_OPACITY),
                false,
            ))
            .child(
                MainText::new(self.text)
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(text_secondary)
                    .max_w(gpui::px(Self::TEXT_MAX_WIDTH)),
            )
            .padding_edges(EdgeSet::VERTICAL, self.vertical_padding)
            .frame_max_width(Alignment::Center)
    }
}

pub enum ChatProfileFormatter {}

impl ChatProfileFormatter {
    pub fn bytes(b: i64) -> String {
        RiseFormat::file_size(b)
    }

    pub fn duration(d: TimeInterval) -> String {
        RiseFormat::duration_time_interval(d)
    }

    pub fn relative_date(ts: TimeInterval) -> String {
        let date = Date::from_time_interval_since_1970(ts);
        let now = Date::now();
        let calendar = Calendar::current();
        if RiseFormat::calendar_band(&date, &now, &calendar) != CalendarBand::Today {
            return RiseFormat::day_label(&date, &now, &calendar, &RiseFormat::app_locale(), false);
        }
        RiseFormat::clock_date(&date, &calendar, RiseFormat::uses_twelve_hour_clock())
    }
}
