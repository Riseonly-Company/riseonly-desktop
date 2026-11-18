use std::rc::Rc;
use std::sync::Arc;

use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, App, ElementId, Entity, FontWeight, IntoElement, RenderOnce, SharedString, Task,
    Window, relative,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out};
use rise_ui::swiftui::prelude::*;

use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::stores::message::message_actions::message_types::{MessagePollInfo, MessagePollOption};

/// sends the viewer whole answer and says if it was counted
pub type MessagePollVote = Rc<dyn Fn(Vec<String>, &mut App) -> Task<bool>>;

/// why: vote moves tallies, bars and radio in one eased step instead of jumping to results
fn vote_animation() -> MotionSpec {
    ease_in_out(0.3)
}

/// the two state values of one bubble
#[derive(Default)]
struct MessagePollViewState {
    pending: Option<Vec<String>>,
    sending: bool,
}

#[derive(IntoElement)]
pub struct MessagePollView {
    id: ElementId,
    pub poll: MessagePollInfo,
    pub is_own: bool,
    pub on_vote: Option<MessagePollVote>,
    pub on_show_voters: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl MessagePollView {
    pub fn new(id: impl Into<ElementId>, poll: MessagePollInfo) -> Self {
        Self {
            id: id.into(),
            poll,
            is_own: false,
            on_vote: None,
            on_show_voters: None,
        }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn on_vote(mut self, on_vote: impl Fn(Vec<String>, &mut App) -> Task<bool> + 'static) -> Self {
        self.on_vote = Some(Rc::new(on_vote));
        self
    }

    pub fn on_show_voters(mut self, on_show_voters: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_show_voters = Some(Rc::new(on_show_voters));
        self
    }

    fn child_id(&self, suffix: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), suffix.into())
    }

    fn chosen(poll: &MessagePollInfo, state: &MessagePollViewState) -> Vec<String> {
        state.pending.clone().unwrap_or_else(|| poll.chosen_option_ids.clone())
    }

    fn is_quiz(poll: &MessagePollInfo) -> bool {
        poll.quiz_correct_id.is_some()
    }

    /// tallies show once the viewer answered or the poll closed
    fn shows_results(poll: &MessagePollInfo, chosen: &[String]) -> bool {
        !chosen.is_empty() || poll.closed
    }

    fn tap(
        id: &str,
        poll: &MessagePollInfo,
        on_vote: Option<&MessagePollVote>,
        state: &Entity<MessagePollViewState>,
        cx: &mut App,
    ) {
        let Some(on_vote) = on_vote else {
            return;
        };
        if poll.closed {
            return;
        }
        let current = Self::chosen(poll, state.read(cx));
        let is_quiz = Self::is_quiz(poll);
        if is_quiz && !current.is_empty() {
            return;
        }
        let next: Vec<String> = if poll.allows_multiple && !is_quiz {
            if current.iter().any(|chosen| chosen == id) {
                current.into_iter().filter(|chosen| chosen != id).collect()
            } else {
                let mut next = current;
                next.push(id.to_owned());
                next
            }
        } else if current.len() == 1 && current[0] == id {
            Vec::new()
        } else {
            vec![id.to_owned()]
        };
        let sending = state.update(cx, |state, cx| {
            state.pending = Some(next.clone());
            cx.notify();
            state.sending
        });
        if sending {
            return;
        }
        Self::send(next, on_vote.clone(), state.clone(), cx);
    }

