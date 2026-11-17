use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, FontWeight, ScrollHandle, SharedString, Subscription, Window, black, point, white};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::modules::user::shared::rise_blocked_content::RiseBlockedContent;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::ui::clever_image_disk_cache::CleverImageWorkLane;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::story::components::story_avatar_ring::StoryAvatarRing;
use crate::modules::story::components::story_composer_host::StoryFeedComposerPresenter;
use crate::modules::story::components::story_player::story_media_view::StoryMediaPrefetcher;
use crate::modules::story::stores::story::story_actions::story_actions::{StoryActionDoneHandler, StoryActionsStore};
use crate::modules::story::stores::story::story_actions::story_types::StoryPlaybackPolicy;
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    GetStoriesFeedGatewayDTO, StoryDTO, StoryGroupDTO,
};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

#[derive(Clone)]
struct Item {
    id: String,
    source_index: usize,
    group: Rc<StoryGroupDTO>,
}

/// one read of the feed cache per pass,
/// a scroll tick redraws this row so decode once instead of four times
struct StoriesTrayFeed {
    groups: Vec<Rc<StoryGroupDTO>>,
    is_loaded: bool,
    has_more: bool,
    next_cursor: Option<String>,
}

/// state and stores alive as long as the tray is drawn
struct StoriesTrayState {
    auth: Entity<AuthServicesStore>,
    actions: Entity<StoryActionsStore>,
    service: Entity<StoryServicesStore>,
    empty_continuation_attempts: i64,
    last_empty_continuation_cursor: Option<String>,
    scroll: ScrollHandle,
    returning_to_start: Option<Animated<f32>>,
    is_hovered: bool,
    paging: Option<Animated<f32>>,
    _observations: Vec<Subscription>,
}

impl StoriesTrayState {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let actions = StoryActionsStore::shared(cx);
        let service = StoryServicesStore::shared(cx);
        let observations = vec![
            cx.observe(&auth, |_, _, cx| cx.notify()),
            cx.observe(&actions, |_, _, cx| cx.notify()),
            cx.observe(&service, |_, _, cx| cx.notify()),
        ];
        Self {
            auth,
            actions,
            service,
            empty_continuation_attempts: 0,
            last_empty_continuation_cursor: None,
            scroll: ScrollHandle::new(),
            returning_to_start: None,
            is_hovered: false,
            paging: None,
            _observations: observations,
        }
    }
}

/// horizontal row of story groups above the feed, the viewer own bubble then one per author
#[derive(IntoElement)]
pub struct StoriesTray {
    id: ElementId,
}

