use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, Pixels, Window, px, relative};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::bottom_sheet_screen::{BottomSheetNavigationState, BottomSheetScreenItem};
use rise_ui::keyboard_dismiss::KeyboardDismiss;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{duration_from_seconds, ease_out};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_page_back_swipe_gate::{
    BottomSheetPageBackSwipeCallbacks, BottomSheetPageBackSwipeConfiguration,
    BottomSheetPageBackSwipeGate, BottomSheetPageBackSwipePolicy,
};

pub type BottomSheetPageDeckClose = Rc<dyn Fn(&mut Window, &mut App)>;
/// fraction and duration of the sheet height the top page declares
pub type BottomSheetPageDeckTransition = Rc<dyn Fn(f32, f64, &mut Window, &mut App)>;

/// constructor, callers fill every field
#[derive(IntoElement)]
pub struct BottomSheetPageDeck {
    pub id: ElementId,
    pub pages: Vec<BottomSheetScreenItem>,
    pub root_page_id: String,
    pub initial_page_ids: Vec<String>,
    pub navigation: Entity<BottomSheetNavigationState>,
    pub background: Hsla,
    pub bottom_inset: f32,
    pub is_locked: bool,
    pub shows_close_button: bool,
    pub on_close: BottomSheetPageDeckClose,
    pub on_transition: BottomSheetPageDeckTransition,
}

/// what the deck state reads from the latest render
#[derive(Clone)]
struct DeckProps {
    pages: Vec<BottomSheetScreenItem>,
    root_page_id: String,
    initial_page_ids: Vec<String>,
    navigation: Entity<BottomSheetNavigationState>,
    is_locked: bool,
    on_transition: BottomSheetPageDeckTransition,
    reduce_motion: bool,
}

/// deck state
/// active_index and drag_offset are the model values and the strip draws their animated twins
struct DeckState {
    props: DeckProps,
    stack: Vec<String>,
    active_index: usize,
    drag_offset: f32,
    strip_index: Animated<f32>,
    strip_drag_offset: Animated<f32>,
    is_settling: bool,
    page_width: f32,
}

impl DeckState {
    fn new(props: DeckProps) -> Self {
        Self {
            props,
            stack: Vec::new(),
            active_index: 0,
            drag_offset: 0.0,
            strip_index: Animated::new(0.0),
            strip_drag_offset: Animated::new(0.0),
            is_settling: false,
            page_width: 0.0,
        }
    }

    /// first page wins on a duplicate id
    fn page_map(&self) -> HashMap<String, BottomSheetScreenItem> {
        let mut map = HashMap::new();
        for page in &self.props.pages {
            map.entry(page.id.clone()).or_insert_with(|| page.clone());
        }
        map
    }

    fn can_go_back(&self) -> bool {
        self.active_index > 0
    }

    /// shell animates sheet height with this duration and cuts it to one frame under reduce motion
    /// without the same cut here the sheet jumped instantly while the page kept sliding for 280 ms
    fn transition_duration(&self) -> f64 {
        if self.props.reduce_motion { 0.01 } else { BottomSheetMetrics::PAGE_TRANSITION_DURATION }
    }

    /// animates active index to its value and drag offset to 0 with ease out
    fn animate_strip(&mut self, duration: f64) {
        let now = RiseClock::monotonic();
        self.strip_index.set(self.active_index as f32, ease_out(duration), now, false);
        self.strip_drag_offset.set(self.drag_offset, ease_out(duration), now, false);
    }

