use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Bounds, ClickEvent, Context, Corners, CursorStyle, ElementId, Hitbox, HitboxBehavior, InteractiveElement,
    IntoElement, ParentElement, Pixels, Point, Priority, RenderImage, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Task, Window, bounds, canvas, div, point, px,
};
use rise_core::foundation::ns_cache::NSCache;

use crate::core::ui::rich_text::post_rich_text_render::PostRichTextCalibration;
use crate::core::ui::rich_text::post_text_layout::{PostTextBitmap, PostTextLayout};
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListCachePurge, PostRowActionRouting, PostRowCaptionLayout, PostRowMainContext, PostRowTextBlockLayout,
    PostTextLayouting, PostTextToken, PostTextTokenKind,
};

const TOKEN_SLOP: f32 = 4.0;
const TOGGLE_SLOP: f32 = 10.0;
const TOKEN_TAP_FAILURE_WAIT: Duration = Duration::from_millis(300);

type PostRowCaptionDoubleTap = Rc<dyn Fn(&mut Window, &mut App)>;

/// caption as an element, content_frame is caption.frame in this views coords
/// the parent gives this view its frame
#[derive(IntoElement)]
pub struct PostRowCaptionView {
    post_id: String,
    caption: PostRowCaptionLayout,
    content_frame: Bounds<Pixels>,
    scale: f32,
    router: Weak<dyn PostRowActionRouting>,
    on_double_tap: Option<PostRowCaptionDoubleTap>,
}

impl PostRowCaptionView {
    pub fn new(
        post_id: String,
        caption: PostRowCaptionLayout,
        content_frame: Bounds<Pixels>,
        context: &PostRowMainContext,
    ) -> Self {
        Self {
            post_id,
            caption,
            content_frame,
            scale: context.spec.display_scale,
            router: Rc::downgrade(&context.router),
            on_double_tap: None,
        }
    }

    pub fn on_double_tap(mut self, on_double_tap: PostRowCaptionDoubleTap) -> Self {
        self.on_double_tap = Some(on_double_tap);
        self
    }

    fn flattened_blocks(caption: &PostRowCaptionLayout) -> Vec<PostRowTextBlockLayout> {
        let mut flattened = Vec::new();
        if let Some(title) = caption.title.as_ref() {
            flattened.push(title.clone());
        }
        for paragraph in caption.paragraphs.iter() {
            flattened.push(paragraph.body.clone());
            if let Some(subtitle) = paragraph.link_subtitle.as_ref() {
                flattened.push(subtitle.clone());
            }
        }
        if let Some(toggle) = caption.toggle.as_ref() {
            flattened.push(toggle.clone());
        }
        if let Some(hashtags) = caption.hashtags.as_ref() {
            flattened.push(hashtags.clone());
        }
        flattened
    }

    /// mentions open the public tag url
    /// hashtags open search, web links open the browser
    fn open(
        router: &Weak<dyn PostRowActionRouting>,
        post_id: &str,
        kind: &PostTextTokenKind,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(router) = router.upgrade() else {
            return;
        };
        match kind {
            PostTextTokenKind::Mention { url, .. } => router.open_mention(url, post_id, window, cx),
            PostTextTokenKind::Hashtag(tag) => router.open_hashtag(tag, post_id, cx),
            PostTextTokenKind::Url(url) => router.open_link(url, window, cx),
        }
    }

    fn tappable_token(blocks: &[PostRowTextBlockLayout], location: Point<Pixels>) -> Option<PostTextToken> {
        for block in blocks {
            let local = location - block.frame.origin;
            for token in block.text.tokens().iter().filter(|token| is_tappable(token)) {
                if token.rects.iter().any(|rect| rect.dilate(px(TOKEN_SLOP)).contains(&local)) {
                    return Some(token.clone());
                }
            }
        }
        None
    }

    /// the more or less line in content coords, none without one
    fn toggle_frame(caption: &PostRowCaptionLayout) -> Option<Bounds<Pixels>> {
        caption.toggle.as_ref().map(|toggle| bounds(toggle.frame.origin, toggle.text.size()))
    }

    fn is_on_toggle(toggle_frame: Option<Bounds<Pixels>>, location: Point<Pixels>) -> bool {
        toggle_frame.is_some_and(|frame| frame.dilate(px(TOGGLE_SLOP)).contains(&location))
    }

