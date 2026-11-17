use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Window, relative};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButtonModifiers, LiquidPressShape};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::ButtonAction;
use rise_ui::swiftui::motion::{ease_in_out, linear};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::{IconUi, MainText};

use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::modules::chat::components::chat::chat_group_voice_banner::ChatHeaderAccessoryMetrics;
use crate::modules::chat::shared::utils::message_preview::MessagePreview;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

pub enum ChatPinnedCyclePolicy {}

impl ChatPinnedCyclePolicy {
    pub fn next_index(after_current_index: i64, count: i64) -> i64 {
        if count <= 1 {
            return 0;
        }
        let normalized_index = after_current_index.max(0).min(count - 1);
        if normalized_index == 0 { count - 1 } else { normalized_index - 1 }
    }
}

const CORNER_RADIUS: f32 = 16.0;
const SWAP_DURATION: f64 = 0.2;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

fn stable_id(message: &GetMessageMessage) -> String {
    let server_id = message.sai_server_id();
    let trimmed = trimming_whitespaces_and_newlines(&server_id);
    if trimmed.is_empty() { message.id.clone() } else { trimmed.to_owned() }
}

fn active_index(pinned_messages: &[GetMessageMessage], active_message_id: Option<&str>) -> usize {
    if pinned_messages.is_empty() {
        return 0;
    }
    active_message_id
        .and_then(|active| pinned_messages.iter().position(|message| stable_id(message) == active))
        .unwrap_or(pinned_messages.len() - 1)
}

struct ChatPinnedBarState {
    active_message_id: Option<String>,
    swap: Animated<f32>,
    outgoing_snippet: Option<String>,
    displayed: Option<(String, String)>,
}

impl ChatPinnedBarState {
    fn new() -> Self {
        Self {
            active_message_id: None,
            swap: Animated::new(1.0),
            outgoing_snippet: None,
            displayed: None,
        }
    }

    fn synchronize_active_message(&mut self, previous_ids: &[String], current_ids: &[String]) {
        let Some(newest_id) = current_ids.last() else {
            self.active_message_id = None;
            return;
        };

        let Some(active_message_id) = self.active_message_id.as_ref() else {
            self.active_message_id = Some(newest_id.clone());
            return;
        };
        if !current_ids.contains(active_message_id) {
            self.active_message_id = Some(newest_id.clone());
            return;
        }

        if let Some(previous_newest_id) = previous_ids.last()
            && active_message_id == previous_newest_id
            && newest_id != previous_newest_id
        {
            self.active_message_id = Some(newest_id.clone());
        }
    }

    fn begin_swap(&mut self, outgoing_snippet: String, cx: &mut Context<Self>) {
        self.outgoing_snippet = Some(outgoing_snippet);
        self.swap.jump(0.0);
        self.swap.set_in(1.0, ease_in_out(SWAP_DURATION), RiseClock::monotonic(), cx);
    }

    fn note_displayed(&mut self, message_id: &str, snippet: &str, cx: &mut Context<Self>) {
        if let Some((id, previous)) = &self.displayed {
            if id == message_id && previous == snippet {
                return;
            }
            if id != message_id {
                let outgoing = previous.clone();
                self.begin_swap(outgoing, cx);
            }
        }
        self.displayed = Some((message_id.to_owned(), snippet.to_owned()));
    }
}

struct ChatPinnedBarDisplayed {
    index: usize,
    snippet: String,
    thumb_source: Option<String>,
    swap: f32,
    outgoing_snippet: Option<String>,
}

#[derive(IntoElement)]
pub struct ChatPinnedBar {
    id: ElementId,
    pinned_messages: Rc<Vec<GetMessageMessage>>,
    theme: AppTheme,
    embedded: bool,
    on_open_message: Rc<dyn Fn(&GetMessageMessage, &mut Window, &mut App)>,
    on_open_list: ButtonAction,
}