    fn reset_navigation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pages = self.page_map();
        self.stack = BottomSheetPageDeck::validated_stack(
            &self.props.initial_page_ids,
            &self.props.root_page_id,
            &pages,
        );
        self.active_index = self.stack.len() - 1;
        self.drag_offset = 0.0;
        self.strip_index.jump(self.active_index as f32);
        self.strip_drag_offset.jump(0.0);
        self.is_settling = false;
        self.install_navigation_callbacks(cx);
        let top = self.stack[self.active_index].clone();
        let can_go_back = self.active_index > 0;
        self.props.navigation.update(cx, |navigation, cx| navigation.sync(&top, can_go_back, cx));
        if let Some(page) = pages.get(&top) {
            (self.props.on_transition)(page.fraction, self.transition_duration(), window, cx);
        }
        cx.notify();
    }

    /// closures reach the deck through its handle and read props of the latest render
    /// so is_locked inside them cant freeze at mount state
    /// header button and navigation always agree
    fn install_navigation_callbacks(&self, cx: &mut Context<Self>) {
        let navigate = cx.weak_entity();
        let go_back = cx.weak_entity();
        self.props.navigation.update(cx, |navigation, _| {
            navigation.set_on_navigate(move |page_id, window, cx| {
                navigate.update(cx, |deck, cx| deck.push(&page_id, window, cx)).ok();
            });
            navigation.set_on_go_back(move |window, cx| {
                go_back.update(cx, |deck, cx| deck.go_back(window, cx)).ok();
            });
        });
    }

    fn push(&mut self, page_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_settling || self.props.is_locked || self.page_width <= 1.0 {
            return;
        }
        let pages = self.page_map();
        let Some(page) = pages.get(page_id) else {
            return;
        };
        let visited = &self.stack[..=self.active_index];
        if visited.iter().any(|id| id == page_id) {
            return;
        }

        KeyboardDismiss::perform(cx);
        self.is_settling = true;
        let mut stack = visited.to_vec();
        stack.push(page_id.to_owned());
        self.stack = stack;
        self.props.navigation.update(cx, |navigation, cx| navigation.sync(page_id, true, cx));
        let duration = self.transition_duration();
        (self.props.on_transition)(page.fraction, duration, window, cx);

        self.active_index = self.stack.len() - 1;
        self.drag_offset = 0.0;
        self.animate_strip(duration);
        cx.notify();
        Self::after(duration, cx, |deck, _| deck.is_settling = false);
    }

    fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_settling || self.props.is_locked {
            return;
        }
        let duration = self.transition_duration();
        self.pop(duration, window, cx);
    }

    fn pop(&mut self, duration: f64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_go_back() || self.active_index > self.stack.len() {
            return;
        }

        KeyboardDismiss::perform(cx);
        self.is_settling = true;
        let target_index = self.active_index - 1;
        let target_id = self.stack[target_index].clone();
        self.props
            .navigation
            .update(cx, |navigation, cx| navigation.sync(&target_id, target_index > 0, cx));
        if let Some(target) = self.page_map().get(&target_id) {
            (self.props.on_transition)(target.fraction, duration, window, cx);
        }

        self.active_index = target_index;
        self.drag_offset = 0.0;
        self.animate_strip(duration);
        cx.notify();
        Self::after(duration, cx, move |deck, _| {
            if deck.stack.len() > target_index + 1 {
                deck.stack.truncate(target_index + 1);
            }
            deck.is_settling = false;
        });
    }

    /// completion that runs once the curve had its duration
    /// a deck rebuilt meanwhile is a new entity so the late completion of the old one lands nowhere
    fn after(
        duration: f64,
        cx: &mut Context<Self>,
        completion: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let delay = duration_from_seconds(duration);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |deck, cx| {
                completion(deck, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn track_drag(&mut self, translation: Pixels, width: f32, cx: &mut Context<Self>) {
        if !self.can_go_back() || self.is_settling || self.props.is_locked || width <= 1.0 {
            return;
        }
        let clamped = f32::from(translation).max(0.0).min(width);
        if !((clamped - self.drag_offset).abs() > BottomSheetMetrics::PAGE_DRAG_EPSILON
            || (clamped == 0.0 && self.drag_offset != 0.0))
        {
            return;
        }

        self.drag_offset = clamped;
        self.strip_drag_offset.jump(clamped);
        cx.notify();
    }

    fn release_drag(
        &mut self,
        translation: Pixels,
        velocity: Pixels,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_go_back() || self.is_settling || self.props.is_locked || width <= 1.0 {
            self.cancel_drag(cx);
            return;
        }

        if !BottomSheetPageBackSwipePolicy::commits(translation, velocity, px(width)) {
            self.cancel_drag(cx);
            return;
        }

        let duration = self.release_duration(f32::from(velocity), width);
        self.pop(duration, window, cx);
    }

    fn cancel_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag_offset == 0.0 {
            return;
        }
        let duration = if self.props.reduce_motion {
            0.01
        } else {
            BottomSheetMetrics::PAGE_MINIMUM_TRANSITION_DURATION
        };
        self.drag_offset = 0.0;
        self.strip_drag_offset.set(0.0, ease_out(duration), RiseClock::monotonic(), false);
        cx.notify();
    }

    fn release_duration(&self, velocity: f32, width: f32) -> f64 {
        if self.props.reduce_motion {
            return 0.01;
        }
        let normalized = (velocity.abs() / width.max(1.0)).min(3.0);
        BottomSheetMetrics::PAGE_MINIMUM_TRANSITION_DURATION
            .max(BottomSheetMetrics::PAGE_TRANSITION_DURATION - f64::from(normalized) * 0.03)
    }
}

impl BottomSheetPageDeck {
    pub fn validated_stack(
        requested: &[String],
        root_page_id: &str,
        pages: &HashMap<String, BottomSheetScreenItem>,
    ) -> Vec<String> {
        let unique: HashSet<&String> = requested.iter().collect();
        let accepted = requested.first().is_some_and(|first| first == root_page_id)
            && requested.iter().all(|id| pages.contains_key(id))
            && unique.len() == requested.len();
        if !accepted {
            return vec![root_page_id.to_owned()];
        }
        requested.to_vec()
    }

    fn page_view(
        &self,
        deck: &Entity<DeckState>,
        page: &BottomSheetScreenItem,
        is_root: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let header = if !is_root || self.shows_close_button {
            let on_close = self.on_close.clone();
            let deck = deck.downgrade();
            SheetHeaderBar::new(
                (self.id.clone(), format!("header-{}", page.id)),
                page.title.clone(),
                move |window, cx| {
                    if is_root {
                        on_close(window, cx);
                    } else {
                        deck.update(cx, |deck, cx| deck.go_back(window, cx)).ok();
                    }
                },
            )
            .left_system_image(if is_root { "xmark" } else { "chevron.backward" })
            .right_system_image(None)
            .into_any_element()
        } else {
            div()
                .w_full()
                .px(px(14.0))
                .py(px(10.0))
                .child(
                    MainText::new(page.title.clone())
                        .px(17.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .pb(px(self.bottom_inset))
            .bg(self.background)
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child((page.content)(window, cx)),
            )
            .into_any_element()
    }
}

impl RenderOnce for BottomSheetPageDeck {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let props = DeckProps {
            pages: self.pages.clone(),
            root_page_id: self.root_page_id.clone(),
            initial_page_ids: self.initial_page_ids.clone(),
            navigation: self.navigation.clone(),
            is_locked: self.is_locked,
            on_transition: self.on_transition.clone(),
            reduce_motion: reduce_motion(cx),
        };
        let initial = props.clone();
        let deck = window.use_keyed_state((self.id.clone(), "state"), cx, move |_, _| DeckState::new(initial));
        deck.update(cx, |state, _| state.props = props);

        let reinstall = deck.clone();
        on_change((self.id.clone(), "is-locked"), self.is_locked, false, window, cx, move |_, _, _, cx| {
            reinstall.update(cx, |state, cx| state.install_navigation_callbacks(cx));
        });

        let now = RiseClock::monotonic();
        let (stack, pages, can_go_back, is_settling, strip_index, drag_offset) = {
            let state = deck.read(cx);
            (
                state.stack.clone(),
                state.page_map(),
                state.can_go_back(),
                state.is_settling,
                state.strip_index.frame_value(now, window),
                state.strip_drag_offset.frame_value(now, window),
            )
        };

        let changed = deck.downgrade();
        let ended = deck.downgrade();
        let cancelled = deck.downgrade();
        let gate = BottomSheetPageBackSwipeGate::new(
            (self.id.clone(), "back-swipe"),
            BottomSheetPageBackSwipeConfiguration { is_enabled: can_go_back && !is_settling && !self.is_locked },
            BottomSheetPageBackSwipeCallbacks {
                on_began: Rc::new(|_: &mut Window, _: &mut App| {}),
                on_changed: Rc::new(move |translation: Pixels, _: &mut Window, cx: &mut App| {
                    changed
                        .update(cx, |deck, cx| {
                            let width = deck.page_width;
                            deck.track_drag(translation, width, cx);
                        })
                        .ok();
                }),
                on_ended: Rc::new(move |translation: Pixels, velocity: Pixels, window: &mut Window, cx: &mut App| {
                    ended
                        .update(cx, |deck, cx| {
                            let width = deck.page_width;
                            deck.release_drag(translation, velocity, width, window, cx);
                        })
                        .ok();
                }),
                on_cancelled: Rc::new(move |_: &mut Window, cx: &mut App| {
                    cancelled.update(cx, |deck, cx| deck.cancel_drag(cx)).ok();
                }),
            },
        );

        let root_page_id = stack.first().cloned();
        let mut strip = div().absolute().top_0().left(px(drag_offset)).size_full();
        for (slot, page_id) in stack.iter().enumerate() {
            let Some(page) = pages.get(page_id) else {
                continue;
            };
            let is_root = root_page_id.as_deref() == Some(page_id.as_str());
            let content = self.page_view(&deck, page, is_root, window, cx);
            strip = strip.child(
                div()
                    .id((self.id.clone(), format!("page-{page_id}")))
                    .absolute()
                    .top_0()
                    .left(relative(slot as f32 - strip_index))
                    .size_full()
                    .child(content),
            );
        }

        let measured = deck.downgrade();
        let appeared = deck.clone();
        div()
            .id((self.id.clone(), "deck"))
            .relative()
            .size_full()
            .overflow_hidden()
            .child(gate)
            .child(strip)
            .child(measure((self.id.clone(), "width"), move |bounds, _, cx| {
                measured
                    .update(cx, |deck, cx| {
                        deck.page_width = f32::from(bounds.size.width);
                        cx.notify();
                    })
                    .ok();
            }))
            .on_appear((self.id, "appear"), move |window, cx| {
                appeared.update(cx, |deck, cx| deck.reset_navigation(window, cx));
            })
    }
}