    fn accessibility_label(blocks: &[PostRowTextBlockLayout]) -> Option<SharedString> {
        let parts: Vec<&str> = blocks.iter().map(|block| block.text.plain_text()).filter(|text| !text.is_empty()).collect();
        if parts.is_empty() { None } else { Some(parts.join("\n").into()) }
    }
}

fn is_tappable(token: &PostTextToken) -> bool {
    match &token.kind {
        PostTextTokenKind::Url(url) => matches!(url.scheme(), Some("https" | "http")),
        PostTextTokenKind::Mention { .. } | PostTextTokenKind::Hashtag(_) => true,
    }
}

impl RenderOnce for PostRowCaptionView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { post_id, caption, content_frame, scale, router, on_double_tap } = self;
        let blocks = Rc::new(Self::flattened_blocks(&caption));
        let toggle_frame = Self::toggle_frame(&caption);
        let state_id = ElementId::Name(SharedString::from(format!("post-row-caption-{post_id}")));
        let state = window.use_keyed_state(state_id.clone(), cx, |_, cx| PostRowCaptionViewState::new(cx));
        state.update(cx, |state, cx| state.configure(&blocks, scale, cx));
        let (token_tap_generation, content_origin) = {
            let state = state.read(cx);
            (state.token_tap_generation.clone(), state.content_origin.clone())
        };
        let label = Self::accessibility_label(&blocks);

        let content_id = ElementId::Name(SharedString::from(format!("post-row-caption-content-{post_id}")));
        let prepaint_blocks = blocks.clone();
        let prepaint_origin = content_origin.clone();
        let paint_blocks = blocks.clone();
        let paint_state = state.clone();
        let content = div()
            .id(content_id)
            .absolute()
            .left(content_frame.origin.x)
            .top(content_frame.origin.y)
            .w(content_frame.size.width)
            .h(content_frame.size.height)
            .when_some(label, |content, label| content.aria_label(label))
            .child(
                canvas(
                    move |canvas_bounds, window, _| {
                        prepaint_origin.set(canvas_bounds.origin);
                        let mut hitboxes = token_hitboxes(&prepaint_blocks, canvas_bounds.origin, window);
                        if let Some(frame) = toggle_frame {
                            let toggle_bounds = bounds(canvas_bounds.origin + frame.origin, frame.size).dilate(px(TOGGLE_SLOP));
                            hitboxes.push(window.insert_hitbox(toggle_bounds, HitboxBehavior::Normal));
                        }
                        hitboxes
                    },
                    move |canvas_bounds, hitboxes: Vec<Hitbox>, window, cx| {
                        for hitbox in hitboxes.iter() {
                            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
                        }
                        for image in PostRowTextBitmapCache::take_retired() {
                            let _ = window.drop_image(image);
                        }
                        paint_state.update(cx, |state, cx| {
                            for (block, view) in paint_blocks.iter().zip(state.block_views.iter()) {
                                view.draw(canvas_bounds.origin + block.frame.origin, window, cx);
                            }
                        });
                    },
                )
                .size_full(),
            );

        let click_blocks = blocks;
        div()
            .id(state_id)
            .relative()
            .size_full()
            .on_click(move |event: &ClickEvent, window: &mut Window, cx: &mut App| match event.click_count() {
                1 => {
                    let location = event.position() - content_origin.get();
                    let action = if Self::is_on_toggle(toggle_frame, location) {
                        PostRowCaptionTap::Toggle
                    } else if let Some(token) = Self::tappable_token(&click_blocks, location) {
                        PostRowCaptionTap::Token(token)
                    } else {
                        return;
                    };
                    let generation = token_tap_generation.get().wrapping_add(1);
                    token_tap_generation.set(generation);
                    let current_generation = token_tap_generation.clone();
                    let router = router.clone();
                    let post_id = post_id.clone();
                    let wait = cx.background_executor().timer(TOKEN_TAP_FAILURE_WAIT);
                    window
                        .spawn(cx, async move |cx| {
                            wait.await;
                            if current_generation.get() != generation {
                                return;
                            }
                            let _ = cx.update(|window, cx| match &action {
                                PostRowCaptionTap::Toggle => {
                                    if let Some(router) = router.upgrade() {
                                        router.toggle_caption_expansion(&post_id, cx);
                                    }
                                }
                                PostRowCaptionTap::Token(token) => Self::open(&router, &post_id, &token.kind, window, cx),
                            });
                        })
                        .detach();
                }
                2 => {
                    token_tap_generation.set(token_tap_generation.get().wrapping_add(1));
                    if let Some(on_double_tap) = on_double_tap.as_ref() {
                        on_double_tap(window, cx);
                    }
                }
                _ => {}
            })
            .child(content)
    }
}

