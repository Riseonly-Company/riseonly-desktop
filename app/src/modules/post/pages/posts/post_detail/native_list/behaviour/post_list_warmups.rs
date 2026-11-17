use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{App, Task};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::power::system_low_power_mode;
use rise_platform::process_info::{ThermalState, thermal_state};

use crate::modules::post::components::post::post_caption_prewarm::PostCaptionPrewarm;
use crate::modules::post::components::post_feed_media_prefetch::PostFeedMediaPrefetchController;
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_preparer::{
    PostRowHostedHeightLookup, PostRowPrepareEnvironment, PostRowPreparePriority, PostRowPreparer,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum TextSlot {
    Above,
    Below,
    Appended,
}

pub struct PostListWarmups {
    this: Weak<PostListWarmups>,
    preparer: Rc<PostRowPreparer>,
    hosted_height: Option<PostRowHostedHeightLookup>,
    posts: Rc<dyn Fn(&App) -> Rc<Vec<GetPostFeedResponse>>>,
    environment: Rc<dyn Fn(&mut App) -> Option<PostRowPrepareEnvironment>>,
    media_prefetch: RefCell<PostFeedMediaPrefetchController>,
    is_active: Cell<bool>,
    is_stopped: Cell<bool>,
    flush_scheduled: Cell<bool>,
    flush_token: Cell<u64>,
    pending_above: Cell<Option<usize>>,
    pending_below: Cell<Option<(usize, usize)>>,
    pending_appended_from: Cell<Option<usize>>,
    last_below: Cell<Option<(usize, usize)>>,
    text_tasks: RefCell<HashMap<TextSlot, Task<()>>>,
    text_generations: RefCell<HashMap<TextSlot, u64>>,
}

impl PostListWarmups {
    pub const TEXT_ROWS: usize = 6;

    /// posts gives the rendered items the window indices point
    /// at, environment is none until the list has a usable width
    pub fn new(
        preparer: Rc<PostRowPreparer>,
        hosted_height: Option<PostRowHostedHeightLookup>,
        posts: Rc<dyn Fn(&App) -> Rc<Vec<GetPostFeedResponse>>>,
        environment: Rc<dyn Fn(&mut App) -> Option<PostRowPrepareEnvironment>>,
    ) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            preparer,
            hosted_height,
            posts,
            environment,
            media_prefetch: RefCell::new(PostFeedMediaPrefetchController::new(&Uuid::new_v4().uuid_string())),
            is_active: Cell::new(true),
            is_stopped: Cell::new(false),
            flush_scheduled: Cell::new(false),
            flush_token: Cell::new(0),
            pending_above: Cell::new(None),
            pending_below: Cell::new(None),
            pending_appended_from: Cell::new(None),
            last_below: Cell::new(None),
            text_tasks: RefCell::new(HashMap::new()),
            text_generations: RefCell::new(HashMap::new()),
        })
    }

    pub fn activate(&self) {
        if self.is_stopped.get() {
            return;
        }
        self.is_active.set(true);
    }

    pub fn deactivate(&self, cx: &mut App) {
        self.is_active.set(false);
        self.flush_token.set(self.flush_token.get().wrapping_add(1));
        self.flush_scheduled.set(false);
        self.pending_above.set(None);
        self.pending_below.set(None);
        self.pending_appended_from.set(None);
        self.last_below.set(None);
        self.media_prefetch.borrow_mut().cancel_all(cx);
        self.text_tasks.borrow_mut().clear();
    }

    pub fn stop(&self, cx: &mut App) {
        self.deactivate(cx);
        self.is_stopped.set(true);
    }

    /// window top moved so load media and text for the rows revealed next
    pub fn window_top_did_move(&self, lower: usize, cx: &mut App) {
        self.pending_above.set(Some(lower));
        self.schedule_flush(cx);
    }

    pub fn row_did_appear(&self, index: usize, lower: usize, cx: &mut App) {
        if index <= lower || self.last_below.get() == Some((index, lower)) {
            return;
        }
        self.pending_below.set(Some((index, lower)));
        self.last_below.set(Some((index, lower)));
        self.schedule_flush(cx);
    }

    pub fn rows_did_append(&self, from: usize, lower: usize, cx: &mut App) {
        let pending = self.pending_appended_from.get().map_or(from, |pending| pending.min(from));
        self.pending_appended_from.set(Some(pending));
        if self.pending_below.get().is_none()
            && let Some((index, _)) = self.last_below.get()
        {
            self.pending_below.set(Some((index, lower)));
        }
        self.schedule_flush(cx);
    }

    fn schedule_flush(&self, cx: &mut App) {
        if !self.is_active.get() || self.is_stopped.get() || self.flush_scheduled.get() {
            return;
        }
        self.flush_scheduled.set(true);
        let token = self.flush_token.get();
        let this = self.this.clone();
        cx.defer(move |cx| {
            if let Some(this) = this.upgrade()
                && this.flush_token.get() == token
            {
                this.flush(cx);
            }
        });
    }

    fn flush(&self, cx: &mut App) {
        self.flush_scheduled.set(false);
        if !self.is_active.get() || self.is_stopped.get() {
            return;
        }
        let posts = (self.posts)(cx);
        let count = posts.len();
        if let Some(requested) = self.pending_above.take() {
            let lower = requested.min(count);
            let lower_index = lower as i64;
            self.media_prefetch.borrow_mut().update_edge_window(
                "above",
                lower_index,
                &posts,
                true,
                (lower_index - 1).max(0)..lower_index,
                cx,
            );
            self.warm_text(&posts[lower.saturating_sub(Self::TEXT_ROWS)..lower], TextSlot::Above, true, cx);
        }
        if let Some((index, below_lower)) = self.pending_below.take() {
            let lower = below_lower.min(count);
            self.media_prefetch.borrow_mut().update_edge_window(
                "below",
                index as i64,
                &posts,
                false,
                lower as i64..count as i64,
                cx,
            );
            let start = index.saturating_add(1).min(count);
            self.warm_text(&posts[start..start.saturating_add(Self::TEXT_ROWS).min(count)], TextSlot::Below, true, cx);
        }
        if let Some(from) = self.pending_appended_from.take() {
            self.warm_text(&posts[from.min(count)..], TextSlot::Appended, false, cx);
        }
    }

    fn warm_text(&self, slice: &[GetPostFeedResponse], slot: TextSlot, includes_native: bool, cx: &mut App) {
        if slice.is_empty() {
            return;
        }
        let environment = if includes_native && Self::allows_speculative_text() { (self.environment)(cx) } else { None };
        let mut hosted = Vec::new();
        let mut native = Vec::new();
        for post in slice {
            let input = self.preparer.input(post, cx);
            if input.route.is_hosted_fallback() {
                hosted.push(post.clone());
                continue;
            }
            if let Some(environment) = environment.as_ref()
                && self.preparer.cached_row(&input, environment).is_none()
            {
                native.push(input);
            }
        }
        if !hosted.is_empty() {
            PostCaptionPrewarm::warm(&hosted, cx);
        }
        let Some(environment) = environment else {
            return;
        };
        if native.is_empty() {
            return;
        }

        self.text_tasks.borrow_mut().remove(&slot);
        let generation = self.text_generations.borrow().get(&slot).copied().unwrap_or(0).wrapping_add(1);
        self.text_generations.borrow_mut().insert(slot, generation);
        let work = self.preparer.prepare(
            native,
            &environment,
            generation,
            PostRowPreparePriority::Warm,
            self.hosted_height.clone(),
            cx,
        );
        let this = self.this.clone();
        let task = cx.spawn(async move |cx| {
            let _ = work.await;
            cx.update(|_| {
                if let Some(this) = this.upgrade() {
                    this.text_task_did_finish(slot, generation);
                }
            });
        });
        self.text_tasks.borrow_mut().insert(slot, task);
    }

    fn text_task_did_finish(&self, slot: TextSlot, generation: u64) {
        if self.text_generations.borrow().get(&slot).copied() != Some(generation) {
            return;
        }
        if let Some(task) = self.text_tasks.borrow_mut().remove(&slot) {
            task.detach();
        }
    }

    fn allows_speculative_text() -> bool {
        match thermal_state() {
            ThermalState::Serious | ThermalState::Critical => false,
            ThermalState::Nominal | ThermalState::Fair => !system_low_power_mode(),
        }
    }
}