impl ChatPinnedBar {
    pub fn new(
        id: impl Into<ElementId>,
        pinned_messages: Vec<GetMessageMessage>,
        theme: AppTheme,
        on_open_message: impl Fn(&GetMessageMessage, &mut Window, &mut App) + 'static,
        on_open_list: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            pinned_messages: Rc::new(pinned_messages),
            theme,
            embedded: false,
            on_open_message: Rc::new(on_open_message),
            on_open_list: Rc::new(on_open_list),
        }
    }

    pub fn embedded(mut self, embedded: bool) -> Self {
        self.embedded = embedded;
        self
    }

    fn snippet_lines(title_color: Hsla, snippet_color: Hsla, snippet: String) -> Div {
        div()
            .w_full()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(1.0))
            .child(
                MainText::new(tr("pinned_bar_title"))
                    .px(13.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(title_color)
                    .number_of_lines(1),
            )
            .child(MainText::new(snippet).px(13.0).color(snippet_color).number_of_lines(1))
    }

    fn bar_content(
        &self,
        state: &Entity<ChatPinnedBarState>,
        message: &GetMessageMessage,
        displayed: ChatPinnedBarDisplayed,
    ) -> Div {
        let ChatPinnedBarDisplayed {
            index,
            snippet,
            thumb_source,
            swap,
            outgoing_snippet,
        } = displayed;
        let theme = &self.theme;
        let title_color = theme.primary._100;
        let snippet_color = theme.text.secondary;

        let press_state = state.clone();
        let pinned_messages = self.pinned_messages.clone();
        let on_open_message = self.on_open_message.clone();
        let opened = message.clone();
        let open_and_advance: ButtonAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            on_open_message(&opened, window, cx);

            let count = pinned_messages.len() as i64;
            if count <= 1 || index >= pinned_messages.len() {
                return;
            }
            let next_index = ChatPinnedCyclePolicy::next_index(index as i64, count) as usize;
            let next_id = stable_id(&pinned_messages[next_index]);
            press_state.update(cx, |state, cx| {
                state.active_message_id = Some(next_id);
                cx.notify();
            });
        });
        let on_open_list = self.on_open_list.clone();

        let is_swapping = swap < 1.0;
        let height_holder = Self::snippet_lines(title_color, snippet_color, snippet.clone())
            .opacity(if is_swapping { 0.0 } else { 1.0 });
        let incoming = is_swapping.then(|| {
            Self::snippet_lines(title_color, snippet_color, snippet)
                .absolute()
                .left_0()
                .right_0()
                .top(relative(swap - 1.0))
                .opacity(swap)
        });
        let outgoing = outgoing_snippet.map(|snippet| {
            Self::snippet_lines(title_color, snippet_color, snippet)
                .absolute()
                .left_0()
                .right_0()
                .top(relative(swap))
                .opacity(1.0 - swap)
        });

        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .child(
                div().flex_1().min_w_0().child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .pl(px(ChatHeaderAccessoryMetrics::LEADING_PADDING))
                        .py(px(ChatHeaderAccessoryMetrics::VERTICAL_PADDING))
                        .child(
                            PinnedNavigationStripe::new(
                                child_id(&self.id, "stripe"),
                                self.pinned_messages.len() as i64,
                                index as i64,
                                theme.primary._100,
                            ),
                        )
                        .children(thumb_source.map(|thumb| {
                            div().w(px(34.0)).h(px(34.0)).flex_none().child(
                                CleverImageUi::new(thumb)
                                    .id(child_id(&self.id, "thumb"))
                                    .corner_radius(8.0)
                                    .persist_to_disk(true)
                                    .maximum_decode_pixel_dimension(102_i64),
                            )
                        }))
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(height_holder)
                                .children(incoming)
                                .children(outgoing),
                        )
                        .child(div().w(px(8.0)).flex_none())
                        .liquid_press(
                            child_id(&self.id, "open"),
                            LiquidPressShape::RoundedRect(CORNER_RADIUS),
                            true,
                            false,
                            false,
                            Some(open_and_advance),
                        ),
                ),
            )
            .child(
                div().pr(px(4.0)).flex_none().child(
                    div()
                        .id(child_id(&self.id, "list-label"))
                        .tooltip_label(tr("pinned_bar_title"))
                        .w(px(38.0))
                        .h(px(38.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized("list.bullet", px(15.0), theme.text.secondary, false))
                        .liquid_press(
                            child_id(&self.id, "list"),
                            LiquidPressShape::Circle,
                            true,
                            false,
                            false,
                            Some(on_open_list),
                        ),
                ),
            )
    }
}