/// what a single click lands on, both wait out the double click
enum PostRowCaptionTap {
    Toggle,
    Token(PostTextToken),
}

fn token_hitboxes(blocks: &[PostRowTextBlockLayout], origin: Point<Pixels>, window: &mut Window) -> Vec<Hitbox> {
    let mut hitboxes = Vec::new();
    for block in blocks {
        let block_origin = origin + block.frame.origin;
        for token in block.text.tokens().iter().filter(|token| is_tappable(token)) {
            for rect in token.rects.iter() {
                let token_bounds = bounds(block_origin + rect.origin, rect.size).dilate(px(TOKEN_SLOP));
                hitboxes.push(window.insert_hitbox(token_bounds, HitboxBehavior::Normal));
            }
        }
    }
    hitboxes
}

struct PostRowCaptionViewState {
    block_views: Vec<PostRowTextBlockView>,
    token_tap_generation: Rc<Cell<u64>>,
    content_origin: Rc<Cell<Point<Pixels>>>,
}

impl PostRowCaptionViewState {
    fn new(cx: &mut App) -> Self {
        PostRowTextBitmapCache::register_purge_if_needed(cx);
        Self {
            block_views: Vec::new(),
            token_tap_generation: Rc::new(Cell::new(0)),
            content_origin: Rc::new(Cell::new(point(px(0.0), px(0.0)))),
        }
    }

    fn configure(&mut self, blocks: &[PostRowTextBlockLayout], scale: f32, cx: &mut Context<Self>) {
        while self.block_views.len() < blocks.len() {
            self.block_views.push(PostRowTextBlockView::new());
        }
        for (index, view) in self.block_views.iter_mut().enumerate() {
            match blocks.get(index) {
                Some(block) => view.set_text_layout(&block.text, scale, cx),
                None => view.clear(),
            }
        }
    }
}

/// one prepared text layout, a cached bitmap if there is one
/// otherwise lines paint directly while a bitmap renders off main for the next mount
pub struct PostRowTextBlockView {
    text_layout: Option<Arc<dyn PostTextLayouting>>,
    bitmap: Option<Arc<PostRowTextBitmapCacheValue>>,
    bitmap_task: Option<Task<()>>,
    bitmap_is_cancelled: Option<Arc<AtomicBool>>,
}

impl PostRowTextBlockView {
    pub fn new() -> Self {
        Self { text_layout: None, bitmap: None, bitmap_task: None, bitmap_is_cancelled: None }
    }

    pub fn set_text_layout(&mut self, layout: &Arc<dyn PostTextLayouting>, scale: f32, cx: &App) {
        if let Some(current) = self.text_layout.as_ref()
            && Arc::ptr_eq(current, layout)
        {
            return;
        }
        self.cancel_bitmap_task();
        self.bitmap = None;
        self.text_layout = Some(layout.clone());
        let Ok(concrete) = layout.clone().as_any().downcast::<PostTextLayout>() else {
            return;
        };
        if let Some(bitmap) = PostRowTextBitmapCache::shared().bitmap(&concrete, scale) {
            self.bitmap = Some(bitmap);
            return;
        }
        let is_cancelled = Arc::new(AtomicBool::new(false));
        let task_is_cancelled = is_cancelled.clone();
        self.bitmap_is_cancelled = Some(is_cancelled);
        self.bitmap_task = Some(cx.background_executor().spawn_with_priority(Priority::Low, async move {
            let is_cancelled = || task_is_cancelled.load(Ordering::Relaxed);
            let Some(bitmap) = PostRowTextBitmapCache::render(&concrete, scale, &is_cancelled) else {
                return;
            };
            PostRowTextBitmapCache::shared().insert(bitmap, &concrete, scale);
        }));
    }

    pub fn clear(&mut self) {
        self.cancel_bitmap_task();
        self.text_layout = None;
        self.bitmap = None;
    }