    /// one vote at a time per bubble, when it lands the latest tapped answer goes next if it differs
    fn send(answer: Vec<String>, on_vote: MessagePollVote, state: Entity<MessagePollViewState>, cx: &mut App) {
        state.update(cx, |state, cx| {
            state.sending = true;
            cx.notify();
        });
        let vote = on_vote(answer.clone(), cx);
        let weak_state = state.downgrade();
        cx.spawn(async move |cx| {
            let counted = vote.await;
            let _ = cx.update(|cx| {
                let Some(state) = weak_state.upgrade() else {
                    return;
                };
                let latest = state.update(cx, |state, cx| {
                    state.sending = false;
                    cx.notify();
                    state.pending.clone()
                });
                if let Some(latest) = latest
                    && latest != answer
                    && counted
                {
                    Self::send(latest, on_vote, state, cx);
                    return;
                }
                if !counted {
                    state.update(cx, |state, cx| {
                        state.pending = None;
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn option_row(
        &self,
        opt: &MessagePollOption,
        voter_count: i64,
        total_votes: i64,
        theme: &AppTheme,
        chosen: &[String],
        state: &Entity<MessagePollViewState>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let pct = voter_count as f64 / total_votes as f64;
        let voted = chosen.iter().any(|chosen| chosen == &opt.id);
        let shows_results = Self::shows_results(&self.poll, chosen);
        let is_correct = self.poll.quiz_correct_id.as_deref() == Some(opt.id.as_str()) && shows_results;
        let enabled = self.on_vote.is_some() && !self.poll.closed && !(Self::is_quiz(&self.poll) && !chosen.is_empty());

        let ring_color = if self.is_own { theme.content_on(theme.primary._100).opacity(0.55) } else { theme.primary._100.opacity(0.7) };
        let dot_color = if self.is_own { theme.content_on(theme.primary._100) } else { theme.primary._100 };
        let now = RiseClock::monotonic();
        let dot_target: f32 = if voted { 1.0 } else { 0.0 };
        let dot_state = window.use_keyed_state(self.child_id(format!("dot-{}", opt.id)), cx, |_, _| Animated::new(dot_target));
        dot_state.update(cx, |animated, cx| animated.set_in(dot_target, vote_animation(), now, cx));
        let dot = dot_state.read(cx).frame_value(now, window);
        let dot_side = 12.0 * dot.max(0.01);
        let radio = zstack(Alignment::Center)
            .frame_size(18.0, 18.0)
            .child(centred_stroke(ring_color, 1.5, Shape::Circle))
            .child(div().circle().bg(dot_color.opacity(dot.clamp(0.0, 1.0))).frame_size(dot_side, dot_side));

        let mut header = hstack(8.0)
            .child(radio)
            .child(
                MainText::new(opt.text.clone())
                    .px(14.0)
                    .color(if self.is_own { theme.content_on(theme.primary._100) } else { theme.text.primary })
                    .number_of_lines(2),
            )
            .child(spacer());
        if shows_results {
            header = header.child(
                MainText::new(format!("{}%", (pct * 100.0) as i64))
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(if self.is_own { theme.content_on(theme.primary._100).opacity(0.92) } else { theme.text.secondary }),
            );
        }

        let bar_target = if shows_results { pct.max(0.0) as f32 } else { 0.0 };
        let bar_state = window.use_keyed_state(self.child_id(format!("bar-{}", opt.id)), cx, |_, _| Animated::new(bar_target));
        bar_state.update(cx, |animated, cx| animated.set_in(bar_target, vote_animation(), now, cx));
        let bar = bar_state.read(cx).frame_value(now, window);
        let bar_opacity_target: f32 = if shows_results { 1.0 } else { 0.0 };
        let bar_opacity_state =
            window.use_keyed_state(self.child_id(format!("bar-opacity-{}", opt.id)), cx, |_, _| Animated::new(bar_opacity_target));
        bar_opacity_state.update(cx, |animated, cx| animated.set_in(bar_opacity_target, vote_animation(), now, cx));
        let bar_opacity = bar_opacity_state.read(cx).frame_value(now, window).clamp(0.0, 1.0);

        let row = {
            let track = if self.is_own { theme.content_on(theme.primary._100).opacity(0.18) } else { theme.text.secondary.opacity(0.15) };
            let fill = if is_correct {
                theme.semantic.success_100
            } else if self.is_own {
                theme.content_on(theme.primary._100)
            } else {
                theme.primary._100
            };
            vstack(4.0).align(HorizontalAlignment::Leading).w_full().child(header.w_full()).child(
                zstack(Alignment::Leading)
                    .w_full()
                    .h(px(4.0))
                    .opacity(bar_opacity)
                    .child(div().w_full().h(px(4.0)).capsule().bg(track))
                    .child(div().w(relative(bar.max(0.0))).h(px(4.0)).capsule().bg(fill)),
            )
        };

        let option_id = opt.id.clone();
        let poll = self.poll.clone();
        let on_vote = self.on_vote.clone();
        let state = state.clone();
        div()
            .w_full()
            .child(row.bubble_tap_target(
                self.child_id(format!("option-{}", opt.id)),
                move |_window: &mut Window, cx: &mut App| {
                    if enabled {
                        Self::tap(&option_id, &poll, on_vote.as_ref(), &state, cx);
                    }
                },
            ))
            .into_any_element()
    }

    fn footer_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(if self.poll.is_anonymous { tr("chat_poll_anonymous") } else { tr("chat_poll_public") });
        if self.poll.allows_multiple {
            parts.push(tr("chat_poll_multi"));
        }
        if self.poll.closed {
            parts.push(tr("chat_poll_closed"));
        }
        parts.join(" · ")
    }

    fn people_voted_label(n: i64) -> String {
        if n == 0 {
            return tr("chat_poll_no_votes");
        }
        format!("{} {}", tr("chat_poll_voters"), n)
    }
}

impl RenderOnce for MessagePollView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme().clone();

        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| MessagePollViewState::default());
        let watched = state.clone();
        on_change(
            self.child_id("chosen-change"),
            self.poll.chosen_option_ids.clone(),
            false,
            window,
            cx,
            move |_, _, _, cx| {
                watched.update(cx, |state, cx| {
                    if !state.sending {
                        state.pending = None;
                        cx.notify();
                    }
                });
            },
        );
        let chosen = Self::chosen(&self.poll, state.read(cx));
        let tally = MessagePollTally::new(&self.poll, &chosen);
        let total_votes = tally.total_voters.max(1);

        let secondary = if self.is_own { theme.content_on(theme.primary._100).opacity(0.7) } else { theme.text.secondary };
        let header = vstack(4.0)
            .align(HorizontalAlignment::Leading)
            .child(
                MainText::new(self.poll.question.clone())
                    .px(15.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(if self.is_own { theme.content_on(theme.primary._100) } else { theme.text.primary })
                    .number_of_lines(6),
            )
            .child(MainText::new(self.footer_line()).px(11.0).color(secondary).number_of_lines(1));

        let mut rows: Vec<AnyElement> = Vec::with_capacity(self.poll.options.len());
        for opt in &self.poll.options {
            let voter_count = tally.counts.get(&opt.id).copied().unwrap_or(opt.voter_count);
            rows.push(self.option_row(opt, voter_count, total_votes, &theme, &chosen, &state, window, cx));
        }
        let options = vstack(8.0).align(HorizontalAlignment::Leading).w_full().children(rows);

        let voters_label = Self::people_voted_label(tally.total_voters);
        let voters_text: AnyElement = match self.on_show_voters.clone() {
            Some(on_show_voters) if tally.total_voters > 0 => {
                MainText::new(voters_label)
                    .px(12.0)
                    .color(if self.is_own { theme.content_on(theme.primary._100) } else { theme.primary._100 })
                    .bubble_tap_target(self.child_id("show-voters"), move |window: &mut Window, cx: &mut App| {
                        on_show_voters(window, cx);
                    })
                    .into_any_element()
            }
            _ => MainText::new(voters_label)
                .px(12.0)
                .color(if self.is_own { theme.content_on(theme.primary._100).opacity(0.78) } else { theme.text.secondary })
                .into_any_element(),
        };
        let voters = hstack(None).w_full().child(voters_text).child(spacer());

        vstack(10.0)
            .align(HorizontalAlignment::Leading)
            .w(px(280.0))
            .min_w(px(0.0))
            .max_w_full()
            .child(header)
            .child(options)
            .child(voters)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessagePollTally {
    pub counts: HashMap<String, i64>,
    pub total_voters: i64,
}

impl MessagePollTally {
    pub fn new(poll: &MessagePollInfo, chosen: &[String]) -> Self {
        let confirmed: HashSet<&str> = poll.chosen_option_ids.iter().map(String::as_str).collect();
        let answer: HashSet<&str> = chosen.iter().map(String::as_str).collect();
        let mut counts: HashMap<String, i64> = HashMap::with_capacity(poll.options.len());
        if confirmed == answer {
            for option in &poll.options {
                counts.entry(option.id.clone()).or_insert(option.voter_count);
            }
            return Self {
                counts,
                total_voters: poll.total_voters,
            };
        }
        for option in &poll.options {
            let delta = i64::from(answer.contains(option.id.as_str())) - i64::from(confirmed.contains(option.id.as_str()));
            counts.entry(option.id.clone()).or_insert((option.voter_count + delta).max(0));
        }
        let voter_delta = i64::from(!answer.is_empty()) - i64::from(!confirmed.is_empty());
        Self {
            counts,
            total_voters: (poll.total_voters + voter_delta).max(0),
        }
    }
}