impl RenderOnce for ChatPinnedBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state: Entity<ChatPinnedBarState> =
            window.use_keyed_state(self.id.clone(), cx, |_, _| ChatPinnedBarState::new());
        let message_ids: Vec<String> = self.pinned_messages.iter().map(stable_id).collect();
        let ids_state = state.clone();
        on_change(child_id(&self.id, "ids"), message_ids, true, window, cx, move |previous, current, _, cx| {
            ids_state.update(cx, |state, cx| {
                state.synchronize_active_message(previous, current);
                cx.notify();
            });
        });

        let index = active_index(&self.pinned_messages, state.read(cx).active_message_id.as_deref());

        let Some(message) = self.pinned_messages.get(index).cloned() else {
            return div().into_any_element();
        };

        let thumb_source = message.first_grid_media_item().and_then(|first| {
            let source = first.display_source();
            let trimmed = trimming_whitespaces_and_newlines(&source);
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        });
        let snippet = if thumb_source.is_none() {
            MessagePreview::snippet(&message, false, cx)
        } else {
            MessagePreview::snippet_beside_thumbnail(&message, cx)
        };

        state.update(cx, |state, cx| state.note_displayed(&message.id, &snippet, cx));
        let now = RiseClock::monotonic();
        let (swap, outgoing_snippet) = {
            let state = state.read(cx);
            if state.swap.is_running(now) {
                window.request_animation_frame();
            }
            let swap = state.swap.value(now);
            (swap, state.outgoing_snippet.clone().filter(|_| swap < 1.0))
        };

        let content = self.bar_content(
            &state,
            &message,
            ChatPinnedBarDisplayed {
                index,
                snippet,
                thumb_source,
                swap,
                outgoing_snippet,
            },
        );
        if self.embedded {
            content.into_any_element()
        } else {
            div()
                .w_full()
                .px(px(10.0))
                .pt(px(4.0))
                .pb(px(6.0))
                .child(content.liquid_chrome_rounded_rectangle(CORNER_RADIUS))
                .into_any_element()
        }
    }
}

const STRIPE_WIDTH: f32 = 2.5;
const STRIPE_HEIGHT: f32 = 28.0;
const STRIPE_SEGMENT_SPACING: f32 = 2.0;
const STRIPE_VERTICAL_INSET: f32 = 3.0;
const STRIPE_MINIMUM_SEGMENT_HEIGHT: f32 = 5.5;
const STRIPE_CORNER_RADIUS: f32 = 1.25;
const STRIPE_CYCLE_DURATION: f64 = 0.09;

#[derive(Clone, Copy, PartialEq)]
struct NavigationState {
    count: i64,
    index: i64,
}

struct PinnedNavigationLayout {
    index: i64,
    segment_height: f32,
    step: f32,
    active_origin: f32,
    visible_indices: RangeInclusive<i64>,
}

impl PinnedNavigationLayout {
    fn resolved(count: i64, index: i64, height: f32) -> Self {
        let safe_count = count.max(1);
        let safe_index = index.max(0).min(safe_count - 1);
        let available_height = (height - STRIPE_VERTICAL_INSET * 2.0).max(1.0);
        let proposed_height = (available_height - STRIPE_SEGMENT_SPACING * safe_count as f32
            + STRIPE_SEGMENT_SPACING)
            / safe_count as f32;
        let segment_height = proposed_height.max(STRIPE_MINIMUM_SEGMENT_HEIGHT);
        let all_items_height =
            safe_count as f32 * segment_height + (safe_count - 1).max(0) as f32 * STRIPE_SEGMENT_SPACING;
        let step = segment_height + STRIPE_SEGMENT_SPACING;
        let top_items_height = safe_index as f32 * step;
        let bottom_items_height = all_items_height - top_items_height - segment_height;
        let mut active_origin = ((height - segment_height) * 0.5).floor();

        if active_origin - top_items_height > STRIPE_VERTICAL_INSET {
            active_origin = top_items_height + STRIPE_VERTICAL_INSET;
        }
        if active_origin + segment_height + bottom_items_height < height - STRIPE_VERTICAL_INSET {
            active_origin = height - STRIPE_VERTICAL_INSET - segment_height - bottom_items_height;
        }

        let lower_index = (safe_index - 5).max(0);
        let upper_index = (safe_index + 5).min(safe_count - 1);
        Self {
            index: safe_index,
            segment_height,
            step,
            active_origin,
            visible_indices: lower_index..=upper_index,
        }
    }

    /// where item zero sits
    /// inactive segments follow it so they keep their rail position while the active one travels
    fn rail_origin(&self) -> f32 {
        self.active_origin - self.index as f32 * self.step
    }
}

struct PinnedNavigationStripeState {
    active_origin: Animated<f32>,
    rail_origin: Animated<f32>,
    cycle_offset: Animated<f32>,
    cycle_generation: u64,
    navigation: NavigationState,
}