impl StoriesTray {
    const ITEM_SPACING: f32 = 14.0;
    const HORIZONTAL_PADDING: f32 = 14.0;
    const VERTICAL_PADDING: f32 = 10.0;
    const OWN_BUTTON_WIDTH: f32 = 80.0;
    const ITEM_WIDTH: f32 = 74.0;
    const ROW_HEIGHT: f32 = 102.0;
    const LAZY_OVERSCAN: usize = 2;
    const SKELETON_COUNT: usize = 4;
    const PAGINATION_THRESHOLD: usize = 4;
    const MAX_EMPTY_CONTINUATION_ATTEMPTS: i64 = 3;
    const PAGE_ARROW_SIZE: f32 = 28.0;
    const PAGE_ARROW_INSET: f32 = 6.0;
    const PAGE_STEP_FRACTION: f32 = 0.75;

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn keyed_id(&self, prefix: &str, key: &str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), format!("{prefix}-{key}").into())
    }

    fn feed(state: &Entity<StoriesTrayState>, cx: &mut App) -> StoriesTrayFeed {
        let (service, actions) = {
            let read = state.read(cx);
            (read.service.clone(), read.actions.clone())
        };
        let gateway = service.update(cx, |service, cx| {
            service.decode_gateway::<GetStoriesFeedGatewayDTO>(&StoryServicesStore::feed_cache_key(), cx)
        });
        match gateway {
            Some(gateway) => StoriesTrayFeed {
                groups: gateway.tray.into_iter().map(Rc::new).collect(),
                is_loaded: true,
                has_more: gateway.has_more,
                next_cursor: gateway.relative_id,
            },
            None => StoriesTrayFeed {
                groups: actions
                    .read(cx)
                    .get_stories_feed_sai
                    .data()
                    .map(|data| data.tray.into_iter().map(Rc::new).collect())
                    .unwrap_or_default(),
                is_loaded: false,
                has_more: false,
                next_cursor: None,
            },
        }
    }

    fn items(groups: &[Rc<StoryGroupDTO>], own_user_id: &str, cx: &App) -> Vec<Item> {
        let mut seen: HashSet<String> = HashSet::new();
        groups
            .iter()
            .enumerate()
            .filter_map(|(index, group)| {
                let stable = group
                    .stories
                    .first()
                    .map(|story| trimming_whitespaces_and_newlines(&story.author_id).to_owned())
                    .unwrap_or_default();
                let nested = group
                    .author
                    .as_ref()
                    .map(|author| trimming_whitespaces_and_newlines(&author.id).to_owned())
                    .unwrap_or_default();
                let author_id = if stable.is_empty() { nested } else { stable };
                if author_id.is_empty()
                    || author_id == own_user_id
                    || RiseBlockedContent::hides(Some(author_id.as_str()), cx)
                    || group.stories.is_empty()
                    || !seen.insert(author_id.clone())
                {
                    return None;
                }
                Some(Item { id: author_id, source_index: index, group: group.clone() })
            })
            .collect()
    }

    fn prefetched_stories(items: &[Item], service: &StoryServicesStore, cx: &App) -> Vec<StoryDTO> {
        let count = usize::try_from(StoryPlaybackPolicy::PREFETCH_COUNT).unwrap_or(0);
        let mut result: Vec<StoryDTO> = Vec::new();
        let mut story_ids: HashSet<i64> = HashSet::new();

        for item in items.iter().take(count) {
            let stories = &item.group.stories;
            let Some(story) = stories.iter().find(|story| !service.is_story_viewed(story, cx)).or(stories.first())
            else {
                continue;
            };
            if story_ids.insert(story.id) {
                result.push(story.clone());
            }
        }

        if result.len() < count {
            for story in items.iter().flat_map(|item| item.group.stories.iter()) {
                if !story_ids.insert(story.id) {
                    continue;
                }
                result.push(story.clone());
                if result.len() == count {
                    break;
                }
            }
        }

        result
    }

    fn awaits_first_feed(status: SaiWsSliceStatus, is_loaded: bool) -> bool {
        match status {
            SaiWsSliceStatus::Pending | SaiWsSliceStatus::Idle => !is_loaded,
            SaiWsSliceStatus::Fulfilled | SaiWsSliceStatus::Rejected => false,
        }
    }

    fn empty_continuation_cursor(items: &[Item], feed: &StoriesTrayFeed) -> Option<String> {
        if !items.is_empty() || !feed.has_more {
            return None;
        }
        feed.next_cursor.as_deref().map(|cursor| trimming_whitespaces_and_newlines(cursor).to_owned())
    }

    fn lazy_range(item_count: usize, scrolled: f32, viewport: f32, leading: f32) -> Range<usize> {
        let stride = Self::ITEM_WIDTH + Self::ITEM_SPACING;
        let first = ((scrolled - leading) / stride).floor().max(0.0) as usize;
        let last = ((scrolled + viewport - leading) / stride).ceil().max(0.0) as usize;
        let end = last.saturating_add(Self::LAZY_OVERSCAN).min(item_count);
        first.saturating_sub(Self::LAZY_OVERSCAN).min(end)..end
    }

    /// stands in for count rows not drawn, the stack own spacing adds the last gap
    fn lazy_placeholder(count: usize) -> Option<Div> {
        (count > 0).then(|| {
            let width = count as f32 * (Self::ITEM_WIDTH + Self::ITEM_SPACING) - Self::ITEM_SPACING;
            div().flex_none().w(px(width)).h(px(1.0))
        })
    }

    fn own_story_flags(user_id: &str, cx: &mut App) -> (bool, bool) {
        let store = StoryServicesStore::shared(cx);
        let service = store.read(cx);
        (
            service.has_active_stories(Some(user_id), cx),
            service.is_publishing || service.is_shared_story_background_activity() || service.publish_ring_is_settling,
        )
    }

    fn open_own_story(user_id: &str, drawn_has_story: bool, drawn_is_publishing: bool, cx: &mut App) {
        let (has_story, is_publishing) = Self::own_story_flags(user_id, cx);
        tracing::warn!(target: "riseonly::X1017", drawn_has_story, drawn_is_publishing, has_story, is_publishing, "own story pressed");
        let _ = SelectionFeedbackGenerator::new().selection_changed();
        if has_story {
            let on_unavailable: StoryActionDoneHandler =
                Rc::new(move |cx: &mut App| Self::open_story_composer(is_publishing, cx));
            StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.open_stories_for_user_id(user_id, Some(on_unavailable), cx);
            });
        } else {
            Self::open_story_composer(is_publishing, cx);
        }
    }

    fn open_story_composer(is_publishing: bool, cx: &mut App) {
        if is_publishing {
            tracing::warn!(target: "riseonly::X1017", "own story composer refused while publishing");
            return;
        }
        StoryFeedComposerPresenter::shared(cx).update(cx, |presenter, cx| presenter.present(cx));
    }

    fn own_story_button(&self, state: &Entity<StoriesTrayState>, cx: &mut App) -> Option<AnyElement> {
        let (auth, service) = {
            let read = state.read(cx);
            (read.auth.clone(), read.service.clone())
        };
        let profile = auth.read(cx).profile()?.clone();
        let theme = rise_ui::theme(cx).clone();
        let (has_story, has_unseen, is_publishing) = {
            let service = service.read(cx);
            (
                service.has_active_stories(Some(profile.id.as_str()), cx),
                service.has_unseen_stories(Some(profile.id.as_str()), cx),
                service.is_publishing || service.is_shared_story_background_activity() || service.publish_ring_is_settling,
            )
        };
        let is_waiting_for_network = service.update(cx, |service, cx| service.is_publish_waiting_for_network(cx));

        let source = SharedString::from(profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()));
        let avatar_user_id = profile.id.clone();
        let caption_user_id = profile.id.clone();
        let user_id = SharedString::from(profile.id.clone());
        let name = SharedString::from(profile.name.clone());

        let avatar = plain(self.child_id("own-avatar"))
            .on_press(move |_, cx| Self::open_own_story(&avatar_user_id, has_story, is_publishing, cx))
            .disabled(is_publishing && !has_story)
            .relative()
            .size(px(70.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                UserLogoUi::new(self.child_id("own-logo"))
                    .source(source)
                    .name(name)
                    .user_id(user_id)
                    .size(62.0)
                    .show_border(false)
                    .show_story_ring(false),
            )
            .child(
                pass_through().absolute().inset_0().flex().items_center().justify_center().child(
                    StoryAvatarRing::new(self.child_id("own-ring"), is_publishing, has_story, has_unseen, 68.0)
                        .line_width(2.5),
                ),
            );

        let add = plain(self.child_id("own-add"))
            .on_press(move |_, cx| {
                let _ = SelectionFeedbackGenerator::new().selection_changed();
                Self::open_story_composer(is_publishing, cx);
            })
            .disabled(is_publishing)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .relative()
                    .size(px(26.0))
                    .rounded_full()
                    .bg(white())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(centred_stroke(theme.bg._100, 2.0, Shape::Circle))
                    .children(IconUi::sized("plus", px(13.0), black(), false)),
            );

        let caption = plain(self.child_id("own-caption"))
            .on_press(move |_, cx| Self::open_own_story(&caption_user_id, has_story, is_publishing, cx))
            .disabled(is_publishing && !has_story)
            .child(
                MainText::new(tr(if is_waiting_for_network {
                    "chats_status_waiting_for_network"
                } else {
                    "story_your_story"
                }))
                .px(12.0)
                .font_weight(FontWeight::MEDIUM)
                .color(theme.text.primary)
                .tac(TextAlignT::Center)
                .line_limit(1)
                .minimum_scale_factor(0.72)
                .w(px(Self::OWN_BUTTON_WIDTH)),
            );

        Some(
            vstack(7.0)
                .flex_none()
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .size(px(70.0))
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, _| {
                            tracing::warn!(target: "riseonly::X1017", has_story, is_publishing, "own bubble mouse down");
                        })
                        .child(avatar)
                        .child(
                        div()
                            .id(self.child_id("own-add-label"))
                            .absolute()
                            .right(px(-5.0))
                            .bottom(px(-5.0))
                            .size(px(34.0))
                            .occlude()
                            .aria_label(tr("story_create_title"))
                            .tooltip_label(tr("story_create_title"))
                            .child(add),
                    ),
                )
                .child(caption)
                .into_any_element(),
        )
    }

    fn story_skeleton(index: usize) -> impl IntoElement {
        vstack(7.0)
            .flex_none()
            .w(px(Self::ITEM_WIDTH))
            .child(SkeletonCircle::new(68.0))
            .child(SkeletonLine::new(if index % 2 == 0 { 54.0 } else { 66.0 }, 8.0))
            .skeleton_shimmer(None)
    }

    fn story_button(&self, item: &Item, groups: &Rc<Vec<Rc<StoryGroupDTO>>>, service: &StoryServicesStore, cx: &App) -> Div {
        let theme = rise_ui::theme(cx);
        let author = item.group.author.as_ref();
        let display_name = author.map(|author| trimming_whitespaces_and_newlines(&author.name)).unwrap_or_default();
        let fallback_tag = author.map(|author| trimming_whitespaces_and_newlines(&author.tag)).unwrap_or_default();
        let title = SharedString::from(if !display_name.is_empty() { display_name } else { fallback_tag }.to_owned());
        let has_unseen = service.has_unseen_stories(Some(item.id.as_str()), cx);
        let source_index = item.source_index;
        let press_groups = groups.clone();

        div().flex_none().child(
            plain(self.keyed_id("story", &item.id))
                .on_press(move |_, cx| {
                    let _ = SelectionFeedbackGenerator::new().selection_changed();
                    let groups: Vec<StoryGroupDTO> = press_groups.iter().map(|group| (**group).clone()).collect();
                    StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.open_stories(groups, source_index, cx);
                    });
                })
                .flex()
                .flex_col()
                .items_center()
                .gap(px(7.0))
                .child(
                    div()
                        .relative()
                        .size(px(70.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            UserLogoUi::new(self.keyed_id("logo", &item.id))
                                .source(author.map(|author| SharedString::from(author.logo.clone())))
                                .name(title.clone())
                                .user_id(SharedString::from(item.id.clone()))
                                .size(62.0)
                                .show_border(false)
                                .show_story_ring(false),
                        )
                        .child(
                            pass_through().absolute().inset_0().flex().items_center().justify_center().child(
                                StoryAvatarRing::new(self.keyed_id("ring", &item.id), false, true, has_unseen, 68.0)
                                    .line_width(2.5),
                            ),
                        ),
                )
                .child(
                    MainText::new(title)
                        .px(12.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.text.primary)
                        .tac(TextAlignT::Center)
                        .line_limit(1)
                        .w(px(Self::ITEM_WIDTH)),
                ),
        )
    }

    fn return_to_start_if_reordered(state: &Entity<StoriesTrayState>, before: &[String], after: &[String], cx: &mut App) {
        if before.is_empty() || after.is_empty() || after.starts_with(before) {
            return;
        }
        let kept: HashSet<&String> = after.iter().collect();
        let survivors: Vec<String> = before.iter().filter(|id| kept.contains(id)).cloned().collect();
        if after.starts_with(&survivors) {
            return;
        }
        let now = RiseClock::monotonic();
        state.update(cx, |state, cx| {
            let scrolled = -f32::from(state.scroll.offset().x);
            let mut motion = Animated::new(scrolled);
            motion.set_in(Self::HORIZONTAL_PADDING, ease_out(0.25), now, cx);
            state.returning_to_start = Some(motion);
            cx.notify();
        });
    }

    fn advance_return_to_start(state: &Entity<StoriesTrayState>, window: &Window, cx: &mut App) {
        let now = RiseClock::monotonic();
        state.update(cx, |state, _| {
            let Some(motion) = state.returning_to_start.as_ref() else {
                return;
            };
            let x = motion.frame_value(now, window);
            let is_running = motion.is_running(now);
            let y = state.scroll.offset().y;
            state.scroll.set_offset(point(px(-x), y));
            if !is_running {
                state.returning_to_start = None;
            }
        });
    }

    fn page_row(state: &Entity<StoriesTrayState>, forward: bool, cx: &mut App) {
        let now = RiseClock::monotonic();
        state.update(cx, |state, cx| {
            let viewport = f32::from(state.scroll.bounds().size.width);
            let max_x = f32::from(state.scroll.max_offset().x).max(0.0);
            if viewport <= 0.0 || max_x <= 0.0 {
                return;
            }
            let current = -f32::from(state.scroll.offset().x);
            let step = viewport * Self::PAGE_STEP_FRACTION;
            let target = if forward { (current + step).min(max_x) } else { (current - step).max(0.0) };
            let mut motion = Animated::new(current);
            motion.set_in(target, ease_out(0.25), now, cx);
            state.returning_to_start = None;
            state.paging = Some(motion);
            cx.notify();
        });
    }

    fn advance_paging(state: &Entity<StoriesTrayState>, window: &Window, cx: &mut App) {
        let now = RiseClock::monotonic();
        state.update(cx, |state, _| {
            let Some(motion) = state.paging.as_ref() else {
                return;
            };
            let x = motion.frame_value(now, window);
            let is_running = motion.is_running(now);
            let y = state.scroll.offset().y;
            state.scroll.set_offset(point(px(-x), y));
            if !is_running {
                state.paging = None;
            }
        });
    }

    fn page_arrows_shown(state: &StoriesTrayState) -> (bool, bool) {
        if !state.is_hovered {
            return (false, false);
        }
        let offset = -f32::from(state.scroll.offset().x);
        let max_offset = f32::from(state.scroll.max_offset().x);
        (offset > 1.0, max_offset > 1.0 && offset < max_offset - 1.0)
    }

    fn page_arrow(&self, state: &Entity<StoriesTrayState>, forward: bool, cx: &App) -> AnyElement {
        let theme = rise_ui::theme(cx);
        let glyph = if forward { "chevron.right" } else { "chevron.left" };
        let state = state.clone();
        let top = Self::VERTICAL_PADDING + (Self::ITEM_WIDTH - Self::PAGE_ARROW_SIZE) / 2.0;
        div()
            .absolute()
            .top(px(top))
            .when(forward, |arrow| arrow.right(px(Self::PAGE_ARROW_INSET)))
            .when(!forward, |arrow| arrow.left(px(Self::PAGE_ARROW_INSET)))
            .size(px(Self::PAGE_ARROW_SIZE))
            .rounded_full()
            .bg(theme.bg._200)
            .shadow_md()
            .occlude()
            .child(
                plain(self.child_id(if forward { "page-forward" } else { "page-back" }))
                    .on_press(move |_window, cx| Self::page_row(&state, forward, cx))
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(glyph, px(12.0), theme.text.primary, false)),
            )
            .into_any_element()
    }

    fn continue_past_empty_page_if_needed(state: &Entity<StoriesTrayState>, cursor: Option<String>, cx: &mut App) {
        let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) else {
            return;
        };
        let should_continue = state.update(cx, |state, _| {
            if state.last_empty_continuation_cursor.as_ref() == Some(&cursor)
                || state.empty_continuation_attempts >= Self::MAX_EMPTY_CONTINUATION_ATTEMPTS
            {
                return false;
            }
            state.last_empty_continuation_cursor = Some(cursor);
            state.empty_continuation_attempts += 1;
            true
        });
        if should_continue {
            StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more_stories_feed(cx));
        }
    }
}

