use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, ElementId, FontWeight, Hsla, Rgba, ScrollHandle, SharedString, Window, point, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::{IconUi, MainText};

use crate::modules::chat::components::chat::topic_icon_glyph::TopicIconGlyph;
use crate::modules::chat::pages::chats::chat::chat_screen::chat_screen_support::ChatTopicTab;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatTopic;

pub type ChatTopicBarOnSelect = Rc<dyn Fn(ChatTopicTab, &mut Window, &mut App)>;
pub type ChatTopicBarOnCreate = Rc<dyn Fn(&mut Window, &mut App)>;

const SCROLL_TO_SELECTED_DURATION: f64 = 0.2;

fn color(red: f32, green: f32, blue: f32) -> Hsla {
    Rgba { r: red, g: green, b: blue, a: 1.0 }.into()
}

fn accent() -> Hsla {
    color(0.22, 0.60, 0.96)
}

fn badge_colors() -> [Hsla; 6] {
    [
        color(0.43, 0.73, 0.94),
        color(0.98, 0.67, 0.30),
        color(0.80, 0.53, 0.86),
        color(0.56, 0.83, 0.49),
        color(1.00, 0.58, 0.70),
        color(0.96, 0.44, 0.39),
    ]
}

struct ChatTopicBarScroll {
    handle: ScrollHandle,
    shown: Option<String>,
    pending_index: Option<usize>,
    offset: Animated<f32>,
}

#[derive(IntoElement)]
pub struct ChatTopicBar {
    id: ElementId,
    pub topics: Vec<ChatTopic>,
    pub selected: ChatTopicTab,
    pub shows_all: bool,
    pub can_create: bool,
    pub on_select: ChatTopicBarOnSelect,
    pub on_create: ChatTopicBarOnCreate,
}

