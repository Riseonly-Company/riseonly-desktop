use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use gpui::{App, BackgroundExecutor, Priority, Size, Task, px, size};
use rise_i18n::language_store::Locale;
use rise_platform::process_info::active_processor_count;
use rise_theme::{ThemePalette, ThemeStore};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImageAsyncWorkLimiter, CleverImageWorkLane};
use crate::core::ui::rich_text::post_rich_text_render::PostRichTextCalibration;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostCaptionExpansion, PostRowContentVersionTable, PostRowLayoutInputs, PostRowLayoutSpec, PostRowPalette,
    PostRowRoute, PostRowSeed, PreparedPostRow, PreparedPostRowKey,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::{
    PostRowLayoutCalculator, PostRowTextMetrics,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_media_aspect_resolver::{
    PostRowAspectMemo, PostRowAspectOutcome, PostRowAspectProbe, PostRowAspectProbeRequest, PostRowMediaAspectResolver,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_premium_badge_resolver::PostRowPremiumBadgeResolver;
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_prepared_cache::{
    PostRowLayoutCache, PostRowModelCaches, PostRowPreparedCache,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_seed_extractor::PostRowSeedExtractor;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// environment all rows of one prepare pass share, built on main
#[derive(Clone, PartialEq)]
pub struct PostRowPrepareEnvironment {
    pub spec: PostRowLayoutSpec,
    pub palette: PostRowPalette,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

thread_local! {
    static PALETTE_MEMO: RefCell<Option<(ThemePalette, PostRowPalette)>> = const { RefCell::new(None) };
}

impl PostRowPrepareEnvironment {
    pub fn new(
        spec: PostRowLayoutSpec,
        palette: PostRowPalette,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) -> Self {
        Self { spec, palette, account_user_id, account_epoch }
    }

    /// controller and grid tap warm up must both build it here or warm rows miss the cache
    pub fn current(width: f32, display_scale: Option<f32>, cx: &mut App) -> PostRowPrepareEnvironment {
        let scale = display_scale.filter(|scale| scale.is_finite() && *scale > 0.0).unwrap_or_else(|| main_display_scale(cx));
        let theme = ThemeStore::shared(cx);
        let source = theme.read(cx).current_palette().clone();
        let text_delta = theme.read(cx).text_delta();
        let palette = PALETTE_MEMO.with(|memo| {
            let mut memo = memo.borrow_mut();
            if let Some((memo_source, memo_palette)) = memo.as_ref()
                && *memo_source == source
            {
                return memo_palette.clone();
            }
            let palette = PostRowPalette::new(&source);
            *memo = Some((source, palette.clone()));
            palette
        });
        let spec = PostRowLayoutSpec::new(
            PostRowTextMetrics::pixel_round(if width.is_finite() { width.max(1.0) } else { 1.0 }, scale),
            main_screen_size(width),
            scale,
            text_delta,
            Locale::app_locale().identifier().to_owned(),
            palette.theme_id.clone(),
            false,
        );
        PostRowPrepareEnvironment::new(
            spec,
            palette,
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
        )
    }

    pub fn key(&self, input: &PostRowPrepareInput) -> PreparedPostRowKey {
        PreparedPostRowKey {
            presentation_id: input.seed.presentation_id.clone(),
            content_version: input.content_version,
            spec: self.spec.clone(),
            account_user_id: self.account_user_id.clone(),
            account_epoch: self.account_epoch.clone(),
        }
    }
}

fn main_screen_size(width: f32) -> Size<gpui::Pixels> {
    let width = if width.is_finite() { width.max(1.0) } else { 1.0 };
    size(px(width), PostMediaBox::viewport().height)
}

fn main_display_scale(cx: &mut App) -> f32 {
    cx.active_window()
        .and_then(|handle| handle.update(cx, |_, window, _| window.scale_factor()).ok())
        .unwrap_or(PostRichTextCalibration::FALLBACK_DISPLAY_SCALE)
}

/// one post ready to prepare off main
/// with snapshot, seed
/// route and content version
#[derive(Clone)]
pub struct PostRowPrepareInput {
    pub post: GetPostFeedResponse,
    pub seed: PostRowSeed,
    pub route: PostRowRoute,
    pub content_version: i64,
}

impl PostRowPrepareInput {
    pub fn presentation_id(&self) -> &str {
        &self.seed.presentation_id
    }

    /// reads main thread stores for the seed
    pub fn make(post: &GetPostFeedResponse, cx: &mut App) -> PostRowPrepareInput {
        let mut seed = PostRowSeedExtractor::seed(post, cx);
        seed.author.premium_badge = PostRowPremiumBadgeResolver::resolve(&seed.author, cx);
        seed.is_caption_expanded = PostCaptionExpansion::is_expanded(&seed.presentation_id, cx);
        let route = PostRowSeedExtractor::route(&seed);
        let content_version = PostRowContentVersionTable::shared().version(&seed);
        PostRowPrepareInput { post: post.clone(), seed, route, content_version }
    }
}

/// task priority and image loader lane of a prepare pass
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostRowPreparePriority {
    Visible,
    Warm,
}

impl PostRowPreparePriority {
    pub fn task_priority(self) -> Priority {
        match self {
            PostRowPreparePriority::Visible => Priority::High,
            PostRowPreparePriority::Warm => Priority::Low,
        }
    }

    pub fn lane(self) -> CleverImageWorkLane {
        match self {
            PostRowPreparePriority::Visible => CleverImageWorkLane::Visible,
            PostRowPreparePriority::Warm => CleverImageWorkLane::Speculative,
        }
    }
}

/// measured hosted row height for a key
/// none if not measured yet, called off main
pub type PostRowHostedHeightLookup = Arc<dyn Fn(&PreparedPostRowKey) -> Option<f32> + Send + Sync>;

/// result of one prepare call
#[derive(Clone)]
pub struct PostRowPrepareBatch {
    pub generation: u64,
    pub environment: PostRowPrepareEnvironment,
    pub rows: Vec<Arc<PreparedPostRow>>,
    pub is_complete: bool,
    pub cache_hits: usize,
    pub awaiting_aspect_ids: HashSet<String>,
}

pub struct PostRowPreparationOutcome {
    pub row: Arc<PreparedPostRow>,
    pub was_cached: bool,
    pub probe: Option<PostRowAspectProbeRequest>,
}

pub struct PostRowPreparationRun {
    pub rows: Vec<Arc<PreparedPostRow>>,
    pub is_complete: bool,
    pub cache_hits: usize,
    pub probes: HashMap<PostRowAspectProbeRequest, HashSet<String>>,
}

/// off main row preparing, cache lookup
/// aspect chain, layout and hosted estimate
pub enum PostRowPreparation {}

impl PostRowPreparation {
    pub fn maximum_concurrent_rows() -> usize {
        active_processor_count().saturating_sub(1).clamp(1, 3)
    }

    /// limits text work across list sessions and warm ups, one permit stays with the visible lane
    pub fn limiter() -> &'static CleverImageAsyncWorkLimiter {
        static LIMITER: OnceLock<CleverImageAsyncWorkLimiter> = OnceLock::new();
        LIMITER.get_or_init(|| CleverImageAsyncWorkLimiter::new(Self::maximum_concurrent_rows() as i64))
    }

    /// one row, synchronous
    /// any thread
    pub fn prepare(
        input: &PostRowPrepareInput,
        environment: &PostRowPrepareEnvironment,
        hosted_height: Option<&PostRowHostedHeightLookup>,
    ) -> PostRowPreparationOutcome {
        let key = environment.key(input);
        if let Some(cached) = PostRowPreparedCache::shared().row(&key) {
            return PostRowPreparationOutcome { row: Self::adopting(&input.post, &cached), was_cached: true, probe: None };
        }

        let resolution = PostRowMediaAspectResolver::resolve(
            &input.seed,
            environment.account_user_id.as_deref(),
            environment.account_epoch.as_deref(),
        );
        match &input.route {
            PostRowRoute::Native(_) => {
                let inputs = PostRowLayoutInputs::new(
                    &input.seed,
                    input.route,
                    environment.spec.clone(),
                    &resolution.aspects,
                );
                let layout = match PostRowLayoutCache::shared().layout(&inputs) {
                    Some(hit) => hit,
                    None => {
                        let layout = Arc::new(PostRowLayoutCalculator::layout(&inputs, &environment.palette));
                        PostRowLayoutCache::shared().insert(layout.clone());
                        layout
                    }
                };
                let row = Arc::new(PreparedPostRow::new(
                    input.seed.clone(),
                    input.route,
                    input.post.clone(),
                    Some(layout),
                    None,
                    input.content_version,
                    !resolution.is_settled,
                    key,
                ));
                PostRowPreparedCache::shared().insert(row.clone());
                PostRowPreparationOutcome { row, was_cached: false, probe: resolution.probe }
            }
            PostRowRoute::HostedFallback(reason) => {
                let measured = hosted_height.and_then(|lookup| lookup(&key)).filter(|height| height.is_finite() && *height >= 1.0);
                let height = measured.unwrap_or_else(|| {
                    PostRowLayoutCalculator::estimated_hosted_height(
                        &input.seed,
                        *reason,
                        &resolution.aspects,
                        &environment.spec,
                        &environment.palette,
                    )
                });
                let row = Arc::new(PreparedPostRow::new(
                    input.seed.clone(),
                    input.route,
                    input.post.clone(),
                    None,
                    Some(height),
                    input.content_version,
                    measured.is_none(),
                    key,
                ));
                PostRowPreparationOutcome { row, was_cached: false, probe: None }
            }
        }
    }

    /// cache hit with the callers snapshot, since hosted leaves
    /// and menus render the post which may differ outside the seed
    pub fn adopting(post: &GetPostFeedResponse, row: &Arc<PreparedPostRow>) -> Arc<PreparedPostRow> {
        if row.post == *post {
            return row.clone();
        }
        Arc::new(PreparedPostRow::new(
            row.seed.clone(),
            row.route,
            post.clone(),
            row.layout.clone(),
            row.hosted_height,
            row.content_version,
            row.is_height_estimated,
            row.key.clone(),
        ))
    }

    /// bounded concurrency pass over inputs, checks cancel per row
    /// dropping a gpui task only counts at a suspension, and an uncontended limiter never suspends
    pub async fn run(
        inputs: Vec<PostRowPrepareInput>,
        environment: PostRowPrepareEnvironment,
        priority: PostRowPreparePriority,
        hosted_height: Option<PostRowHostedHeightLookup>,
        is_cancelled: Arc<AtomicBool>,
        executor: BackgroundExecutor,
    ) -> PostRowPreparationRun {
        let count = inputs.len();
        if count == 0 {
            return PostRowPreparationRun { rows: Vec::new(), is_complete: true, cache_hits: 0, probes: HashMap::new() };
        }
        let stripes = count.min(Self::maximum_concurrent_rows());
        let lane = priority.lane();
        let inputs = Arc::new(inputs);
        let environment = Arc::new(environment);
        let mut workers = Vec::with_capacity(stripes);
        for stripe in 0..stripes {
            let inputs = inputs.clone();
            let environment = environment.clone();
            let hosted_height = hosted_height.clone();
            let is_cancelled = is_cancelled.clone();
            workers.push(executor.spawn_with_priority(priority.task_priority(), async move {
                let mut produced = Vec::new();
                let mut index = stripe;
                while index < count {
                    if is_cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(permit) = Self::limiter().acquire(lane).await else {
                        break;
                    };
                    let outcome = Self::prepare(&inputs[index], &environment, hosted_height.as_ref());
                    drop(permit);
                    produced.push((index, outcome));
                    index += stripes;
                }
                produced
            }));
        }
        let mut collected: Vec<Option<PostRowPreparationOutcome>> = (0..count).map(|_| None).collect();
        for worker in workers {
            for (index, outcome) in worker.await {
                collected[index] = Some(outcome);
            }
        }

        let mut rows = Vec::with_capacity(count);
        let mut cache_hits = 0;
        let mut probes: HashMap<PostRowAspectProbeRequest, HashSet<String>> = HashMap::new();
        for outcome in collected {
            let Some(outcome) = outcome else {
                break;
            };
            if outcome.was_cached {
                cache_hits += 1;
            }
            if let Some(probe) = outcome.probe {
                probes.entry(probe).or_default().insert(outcome.row.presentation_id.clone());
            }
            rows.push(outcome.row);
        }
        let is_complete = rows.len() == count;
        PostRowPreparationRun { rows, is_complete, cache_hits, probes }
    }
}

struct PostRowPrepareCancelOnDrop(Arc<AtomicBool>);

impl Drop for PostRowPrepareCancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

type PostRowAspectsResolved = Rc<dyn Fn(&HashSet<String>, &mut App)>;

/// per session preparer over the shared caches
pub struct PostRowPreparer {
    this: Weak<PostRowPreparer>,
    pub session: u64,
    on_aspects_resolved: RefCell<Option<PostRowAspectsResolved>>,
    awaiting_ids: RefCell<HashMap<PostRowAspectProbeRequest, HashSet<String>>>,
    probe_tasks: RefCell<HashMap<PostRowAspectProbeRequest, Task<()>>>,
    is_invalidated: Cell<bool>,
}

impl PostRowPreparer {
    pub fn new(session: u64, cx: &mut App) -> Rc<Self> {
        PostRowModelCaches::register_purges_if_needed(cx);
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            session,
            on_aspects_resolved: RefCell::new(None),
            awaiting_ids: RefCell::new(HashMap::new()),
            probe_tasks: RefCell::new(HashMap::new()),
            is_invalidated: Cell::new(false),
        })
    }

    pub fn set_on_aspects_resolved(&self, on_aspects_resolved: PostRowAspectsResolved) {
        *self.on_aspects_resolved.borrow_mut() = Some(on_aspects_resolved);
    }

    pub fn input(&self, post: &GetPostFeedResponse, cx: &mut App) -> PostRowPrepareInput {
        PostRowPrepareInput::make(post, cx)
    }

    /// linear in posts and runs on main, pass only rows the next commit or warm up needs
    pub fn inputs(&self, posts: &[GetPostFeedResponse], cx: &mut App) -> Vec<PostRowPrepareInput> {
        posts.iter().map(|post| PostRowPrepareInput::make(post, cx)).collect()
    }

    /// sync cache lookup, eg row 0 warmed at the grid tap
    pub fn cached_row(
        &self,
        input: &PostRowPrepareInput,
        environment: &PostRowPrepareEnvironment,
    ) -> Option<Arc<PreparedPostRow>> {
        let cached = PostRowPreparedCache::shared().row(&environment.key(input))?;
        Some(PostRowPreparation::adopting(&input.post, &cached))
    }

    pub fn cached_row_for_post(
        &self,
        post: &GetPostFeedResponse,
        environment: &PostRowPrepareEnvironment,
        cx: &mut App,
    ) -> Option<Arc<PreparedPostRow>> {
        let input = self.input(post, cx);
        self.cached_row(&input, environment)
    }

    /// prepares inputs off main, dropping the task cancels the work between rows
    pub fn prepare(
        &self,
        inputs: Vec<PostRowPrepareInput>,
        environment: &PostRowPrepareEnvironment,
        generation: u64,
        priority: PostRowPreparePriority,
        hosted_height: Option<PostRowHostedHeightLookup>,
        cx: &mut App,
    ) -> Task<PostRowPrepareBatch> {
        if inputs.is_empty() {
            return Task::ready(PostRowPrepareBatch {
                generation,
                environment: environment.clone(),
                rows: Vec::new(),
                is_complete: true,
                cache_hits: 0,
                awaiting_aspect_ids: HashSet::new(),
            });
        }
        let this = self.this.clone();
        let environment = environment.clone();
        let executor = cx.background_executor().clone();
        let is_cancelled = Arc::new(AtomicBool::new(false));
        let cancel_on_drop = PostRowPrepareCancelOnDrop(is_cancelled.clone());
        cx.spawn(async move |cx| {
            let run =
                PostRowPreparation::run(inputs, environment.clone(), priority, hosted_height, is_cancelled, executor).await;
            drop(cancel_on_drop);
            let awaiting_aspect_ids: HashSet<String> = run.probes.values().flatten().cloned().collect();
            let probes = run.probes;
            cx.update(|cx| {
                if let Some(this) = this.upgrade() {
                    this.schedule_probes(probes, cx);
                }
            });
            PostRowPrepareBatch {
                generation,
                environment,
                rows: run.rows,
                is_complete: run.is_complete,
                cache_hits: run.cache_hits,
                awaiting_aspect_ids,
            }
        })
    }

    /// one row on main for row 0 in the first usable layout, never during render
    pub fn prepare_synchronously(
        &self,
        input: &PostRowPrepareInput,
        environment: &PostRowPrepareEnvironment,
        hosted_height: Option<PostRowHostedHeightLookup>,
        cx: &mut App,
    ) -> Arc<PreparedPostRow> {
        let outcome = PostRowPreparation::prepare(input, environment, hosted_height.as_ref());
        if let Some(probe) = outcome.probe {
            let ids = HashSet::from([input.presentation_id().to_owned()]);
            self.schedule_probes(HashMap::from([(probe, ids)]), cx);
        }
        outcome.row
    }

    /// height a row is inserted with before its exact
    /// height is known, native layout or hosted estimate
    pub fn estimated_height(input: &PostRowPrepareInput, environment: &PostRowPrepareEnvironment) -> f32 {
        PostRowPreparation::prepare(input, environment, None).row.height()
    }

    /// stops probe callbacks, shared caches stay warm for the next session
    pub fn invalidate(&self) {
        self.is_invalidated.set(true);
        *self.on_aspects_resolved.borrow_mut() = None;
        self.probe_tasks.borrow_mut().clear();
        self.awaiting_ids.borrow_mut().clear();
    }

    pub fn warm_anchor(
        post: &GetPostFeedResponse,
        width: Option<f32>,
        display_scale: Option<f32>,
        bitmap_aspect: Option<f32>,
        cx: &mut App,
    ) {
        PostRowModelCaches::register_purges_if_needed(cx);
        let width = width.unwrap_or_else(|| f32::from(PostMediaBox::viewport().width));
        let environment = PostRowPrepareEnvironment::current(width, display_scale, cx);
        let input = PostRowPrepareInput::make(post, cx);
        if let Some(bitmap_aspect) = bitmap_aspect.filter(|aspect| aspect.is_finite() && *aspect > 0.0)
            && let Some(source) = PostRowMediaAspectResolver::cover_source(&input.seed)
        {
            PostRowAspectMemo::shared().store(
                PostRowAspectOutcome::Aspect(bitmap_aspect),
                &source,
                environment.account_user_id.as_deref(),
                environment.account_epoch.as_deref(),
            );
        }
        if input.route.is_hosted_fallback() || PostRowPreparedCache::shared().row(&environment.key(&input)).is_some() {
            return;
        }
        let priority = PostRowPreparePriority::Visible.task_priority();
        cx.spawn(async move |cx| {
            let first = cx.background_executor().spawn_with_priority(priority, async move {
                let outcome = PostRowPreparation::prepare(&input, &environment, None);
                (outcome.probe, input, environment)
            });
            let (probe, input, environment) = first.await;
            let Some(probe) = probe else {
                return;
            };
            let _ = PostRowAspectProbe::shared().resolve(probe, cx).await;
            let second = cx.background_executor().spawn_with_priority(priority, async move {
                let _ = PostRowPreparation::prepare(&input, &environment, None);
            });
            second.await;
        })
        .detach();
    }

    fn schedule_probes(&self, probes: HashMap<PostRowAspectProbeRequest, HashSet<String>>, cx: &mut App) {
        if self.is_invalidated.get() {
            return;
        }
        for (request, ids) in probes {
            self.awaiting_ids.borrow_mut().entry(request.clone()).or_default().extend(ids);
            if self.probe_tasks.borrow().contains_key(&request) {
                continue;
            }
            let this = self.this.clone();
            let probe = request.clone();
            let finished = request.clone();
            let task = cx.spawn(async move |cx| {
                let _ = PostRowAspectProbe::shared().resolve(probe, cx).await;
                cx.update(|cx| {
                    if let Some(this) = this.upgrade() {
                        this.probe_did_finish(&finished, cx);
                    }
                });
            });
            self.probe_tasks.borrow_mut().insert(request, task);
        }
    }

    fn probe_did_finish(&self, request: &PostRowAspectProbeRequest, cx: &mut App) {
        if let Some(task) = self.probe_tasks.borrow_mut().remove(request) {
            task.detach();
        }
        if self.is_invalidated.get() {
            return;
        }
        let Some(ids) = self.awaiting_ids.borrow_mut().remove(request).filter(|ids| !ids.is_empty()) else {
            return;
        };
        let on_aspects_resolved = self.on_aspects_resolved.borrow().clone();
        if let Some(on_aspects_resolved) = on_aspects_resolved {
            on_aspects_resolved(&ids, cx);
        }
    }
}