impl RenderOnce for StoriesTray {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, StoriesTrayState::new);
        Self::advance_return_to_start(&state, window, cx);
        Self::advance_paging(&state, window, cx);

        let (auth, actions, service, scroll) = {
            let read = state.read(cx);
            (read.auth.clone(), read.actions.clone(), read.service.clone(), read.scroll.clone())
        };
        let feed = Self::feed(&state, cx);
        let (own_user_id, profile_id) = {
            let auth = auth.read(cx);
            (auth.effective_user_id_for_wire().unwrap_or_default(), auth.profile().map(|profile| profile.id.clone()))
        };
        let items = Self::items(&feed.groups, &own_user_id, cx);
        let item_ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        let prefetched = Self::prefetched_stories(&items, service.read(cx), cx);
        let prefetch_story_ids: Vec<i64> = prefetched.iter().map(|story| story.id).collect();
        let empty_continuation_cursor = Self::empty_continuation_cursor(&items, &feed);
        let status = actions.read(cx).get_stories_feed_sai.status();

        {
            let state = state.clone();
            on_change(self.child_id("return-to-start"), item_ids.clone(), false, window, cx, move |before, after, _, cx| {
                Self::return_to_start_if_reordered(&state, before, after, cx);
            });
        }
        {
            let state = state.clone();
            on_change(self.child_id("reset-continuation"), item_ids, false, window, cx, move |_, ids, _, cx| {
                if !ids.is_empty() {
                    state.update(cx, |state, _| {
                        state.empty_continuation_attempts = 0;
                        state.last_empty_continuation_cursor = None;
                    });
                }
            });
        }
        {
            let user_id = profile_id.clone().filter(|user_id| !user_id.is_empty());
            use_task(self.child_id("stories-state"), profile_id, window, cx, async move |cx| {
                let Some(user_id) = user_id else {
                    return;
                };
                let _ = cx.update(|_, cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.load_stories_state(&[user_id], cx);
                    });
                });
            });
        }
        use_task(self.child_id("prefetch"), prefetch_story_ids, window, cx, async move |cx| {
            let Ok(app) = cx.update(|_, cx| cx.to_async()) else {
                return;
            };
            StoryMediaPrefetcher::prefetch(&prefetched, CleverImageWorkLane::Speculative, &app).await;
        });
        {
            let state = state.clone();
            let cursor = empty_continuation_cursor.clone();
            use_task(self.child_id("empty-continuation"), empty_continuation_cursor, window, cx, async move |cx| {
                let _ = cx.update(|_, cx| Self::continue_past_empty_page_if_needed(&state, cursor, cx));
            });
        }

        let own_button = self.own_story_button(&state, cx);
        let has_own_button = own_button.is_some();
        let groups = Rc::new(feed.groups.clone());
        let next_cursor = feed.next_cursor.clone();
        let has_more = feed.has_more;
        let cache_key = SharedString::from(StoryServicesStore::feed_cache_key());
        let item_count = items.len();

        let own_width = if has_own_button { Self::OWN_BUTTON_WIDTH + Self::ITEM_SPACING } else { 0.0 };
        let scrolled = (-f32::from(scroll.offset().x)).min(f32::from(scroll.max_offset().x).max(0.0)).max(0.0);
        let lazy = Self::lazy_range(
            item_count,
            scrolled,
            f32::from(window.viewport_size().width),
            Self::HORIZONTAL_PADDING + own_width,
        );

        let mut row = hstack(Self::ITEM_SPACING)
            .items_start()
            .flex_none()
            .px(px(Self::HORIZONTAL_PADDING))
            .py(px(Self::VERTICAL_PADDING))
            .children(own_button)
            .children(Self::lazy_placeholder(lazy.start));
        for index in lazy.clone() {
            let item = &items[index];
            let button = self.story_button(item, &groups, service.read(cx), cx);
            row = row.child(button.sai_pagination_prefetch(
                self.keyed_id("page", &item.id),
                SaiPaginationPrefetchOptions::new(index, item_count, cache_key.clone(), next_cursor.clone(), has_more)
                    .threshold(Self::PAGINATION_THRESHOLD),
                |_, cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more_stories_feed(cx));
                },
            ));
        }
        row = row.children(Self::lazy_placeholder(item_count - lazy.end));
        if items.is_empty() && Self::awaits_first_feed(status, feed.is_loaded) {
            row = row.children((0..Self::SKELETON_COUNT).map(Self::story_skeleton));
        }

        let (arrow_back, arrow_forward) = Self::page_arrows_shown(state.read(cx));
        let back = arrow_back.then(|| self.page_arrow(&state, false, cx));
        let forward = arrow_forward.then(|| self.page_arrow(&state, true, cx));
        let hover_state = state.clone();

        let scroller = div()
            .id(self.child_id("scroll"))
            .w_full()
            .h(px(Self::ROW_HEIGHT))
            .flex()
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&scroll)
            .child(row)
            .on_appear(self.child_id("appear"), |_, cx| {
                UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.ensure_my_profile_loaded(cx));
            });

        div()
            .id(self.child_id("pager"))
            .relative()
            .w_full()
            .on_hover(move |hovered: &bool, _, cx| {
                let hovered = *hovered;
                hover_state.update(cx, |state, cx| {
                    if state.is_hovered != hovered {
                        state.is_hovered = hovered;
                        cx.notify();
                    }
                });
            })
            .child(scroller)
            .children(back)
            .children(forward)
    }
}
