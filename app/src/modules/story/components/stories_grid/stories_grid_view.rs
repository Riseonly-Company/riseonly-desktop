use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, App, AppContext, Context, Div, ElementId, Entity, IntoElement, ParentElement, Pixels, Point,
    Render, RenderOnce, SharedString, StyleRefinement, Styled, Window, black, div, px, white,
};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::swiftui::environment::{EnvKey, env, env_scope};
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::stacks::{Alignment, zstack};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementPart, SharedElementReturnScrollMath,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::SharedElementOriginContainer;
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementHostProbe;
use crate::core::shared_element_transition::runtime::shared_element_lift_board::{
    SharedElementLiftModifiers, SharedElementSurfaceKey,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::gifts::components::gift_shop_shelf_grid::{GiftScrollWindowProbe, GiftShopShelfRows};
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::components::post_collection_ui::PostPaginationTriggerModifiers;
use crate::modules::story::components::stories_grid::stories_grid_action_bar::{Confirmation, StoriesGridActionBar};
use crate::modules::story::components::stories_grid::stories_grid_cell::{StoriesGridCell, StoriesGridMetrics};
use crate::modules::story::components::stories_grid::stories_grid_shared_element_origin::StoriesGridSharedElementOrigin;
use crate::modules::story::shared::story_grid_selection::StoryGridSelection;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_wire_types::{StoryDTO, StoryDTOExt};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

/// what a stories grid may do with a story, decided by who is looking
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoriesGridPermissions {
    pub is_owner: bool,
    pub scope: StoryProfileScope,
}

impl StoriesGridPermissions {
    pub fn new(is_owner: bool, scope: StoryProfileScope) -> Self {
        Self { is_owner, scope }
    }

    pub fn shows_view_count(&self) -> bool {
        self.is_owner
    }

    pub fn allows_curation(&self) -> bool {
        self.is_owner
    }

    pub fn allows_pinning(&self) -> bool {
        self.is_owner && self.scope == StoryProfileScope::Profile
    }
}

pub type StoriesGridLoadMoreHandler = Rc<dyn Fn(&mut App)>;
pub type StoriesGridStoryHandler = Rc<dyn Fn(&StoryDTO, &mut App)>;
pub type StoriesGridFlagHandler = Rc<dyn Fn(&[i64], bool, &mut App)>;
pub type StoriesGridIdsHandler = Rc<dyn Fn(&[i64], &mut App)>;

thread_local! {
    static LAST_CONTAINER_WIDTH: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// width a grid container is known to have before its measured, only the first frame reads it
pub struct ContainerWidthHintKey;

impl EnvKey for ContainerWidthHintKey {
    type Value = Option<f32>;

    fn default_value() -> Self::Value {
        None
    }

    fn fingerprint(_: &Self::Value) -> Option<u64> {
        None
    }
}

/// first frame width of a grid, the hint then the last measured width then the window
pub(crate) fn initial_container_width(window: &Window, cx: &App) -> f32 {
    if let Some(hint) = env::<ContainerWidthHintKey>(cx).filter(|hint| *hint > 1.0) {
        return hint;
    }
    let last = LAST_CONTAINER_WIDTH.with(|last| last.get());
    if last > 1.0 { last } else { f32::from(window.viewport_size().width) }
}

/// records a measured grid width as the next mount first frame guess
pub(crate) fn remember_container_width(width: f32) {
    if width > 1.0 {
        LAST_CONTAINER_WIDTH.with(|last| last.set(width));
    }
}

/// parent width for layout, the column beside the desktop bars not the window
/// it only changes with the window so it cant feed back into itself
pub(crate) fn use_container_width(id: ElementId, window: &mut Window, cx: &mut App) -> Entity<f32> {
    let initial = initial_container_width(window, cx);
    window.use_keyed_state(id, cx, move |_, _| initial)
}

pub(crate) fn container_width_reader(id: ElementId, state: Entity<f32>) -> impl IntoElement {
    measure(id, move |bounds, _, cx| {
        let width = f32::from(bounds.size.width).max(1.0);
        remember_container_width(width);
        state.update(cx, |current, cx| {
            if *current != width {
                *current = width;
                cx.notify();
            }
        });
    })
}

/// owns cells and hold menu and selection only,
/// paging and refresh and the floating bar belong to the screen
#[derive(IntoElement)]
pub struct StoriesGridView {
    id: ElementId,
    owner_user_id: String,
    permissions: StoriesGridPermissions,
    stories: Rc<Vec<StoryDTO>>,
    has_more: bool,
    on_load_more: StoriesGridLoadMoreHandler,
    on_open: StoriesGridStoryHandler,
    on_edit: StoriesGridStoryHandler,
    on_forward: StoriesGridStoryHandler,
    on_archive: StoriesGridFlagHandler,
    on_pin: StoriesGridFlagHandler,
    on_delete: StoriesGridIdsHandler,
    shared_element_origin: Option<Rc<StoriesGridSharedElementOrigin>>,
}

impl StoriesGridView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner_user_id: impl Into<String>,
        permissions: StoriesGridPermissions,
        stories: Rc<Vec<StoryDTO>>,
        has_more: bool,
        on_load_more: StoriesGridLoadMoreHandler,
        on_open: StoriesGridStoryHandler,
        on_edit: StoriesGridStoryHandler,
        on_forward: StoriesGridStoryHandler,
        on_archive: StoriesGridFlagHandler,
        on_pin: StoriesGridFlagHandler,
        on_delete: StoriesGridIdsHandler,
    ) -> Self {
        let owner_user_id = owner_user_id.into();
        Self {
            id: ElementId::Name(SharedString::from(format!(
                "stories-grid-view-{}-{}",
                owner_user_id,
                permissions.scope.raw_value()
            ))),
            owner_user_id,
            permissions,
            stories,
            has_more,
            on_load_more,
            on_open,
            on_edit,
            on_forward,
            on_archive,
            on_pin,
            on_delete,
            shared_element_origin: None,
        }
    }

    pub fn shared_element_origin(mut self, origin: Option<Rc<StoriesGridSharedElementOrigin>>) -> Self {
        self.shared_element_origin = origin;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn hold_actions(permissions: StoriesGridPermissions, story: &StoryDTO) -> Vec<HoldContextMenuAction> {
        if !permissions.allows_curation() {
            return vec![
                HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
                HoldContextMenuAction::new("forward", tr("ctx_forward")).system_image("arrowshape.turn.up.right"),
                HoldContextMenuAction::new("report", tr("story_menu_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            ];
        }

        let mut items: Vec<HoldContextMenuAction> = Vec::new();
        if permissions.scope == StoryProfileScope::Profile {
            items.push(HoldContextMenuAction::new("archive", tr("story_action_archive")).system_image("archivebox"));
            let is_pinned = story.is_pinned_in_profile();
            items.push(
                HoldContextMenuAction::new("pin", if is_pinned { tr("story_action_unpin") } else { tr("story_action_pin") })
                    .system_image(if is_pinned { "pin.slash" } else { "pin" }),
            );
            items.push(HoldContextMenuAction::new("edit", tr("ctx_edit")).system_image("pencil"));
        } else {
            items.push(
                HoldContextMenuAction::new("unarchive", tr("story_action_unarchive"))
                    .system_image("arrow.uturn.backward"),
            );
        }
        items.push(HoldContextMenuAction::new("forward", tr("ctx_forward")).system_image("arrowshape.turn.up.right"));
        items.push(HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true));
        items.push(
            HoldContextMenuAction::new("select", tr("ctx_select"))
                .system_image("checkmark.circle")
                .section_break(true)
                .guide_anchor(RiseGuideAnchor::StoryMenuSelect),
        );
        items
    }

    fn selection_actions(permissions: StoriesGridPermissions, is_selection_empty: bool) -> Vec<HoldContextMenuAction> {
        let primary = if permissions.scope == StoryProfileScope::Archive {
            HoldContextMenuAction::new("selection-unarchive", tr("story_action_unarchive"))
                .system_image("arrow.uturn.backward")
        } else {
            HoldContextMenuAction::new("selection-archive", tr("story_action_archive")).system_image("archivebox")
        };
        vec![
            primary.disabled(is_selection_empty),
            HoldContextMenuAction::new("selection-delete", tr("ctx_delete"))
                .system_image("trash")
                .destructive(true)
                .disabled(is_selection_empty),
        ]
    }

    fn handle_action(this: &StoriesGridViewActions, id: &str, story: &StoryDTO, cx: &mut App) {
        match id {
            "open" => (this.on_open)(story, cx),
            "report" => {
                let story_id = story.id;
                let author_id = story.author_id.clone();
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.report_story(Some(story_id), Some(&author_id), cx);
                });
            }
            "forward" => (this.on_forward)(story, cx),
            "edit" => (this.on_edit)(story, cx),
            "archive" => (this.on_archive)(&[story.id], true, cx),
            "unarchive" => (this.on_archive)(&[story.id], false, cx),
            "pin" => (this.on_pin)(&[story.id], !story.is_pinned_in_profile(), cx),
            "delete" => (this.on_delete)(&[story.id], cx),
            "select" => {
                let story_id = story.id;
                let owner = this.owner_user_id.clone();
                let scope = this.permissions.scope;
                StoryGridSelection::shared(cx).update(cx, |selection, cx| {
                    selection.begin(&owner, scope, Some(story_id), cx);
                });
            }
            "selection-archive" => StoriesGridActionBar::alert(Confirmation::Archive, &this.owner_user_id, cx),
            "selection-unarchive" => StoriesGridActionBar::alert(Confirmation::Unarchive, &this.owner_user_id, cx),
            "selection-delete" => StoriesGridActionBar::alert(Confirmation::Delete, &this.owner_user_id, cx),
            _ => {}
        }
    }

    fn handle_tap(this: &StoriesGridViewActions, story: &StoryDTO, drawn_selecting: bool, window: &Window, cx: &mut App) {
        let modifiers = window.modifiers();
        let selection = StoryGridSelection::shared(cx);
        let story_id = story.id;
        let is_selecting = selection.read(cx).is_active_for(&this.owner_user_id, this.permissions.scope);
        tracing::warn!(target: "riseonly::X1161", story_id, drawn_selecting, is_selecting, "stories grid tap");
        if is_selecting {
            let range = modifiers.shift.then(|| Self::selection_range(this, selection.read(cx).selected_ids(), story_id));
            selection.update(cx, |selection, cx| match range.flatten() {
                Some(range) => {
                    for id in range {
                        if !selection.contains(id) {
                            selection.toggle(id, cx);
                        }
                    }
                }
                None => selection.toggle(story_id, cx),
            });
            return;
        }
        if this.permissions.allows_curation() && (modifiers.shift || modifiers.secondary()) {
            let owner = this.owner_user_id.clone();
            let scope = this.permissions.scope;
            selection.update(cx, |selection, cx| selection.begin(&owner, scope, Some(story_id), cx));
            return;
        }
        (this.on_open)(story, cx);
    }

    fn selection_range(this: &StoriesGridViewActions, selected_ids: &[i64], story_id: i64) -> Option<Vec<i64>> {
        let anchor = *selected_ids.last()?;
        let position = |id: i64| this.stories.iter().position(|story| story.id == id);
        let (from, to) = (position(anchor)?, position(story_id)?);
        let range = from.min(to)..=from.max(to);
        Some(this.stories[range].iter().map(|story| story.id).collect())
    }

    /// one cell from its props,
    /// handlers read the current actions at the event so a replayed cell still acts on todays list
    fn cell(props: &StoriesGridCellProps, actions: &StoriesGridActionsSlot) -> AnyElement {
        let frame = props.frame;
        let story = &props.story;
        let story_id = story.id;
        let menu_actions = if frame.is_pending {
            Vec::new()
        } else if frame.is_selecting {
            if frame.permissions.allows_curation() {
                Self::selection_actions(frame.permissions, frame.is_selection_empty)
            } else {
                Vec::new()
            }
        } else {
            Self::hold_actions(frame.permissions, story)
        };
        let guide_anchor = (frame.index == 0
            && frame.permissions.scope == StoryProfileScope::Profile
            && frame.permissions.is_owner)
            .then_some(RiseGuideAnchor::ProfileFirstStory);
        let shows_view_count = frame.permissions.shows_view_count();
        let (cell_width, cell_height) = (frame.cell_width, frame.cell_height);
        let (is_pending, is_selecting, selection_order) = (frame.is_pending, frame.is_selecting, frame.selection_order);

        let content_story = story.clone();
        let action_story = story.clone();
        let tap_story = story.clone();
        let action_slot = actions.clone();
        let tap_slot = actions.clone();
        let load_slot = actions.clone();
        HoldContextMenuContainer::new(
            ElementId::NamedInteger(SharedString::new_static("stories-grid-hold"), story_id as u64),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| menu_actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                let cell = StoriesGridCell::new(
                    content_story.clone(),
                    cell_width,
                    cell_height,
                    shows_view_count,
                    selection_order,
                    is_selecting,
                );
                div()
                    .relative()
                    .child(cell)
                    .when(is_pending, |cell| {
                        cell.child(
                            zstack(Alignment::Center)
                                .absolute()
                                .inset_0()
                                .bg(black().opacity(0.35))
                                .child(LoaderUi::new().size(LoaderUiSize::Large).color(Some(white()))),
                        )
                    })
                    .shared_element_anchor(
                        StoriesGridSharedElementOrigin::item_id(content_story.id),
                        SharedElementPart::Media,
                    )
                    .guide_anchor(guide_anchor)
                    .into_any_element()
            }),
        )
        .show_emoji_list(false)
        .tap_movement_tolerance(4.0)
        .source_identity(Some(format!("story-{story_id}")))
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            let Some(this) = action_slot.read(cx).clone() else { return };
            Self::handle_action(&this, &id, &action_story, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, window: &mut Window, cx: &mut App| {
            if is_pending {
                return;
            }
            let Some(this) = tap_slot.read(cx).clone() else { return };
            Self::handle_tap(&this, &tap_story, is_selecting, window, cx);
        }))
        .post_pagination_trigger(
            ElementId::NamedInteger(SharedString::new_static("stories-grid-pagination"), story_id as u64),
            frame.index as i64,
            frame.item_count as i64,
            frame.has_more,
            false,
            move |cx| {
                let Some(this) = load_slot.read(cx).clone() else { return };
                (this.on_load_more)(cx);
            },
        )
        .into_any_element()
    }
}