impl ChatTopicBar {
    pub fn new(
        id: impl Into<ElementId>,
        topics: Vec<ChatTopic>,
        selected: ChatTopicTab,
        can_create: bool,
        on_select: impl Fn(ChatTopicTab, &mut Window, &mut App) + 'static,
        on_create: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            topics,
            selected,
            shows_all: true,
            can_create,
            on_select: Rc::new(on_select),
            on_create: Rc::new(on_create),
        }
    }

    pub fn shows_all(mut self, shows_all: bool) -> Self {
        self.shows_all = shows_all;
        self
    }

    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn tab_id(tab: &ChatTopicTab) -> String {
        match tab {
            ChatTopicTab::All => "__all__".to_owned(),
            ChatTopicTab::General => "__general__".to_owned(),
            ChatTopicTab::Topic(id) => id.clone(),
        }
    }

    fn badge_color(seed: &str) -> Hsla {
        let mut h: i64 = 0;
        for b in seed.bytes() {
            h = h.wrapping_mul(31).wrapping_add(i64::from(b)) & 0x7fff_ffff;
        }
        let colors = badge_colors();
        colors[h as usize % colors.len()]
    }

    /// row position of a tab for scrolling, the row direct children are its scroll items
    fn tab_index(&self, tab: &ChatTopicTab) -> Option<usize> {
        let all_slots = usize::from(self.shows_all);
        match tab {
            ChatTopicTab::All => self.shows_all.then_some(0),
            ChatTopicTab::General => Some(all_slots),
            ChatTopicTab::Topic(id) => self
                .topics
                .iter()
                .position(|topic| topic.id == *id)
                .map(|position| all_slots + 1 + position),
        }
    }

    fn centered_offset(handle: &ScrollHandle, index: usize) -> Option<f32> {
        let item = handle.bounds_for_item(index)?;
        let viewport = handle.bounds();
        if viewport.size.width <= px(0.0) {
            return None;
        }
        let max = f32::from(handle.max_offset().x).max(0.0);
        Some(f32::from(viewport.center().x - item.center().x).clamp(-max, 0.0))
    }

    fn scroll_handle(&self, window: &mut Window, cx: &mut App) -> ScrollHandle {
        let state = window.use_keyed_state(self.child_id("scroll"), cx, |_, _| ChatTopicBarScroll {
            handle: ScrollHandle::new(),
            shown: None,
            pending_index: None,
            offset: Animated::new(0.0),
        });
        let selected_id = Self::tab_id(&self.selected);
        let selected_index = self.tab_index(&self.selected);
        let now = RiseClock::monotonic();
        state.update(cx, |scroll, cx| {
            if scroll.shown.as_deref() != Some(selected_id.as_str()) {
                if scroll.shown.is_some() {
                    scroll.pending_index = selected_index;
                }
                scroll.shown = Some(selected_id);
            }
            if let Some(index) = scroll.pending_index {
                if let Some(target) = Self::centered_offset(&scroll.handle, index) {
                    scroll.pending_index = None;
                    scroll.offset.jump(f32::from(scroll.handle.offset().x));
                    scroll.offset.set_in(target, ease_out(SCROLL_TO_SELECTED_DURATION), now, cx);
                } else {
                    window.request_animation_frame();
                }
            }
            if scroll.offset.is_running(now) {
                let x = scroll.offset.frame_value(now, window);
                scroll.handle.set_offset(point(px(x), scroll.handle.offset().y));
            }
            scroll.handle.clone()
        })
    }

    fn chip(
        &self,
        tab: ChatTopicTab,
        title: String,
        topic: Option<&ChatTopic>,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let is_selected = self.selected == tab;
        let label_color = if is_selected { accent() } else { theme.text.primary };
        let on_select = self.on_select.clone();
        let is_closed = topic.is_some_and(|topic| topic.is_closed);
        let icon = Self::icon(&tab, topic, theme);
        let chip_id = self.child_id(format!("chip-{}", Self::tab_id(&tab)));
        let label = if title.is_empty() { tr("chat_topics_topic").to_string() } else { title };
        plain(chip_id).flex_none().on_press(move |window, cx| on_select(tab.clone(), window, cx)).child(
            div()
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap(px(6.0))
                .px(px(12.0))
                .h(px(42.0))
                .child(icon)
                .child(
                    div()
                        .relative()
                        .child(
                            MainText::new(label)
                                .px(15.0)
                                .font_weight(if is_selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .color(label_color)
                                .number_of_lines(1),
                        )
                        .when(is_selected, |label| {
                            label.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom(px(-6.0))
                                    .h(px(2.5))
                                    .rounded_full()
                                    .bg(accent()),
                            )
                        }),
                )
                .when(is_closed, |row| {
                    row.children(IconUi::sized(
                        "lock.fill",
                        px(10.0),
                        theme.text.secondary.opacity(0.7),
                        true,
                    ))
                }),
        )
    }

    fn icon(tab: &ChatTopicTab, topic: Option<&ChatTopic>, theme: &AppTheme) -> AnyElement {
        match tab {
            ChatTopicTab::All => div()
                .children(IconUi::sized("tray", px(16.0), theme.text.primary, false))
                .into_any_element(),
            ChatTopicTab::General => MainText::new("#")
                .px(17.0)
                .font_weight(FontWeight::BOLD)
                .color(theme.text.secondary)
                .into_any_element(),
            ChatTopicTab::Topic(_) => Self::topic_badge(topic),
        }
    }

    fn topic_badge(topic: Option<&ChatTopic>) -> AnyElement {
        let icon = topic.map_or("", |topic| topic.icon.as_str());
        if !icon.is_empty() {
            return TopicIconGlyph::new(icon.to_owned()).size(16.0).into_any_element();
        }
        let title = topic.map_or("", |topic| topic.title.as_str());
        let letter = title
            .chars()
            .next()
            .map_or_else(|| "#".to_owned(), |first| first.to_uppercase().collect::<String>());
        let seed = topic.map_or(title, |topic| topic.id.as_str());
        div()
            .size(px(22.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(Self::badge_color(seed))
            .child(MainText::new(letter).px(11.0).font_weight(FontWeight::BOLD).color(white()))
            .into_any_element()
    }

    fn create_chip(&self, theme: &AppTheme) -> impl IntoElement + use<> {
        let on_create = self.on_create.clone();
        plain(self.child_id("create")).flex_none().on_press(move |window, cx| on_create(window, cx)).child(
            div().flex_none().pl(px(4.0)).pr(px(6.0)).child(
                div()
                    .size(px(30.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.bg._300.opacity(0.7))
                    .children(IconUi::sized("plus", px(14.0), theme.text.secondary, false)),
            ),
        )
    }
}

impl RenderOnce for ChatTopicBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let handle = self.scroll_handle(window, cx);
        let mut row = div()
            .id(self.child_id("row"))
            .size_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .px(px(6.0))
            .overflow_x_scroll()
            .track_scroll(&handle);
        if self.shows_all {
            row = row.child(self.chip(
                ChatTopicTab::All,
                tr("chat_topics_all").to_string(),
                None,
                &theme,
            ));
        }
        row = row.child(self.chip(
            ChatTopicTab::General,
            tr("chat_topics_general").to_string(),
            None,
            &theme,
        ));
        for topic in &self.topics {
            row = row.child(self.chip(
                ChatTopicTab::Topic(topic.id.clone()),
                topic.title.clone(),
                Some(topic),
                &theme,
            ));
        }
        if self.can_create {
            row = row.child(self.create_chip(&theme));
        }
        div().w_full().px(px(8.0)).pt(px(6.0)).pb(px(2.0)).child(
            div()
                .w_full()
                .h(px(42.0))
                .child(row)
                .liquid_chrome_rounded_rectangle_with(21.0, 0.12, None, false),
        )
    }
}