    pub fn draw(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
        if let Some(value) = self.bitmap.as_ref() {
            let image_bounds = bounds(origin, value.bitmap.size);
            let painted = window.paint_image(image_bounds, image_bounds, Corners::default(), value.bitmap.image.clone(), 0, false);
            if painted.is_ok() {
                return;
            }
        }
        if let Some(text_layout) = self.text_layout.as_ref() {
            text_layout.draw(origin, window, cx);
        }
    }

    fn cancel_bitmap_task(&mut self) {
        if let Some(is_cancelled) = self.bitmap_is_cancelled.take() {
            is_cancelled.store(true, Ordering::Relaxed);
        }
        self.bitmap_task = None;
    }
}

impl Default for PostRowTextBlockView {
    fn default() -> Self {
        Self::new()
    }
}

/// holds the layout strongly so an identity is never reused while cached
#[derive(Clone)]
struct PostRowTextBitmapCacheKey {
    layout: Arc<PostTextLayout>,
    scale: f32,
}

impl PartialEq for PostRowTextBitmapCacheKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.layout, &other.layout) && self.scale.to_bits() == other.scale.to_bits()
    }
}

impl Eq for PostRowTextBitmapCacheKey {}

impl Hash for PostRowTextBitmapCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.layout) as usize).hash(state);
        self.scale.to_bits().hash(state);
    }
}

/// cached bitmap, when the last holder lets go its atlas tile is freed at the next paint
pub struct PostRowTextBitmapCacheValue {
    pub bitmap: PostTextBitmap,
}

impl Drop for PostRowTextBitmapCacheValue {
    fn drop(&mut self) {
        if let Ok(mut retired) = RETIRED_IMAGES.lock() {
            retired.push(self.bitmap.image.clone());
        }
    }
}

static RETIRED_IMAGES: LazyLock<Mutex<Vec<Arc<RenderImage>>>> = LazyLock::new(|| Mutex::new(Vec::new()));
static SHARED_BITMAP_CACHE: LazyLock<PostRowTextBitmapCache> = LazyLock::new(PostRowTextBitmapCache::new);
static IS_PURGE_REGISTERED: AtomicBool = AtomicBool::new(false);

/// caption block bitmaps keyed by layout identity and scale
/// bounded by count and cost, purged with the list caches
pub struct PostRowTextBitmapCache {
    cache: NSCache<PostRowTextBitmapCacheKey, Arc<PostRowTextBitmapCacheValue>>,
}

impl PostRowTextBitmapCache {
    fn new() -> Self {
        let cache = NSCache::new();
        cache.set_count_limit(128);
        cache.set_total_cost_limit(24 * 1_024 * 1_024);
        Self { cache }
    }

    pub fn shared() -> &'static PostRowTextBitmapCache {
        &SHARED_BITMAP_CACHE
    }

    pub fn bitmap(&self, layout: &Arc<PostTextLayout>, scale: f32) -> Option<Arc<PostRowTextBitmapCacheValue>> {
        self.cache.object(&PostRowTextBitmapCacheKey { layout: layout.clone(), scale })
    }

    pub fn insert(&self, bitmap: PostTextBitmap, layout: &Arc<PostTextLayout>, scale: f32) {
        let cost = bitmap.image.as_bytes(0).map_or(1, |bytes| bytes.len().max(1));
        self.cache.set_object_cost(
            Arc::new(PostRowTextBitmapCacheValue { bitmap }),
            PostRowTextBitmapCacheKey { layout: layout.clone(), scale },
            cost,
        );
    }

    pub fn remove_all(&self) {
        self.cache.remove_all_objects();
    }

    pub fn render(layout: &PostTextLayout, scale: f32, is_cancelled: &dyn Fn() -> bool) -> Option<PostTextBitmap> {
        layout.make_bitmap(Some(scale), PostRichTextCalibration::BITMAP_PIXEL_BUDGET, is_cancelled)
    }

    pub fn register_purge_if_needed(cx: &mut App) {
        if IS_PURGE_REGISTERED.swap(true, Ordering::Relaxed) {
            return;
        }
        let _ = PostListCachePurge::register(|_| PostRowTextBitmapCache::shared().remove_all(), cx);
    }

    fn take_retired() -> Vec<Arc<RenderImage>> {
        RETIRED_IMAGES.lock().map(|mut retired| std::mem::take(&mut *retired)).unwrap_or_default()
    }
}