impl PinnedNavigationStripeState {
    fn new(layout: &PinnedNavigationLayout, navigation: NavigationState) -> Self {
        Self {
            active_origin: Animated::new(layout.active_origin),
            rail_origin: Animated::new(layout.rail_origin()),
            cycle_offset: Animated::new(0.0),
            cycle_generation: 0,
            navigation,
        }
    }

    fn follow_layout(&mut self, moved: bool, layout: &PinnedNavigationLayout, cx: &mut Context<Self>) {
        if moved {
            let spec = ease_in_out(SWAP_DURATION);
            let now = RiseClock::monotonic();
            self.active_origin.set_in(layout.active_origin, spec, now, cx);
            self.rail_origin.set_in(layout.rail_origin(), spec, now, cx);
        } else {
            self.active_origin.jump(layout.active_origin);
            self.rail_origin.jump(layout.rail_origin());
        }
    }

    fn is_running(&self, now: Duration) -> bool {
        self.active_origin.is_running(now) || self.rail_origin.is_running(now) || self.cycle_offset.is_running(now)
    }
}

fn animate_cycle_if_needed(
    state: &Entity<PinnedNavigationStripeState>,
    previous: NavigationState,
    current: NavigationState,
    cx: &mut App,
) {
    let cycles = previous.count == current.count
        && current.count > 4
        && previous.index == 0
        && current.index == current.count - 1;
    let generation = state.update(cx, |state, cx| {
        state.cycle_generation = state.cycle_generation.wrapping_add(1);
        if cycles {
            let spec = linear(STRIPE_CYCLE_DURATION);
            state.cycle_offset.set_in(-8.0, spec, RiseClock::monotonic(), cx);
        } else {
            state.cycle_offset.jump(0.0);
        }
        cx.notify();
        state.cycle_generation
    });
    if !cycles {
        return;
    }
    let weak_state = state.downgrade();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs_f64(STRIPE_CYCLE_DURATION)).await;
        let _ = weak_state.update(cx, |state, cx| {
            if state.cycle_generation != generation {
                return;
            }
            let spec = linear(STRIPE_CYCLE_DURATION);
            state.cycle_offset.set_in(0.0, spec, RiseClock::monotonic(), cx);
            cx.notify();
        });
    })
    .detach();
}

#[derive(IntoElement)]
struct PinnedNavigationStripe {
    id: ElementId,
    count: i64,
    index: i64,
    color: Hsla,
}

impl PinnedNavigationStripe {
    fn new(id: ElementId, count: i64, index: i64, color: Hsla) -> Self {
        Self { id, count, index, color }
    }
}

impl RenderOnce for PinnedNavigationStripe {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let height = STRIPE_HEIGHT;
        let layout = PinnedNavigationLayout::resolved(self.count, self.index, height);
        let safe_count = self.count.max(1);
        let navigation_state = NavigationState {
            count: safe_count,
            index: self.index.max(0).min(safe_count - 1),
        };
        let state: Entity<PinnedNavigationStripeState> = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            PinnedNavigationStripeState::new(&layout, navigation_state)
        });

        let previous_navigation = state.read(cx).navigation;
        if previous_navigation != navigation_state {
            let moved = previous_navigation.index != navigation_state.index;
            state.update(cx, |state, cx| {
                state.navigation = navigation_state;
                state.follow_layout(moved, &layout, cx);
            });
            animate_cycle_if_needed(&state, previous_navigation, navigation_state, cx);
        }

        let now = RiseClock::monotonic();
        let (active_origin, rail_origin, cycle_offset) = {
            let state = state.read(cx);
            if state.is_running(now) {
                window.request_animation_frame();
            }
            (state.active_origin.value(now), state.rail_origin.value(now), state.cycle_offset.value(now))
        };

        let segment = |origin: f32, color: Hsla| {
            div()
                .absolute()
                .left_0()
                .top(px(origin))
                .w(px(STRIPE_WIDTH))
                .h(px(layout.segment_height))
                .rounded(px(STRIPE_CORNER_RADIUS))
                .bg(color)
        };
        let rail = layout
            .visible_indices
            .clone()
            .map(|item_index| segment(rail_origin + item_index as f32 * layout.step, self.color.opacity(0.3)));

        div()
            .relative()
            .flex_none()
            .w(px(STRIPE_WIDTH))
            .h(px(height))
            .top(px(cycle_offset))
            .overflow_hidden()
            .children(rail)
            .child(segment(active_origin, self.color))
    }
}
