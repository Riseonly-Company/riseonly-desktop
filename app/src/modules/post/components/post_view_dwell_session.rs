use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, Task};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementSurface;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::post::engine::rise_post_engine_models::RisePostViewSurface;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::user::engine::rise_meaningful_activity::RiseMeaningfulActivity;

/// view qualification for one post on one surface
/// 500 ms dwell, retry backoff and skip
/// knows nothing about views, the owner computes qualification
pub struct PostViewDwellSession {
    post_id: String,
    surface: RisePostViewSurface,
    is_visible: Rc<dyn Fn(&App) -> bool>,
    scope_id: Uuid,
    scope_generation: u64,
    is_qualified: bool,
    reported: bool,
    retry_attempt: usize,
    dwell_task: Option<Task<()>>,
    visible_since_uptime: Option<Duration>,
    is_source_active: bool,
}

impl PostViewDwellSession {
    const DWELL_NANOSECONDS: u64 = 500_000_000;

    const RETRY_DELAYS_NANOSECONDS: [u64; 5] =
        [750_000_000, 1_500_000_000, 3_000_000_000, 6_000_000_000, 8_000_000_000];

    const SKIP_LOWER_BOUND_MS: i64 = 500;
    const SKIP_UPPER_BOUND_MS: i64 = 2_500;

    /// is_visible must return the live qualification
    /// its read after the dwell, after the store call and after a retry delay
    pub fn new(
        post_id: impl Into<String>,
        surface: RisePostViewSurface,
        is_visible: impl Fn(&App) -> bool + 'static,
    ) -> Self {
        Self {
            post_id: post_id.into(),
            surface,
            is_visible: Rc::new(is_visible),
            scope_id: Uuid::new_v4(),
            scope_generation: 0,
            is_qualified: false,
            reported: false,
            retry_attempt: 0,
            dwell_task: None,
            visible_since_uptime: None,
            is_source_active: false,
        }
    }

    pub fn post_id(&self) -> &str {
        &self.post_id
    }

    pub fn surface(&self) -> RisePostViewSurface {
        self.surface
    }

    /// true when the view for the current scope reached the store, cleared when the post leaves the screen
    pub fn is_reported(&self) -> bool {
        self.reported
    }

    pub fn needs_visibility_update(&self, qualified: bool) -> bool {
        qualified != self.is_qualified || qualified != self.is_source_active
    }

    /// starts a new scope
    /// cancels the dwell, forgets qualification and the reported flag
    /// keeps scope_id and the skip clock
    pub fn reset_scope(&mut self, post_id: &str, surface: RisePostViewSurface, cx: &mut Context<Self>) {
        self.remove_activity_source(cx);
        self.cancel_dwell();
        self.post_id = post_id.to_owned();
        self.surface = surface;
        self.is_qualified = false;
        self.reported = false;
        self.retry_attempt = 0;
        self.scope_generation = self.scope_generation.wrapping_add(1);
    }