/// latest handlers of the grid, swapped in silently on every grid render
type StoriesGridActionsSlot = Entity<Option<Rc<StoriesGridViewActions>>>;

/// everything a cell draws except the story, compared by value each grid render
#[derive(Clone, Copy, PartialEq)]
struct StoriesGridCellFrame {
    index: usize,
    item_count: usize,
    has_more: bool,
    permissions: StoriesGridPermissions,
    cell_width: f32,
    cell_height: f32,
    is_selecting: bool,
    selection_order: Option<usize>,
    is_selection_empty: bool,
    is_pending: bool,
}

/// what one cell draws from, a change renders it fresh otherwise it replays its last frame
#[derive(Clone)]
struct StoriesGridCellProps {
    frame: StoriesGridCellFrame,
    story: StoryDTO,
}

impl StoriesGridCellProps {
    /// true if these props are what this grid render would build, compared without cloning
    fn matches(&self, frame: &StoriesGridCellFrame, story: &StoryDTO) -> bool {
        self.frame == *frame && self.story == *story
    }
}

struct StoriesGridCellHolder {
    cell: Entity<StoriesGridCellView>,
}

struct StoriesGridCellView {
    props: Option<StoriesGridCellProps>,
    actions: Option<StoriesGridActionsSlot>,
}

impl StoriesGridCellView {
    fn new() -> Self {
        Self { props: None, actions: None }
    }

    fn set_props(&mut self, props: Option<StoriesGridCellProps>, actions: StoriesGridActionsSlot) {
        self.actions = Some(actions);
        if let Some(props) = props {
            self.props = Some(props);
        }
    }
}

impl Render for StoriesGridCellView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (width, height) =
            self.props.as_ref().map_or((0.0, 0.0), |props| (props.frame.cell_width, props.frame.cell_height));
        let cell = self
            .props
            .as_ref()
            .zip(self.actions.as_ref())
            .map(|(props, actions)| StoriesGridView::cell(props, actions));
        div().w(px(width)).h(px(height)).children(cell)
    }
}

/// what the handlers of one frames cells share
struct StoriesGridViewActions {
    owner_user_id: String,
    permissions: StoriesGridPermissions,
    stories: Rc<Vec<StoryDTO>>,
    on_open: StoriesGridStoryHandler,
    on_edit: StoriesGridStoryHandler,
    on_forward: StoriesGridStoryHandler,
    on_archive: StoriesGridFlagHandler,
    on_pin: StoriesGridFlagHandler,
    on_delete: StoriesGridIdsHandler,
    on_load_more: StoriesGridLoadMoreHandler,
}