    /// feeds the current qualification, cheap when unchanged
    pub fn visibility_changed(&mut self, qualified: bool, cx: &mut Context<Self>) {
        let scope_id = self.scope_id;
        let is_visible = self.is_visible.clone();
        self.is_source_active = qualified;
        RiseMeaningfulActivity::shared(cx).update(cx, |activity, cx| {
            activity.set_source(scope_id, qualified, move |cx: &App| is_visible(cx), cx);
        });
        if qualified == self.is_qualified {
            return;
        }
        self.is_qualified = qualified;
        if !qualified {
            self.report_skip_if_briefly_seen(cx);
            self.cancel_dwell();
            self.reported = false;
            self.retry_attempt = 0;
            return;
        }
        self.visible_since_uptime = Some(RiseClock::monotonic());
        if self.reported {
            return;
        }

        self.scope_generation = self.scope_generation.wrapping_add(1);
        let generation = self.scope_generation;
        let intent = PostInteractionsStore::shared(cx).read(cx).prepare_post_view_intent(
            &self.post_id,
            self.surface,
            scope_id,
            generation,
        );
        let Some(intent) = intent else {
            self.is_qualified = false;
            return;
        };
        self.dwell_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_nanos(Self::DWELL_NANOSECONDS)).await;
            let may_record = this.update(cx, |session, cx| session.continues_after_wait(generation, cx));
            if !matches!(may_record, Ok(true)) {
                return;
            }
            let record = this.update(cx, |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.record_post_view(intent, cx))
            });
            let Ok(record) = record else {
                return;
            };
            let handled = record.await;
            let retry_delay = this.update(cx, |session, cx| session.finish_record(generation, handled, cx));
            let Ok(Some(retry_delay)) = retry_delay else {
                return;
            };
            cx.background_executor().timer(retry_delay).await;
            let _ = this.update(cx, |session, cx| {
                if !session.continues_after_wait(generation, cx) {
                    return;
                }
                session.dwell_task = None;
                session.is_qualified = false;
                session.evaluate(cx);
            });
        }));
    }

    /// guards that follow every await
    /// a dropped task never gets here
    fn continues_after_wait(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        if self.scope_generation != generation {
            return false;
        }
        if !self.is_qualified || !(self.is_visible)(&**cx) {
            self.dwell_task = None;
            self.is_qualified = false;
            return false;
        }
        true
    }

    fn finish_record(&mut self, generation: u64, handled: bool, cx: &mut Context<Self>) -> Option<Duration> {
        if !self.continues_after_wait(generation, cx) {
            return None;
        }
        if handled {
            self.reported = true;
            self.retry_attempt = 0;
            self.dwell_task = None;
            return None;
        }
        let last = Self::RETRY_DELAYS_NANOSECONDS.len() - 1;
        let delay = Self::RETRY_DELAYS_NANOSECONDS[self.retry_attempt.min(last)];
        self.retry_attempt = (self.retry_attempt + 1).min(last);
        Some(Duration::from_nanos(delay))
    }

    pub fn evaluate(&mut self, cx: &mut Context<Self>) {
        let qualified = (self.is_visible)(&**cx);
        self.visibility_changed(qualified, cx);
    }

    pub fn suspend(&mut self, cx: &mut Context<Self>) {
        self.remove_activity_source(cx);
        self.cancel_dwell();
        self.is_qualified = false;
    }

    /// silent teardown, no skip is reported
    /// reusable afterwards
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.remove_activity_source(cx);
        self.visible_since_uptime = None;
        self.cancel_dwell();
        self.is_qualified = false;
        self.reported = false;
        self.retry_attempt = 0;
    }

    /// leaves the screen and reports a skip when due, then stops
    pub fn end(&mut self, cx: &mut Context<Self>) {
        self.visibility_changed(false, cx);
        self.stop(cx);
    }

    fn report_skip_if_briefly_seen(&mut self, cx: &mut Context<Self>) {
        let Some(since) = self.visible_since_uptime.take() else {
            return;
        };
        let visible_ms = RiseClock::monotonic().saturating_sub(since).as_millis() as i64;
        if !(Self::SKIP_LOWER_BOUND_MS..Self::SKIP_UPPER_BOUND_MS).contains(&visible_ms) {
            return;
        }
        let Some(analytics_surface) = EngagementSurface::from_slug(self.surface.wire()) else {
            return;
        };
        let post_id = self.post_id.clone();
        EngagementInteractionsStore::shared(cx).update(cx, |engagement, cx| {
            engagement.skip(&post_id, None, analytics_surface, -1, visible_ms, cx);
        });
    }

    fn remove_activity_source(&mut self, cx: &mut Context<Self>) {
        let scope_id = self.scope_id;
        self.is_source_active = false;
        RiseMeaningfulActivity::shared(cx).update(cx, |activity, cx| activity.remove_source(scope_id, cx));
    }

    fn cancel_dwell(&mut self) {
        self.dwell_task = None;
    }
}