impl RenderOnce for StoriesGridView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let width_state = use_container_width(self.child_id("width"), window, cx);
        let width = (*width_state.read(cx)).max(1.0);
        let cell_width = StoriesGridMetrics::cell_width(width);
        let cell_height = StoriesGridMetrics::cell_height(width);
        let item_count = self.stories.len() as i64;
        let grid_height = StoriesGridMetrics::grid_height(item_count, width);
        let column_count = StoriesGridMetrics::columns(width);

        let selection = StoryGridSelection::shared(cx);
        let is_selecting = selection.read(cx).is_active_for(&self.owner_user_id, self.permissions.scope);
        let is_selection_empty = selection.read(cx).is_empty();

        let ids: Vec<i64> = self.stories.iter().map(|story| story.id).collect();
        let prune_selection = selection.clone();
        on_change(self.child_id("ids"), ids, false, window, cx, move |_, ids, _, cx| {
            let available: HashSet<i64> = ids.iter().copied().collect();
            prune_selection.update(cx, |selection, cx| selection.prune(&available, cx));
        });

        let row_count = GiftShopShelfRows::count(item_count, column_count);
        let initial_rows =
            (f32::from(window.viewport_size().height) / StoriesGridMetrics::row_height(width)).ceil() as i64 + 1;
        let rows_state: Entity<Option<GiftShopShelfRows>> =
            window.use_keyed_state(self.child_id("rows"), cx, |_, _| None);
        let rows = rows_state
            .read(cx)
            .unwrap_or(GiftShopShelfRows::leading(initial_rows))
            .clamped(row_count);
        let range = rows.items(column_count, item_count);

        let actions = Rc::new(StoriesGridViewActions {
            owner_user_id: self.owner_user_id.clone(),
            permissions: self.permissions,
            stories: self.stories.clone(),
            on_open: self.on_open.clone(),
            on_edit: self.on_edit.clone(),
            on_forward: self.on_forward.clone(),
            on_archive: self.on_archive.clone(),
            on_pin: self.on_pin.clone(),
            on_delete: self.on_delete.clone(),
            on_load_more: self.on_load_more.clone(),
        });
        let actions_slot: StoriesGridActionsSlot = window.use_keyed_state(self.child_id("actions"), cx, |_, _| None);
        actions_slot.update(cx, |slot, _| *slot = Some(actions));

        let columns = column_count as usize;
        let columns_width = columns as f32 * cell_width + (columns - 1) as f32 * StoriesGridMetrics::SPACING;
        let origin_x = ((width - columns_width) / 2.0).max(0.0);
        let service = StoryServicesStore::shared(cx);
        let mut cells: Vec<AnyElement> = Vec::with_capacity((range.end - range.start).max(0) as usize);
        let item_count_usize = self.stories.len();
        for index in range.start as usize..range.end as usize {
            let story = &self.stories[index];
            let cell_frame = StoriesGridCellFrame {
                index,
                item_count: item_count_usize,
                has_more: self.has_more,
                permissions: self.permissions,
                cell_width,
                cell_height,
                is_selecting,
                selection_order: selection.read(cx).order(story.id),
                is_selection_empty,
                is_pending: service.read(cx).is_pending_story(story.id, cx),
            };
            let frame = SharedElementReturnScrollMath::grid_item_rect_with_origin_x(
                index,
                columns,
                cell_width,
                cell_height,
                StoriesGridMetrics::SPACING,
                0.0,
                origin_x,
            );
            let host_id = ElementId::NamedChild(
                Arc::new(self.id.clone()),
                SharedString::from(format!("cell-view-{}", story.id)),
            );
            let host = window
                .use_keyed_state(host_id, cx, |_, cx| StoriesGridCellHolder {
                    cell: cx.new(|_| StoriesGridCellView::new()),
                })
                .read(cx)
                .cell
                .clone();
            let mounted = host.read(cx).props.as_ref().map(|props| props.matches(&cell_frame, story));
            let fresh = (mounted != Some(true)).then(|| StoriesGridCellProps { frame: cell_frame, story: story.clone() });
            let stale = mounted == Some(false);
            let host_actions = actions_slot.clone();
            host.update(cx, |host, _| host.set_props(fresh, host_actions));
            let cell = if stale {
                AnyView::from(host).into_any_element()
            } else {
                host.cached(StyleRefinement::default().w(px(cell_width)).h(px(cell_height)))
                    .replay_when_moved()
                    .into_any_element()
            };
            cells.push(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(cell)
                    .into_any_element(),
            );
        }

        let probe_rows = rows_state;
        let window_probe = GiftScrollWindowProbe::new(self.child_id("window-probe"), move |band, height, cx| {
            let Some(band) = band else { return };
            if height <= 0.0 {
                return;
            }
            probe_rows.update(cx, |mounted, cx| {
                let next = GiftShopShelfRows::following(
                    &band,
                    height,
                    row_count,
                    StoriesGridMetrics::SPACING,
                    mounted.unwrap_or(GiftShopShelfRows::leading(initial_rows)),
                );
                if Some(next) != *mounted {
                    *mounted = Some(next);
                    cx.notify();
                }
            });
        });

        let surface = self
            .shared_element_origin
            .as_ref()
            .map(|_| StoriesGridSharedElementOrigin::surface(&self.owner_user_id));
        let host_probe = self.shared_element_origin.as_ref().map(|origin| {
            let container: Rc<dyn SharedElementOriginContainer> = origin.clone();
            let origin = origin.clone();
            SharedElementHostProbe::origin(container).on_layout(move |bounds, window, _| {
                origin.record_layout(bounds, width, window);
            })
        });

        div()
            .relative()
            .w_full()
            .h(px(grid_height))
            .child(container_width_reader(self.child_id("width-reader"), width_state))
            .children(host_probe)
            .child(env_scope::<SharedElementSurfaceKey>(surface).child(div().absolute().inset_0().children(cells)))
            .child(div().absolute().inset_0().child(window_probe))
    }
}

type StoriesGridStateContent = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;

/// own skeleton with the story cell shape, a square placeholder would resize the page on first paint
#[derive(IntoElement)]
pub struct StoriesGridStateUi {
    id: ElementId,
    pub is_loaded: bool,
    pub is_empty: bool,
    pub empty_icon: String,
    pub empty_text: String,
    content: StoriesGridStateContent,
}

impl StoriesGridStateUi {
    pub fn new<Content: IntoElement>(
        id: impl Into<ElementId>,
        is_loaded: bool,
        is_empty: bool,
        empty_icon: impl Into<String>,
        empty_text: impl Into<String>,
        content: impl FnOnce(&mut Window, &mut App) -> Content + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_loaded,
            is_empty,
            empty_icon: empty_icon.into(),
            empty_text: empty_text.into(),
            content: Box::new(move |window, cx| content(window, cx).into_any_element()),
        }
    }

    fn skeleton(id: &ElementId, window: &mut Window, cx: &mut App) -> Div {
        let width_id = ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static("width"));
        let reader_id = ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static("width-reader"));
        let width_state = use_container_width(width_id, window, cx);
        let width = (*width_state.read(cx)).max(1.0);
        div()
            .relative()
            .w_full()
            .px(px(StoriesGridMetrics::HORIZONTAL_PADDING))
            .child(
                PostGridSkeleton::default()
                    .columns(StoriesGridMetrics::columns(width))
                    .spacing(StoriesGridMetrics::SPACING)
                    .cell_height(StoriesGridMetrics::cell_height(width))
                    .rows(4),
            )
            .child(container_width_reader(reader_id, width_state))
    }
}

impl RenderOnce for StoriesGridStateUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.is_loaded {
            Self::skeleton(&self.id, window, cx).into_any_element()
        } else if self.is_empty {
            ChatProfileEmptyState::new(self.empty_icon, self.empty_text).into_any_element()
        } else {
            (self.content)(window, cx)
        }
    }
}
