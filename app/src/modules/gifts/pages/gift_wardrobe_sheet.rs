use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyView, App, Bounds, Context, Entity, FontWeight, MouseButton, MouseDownEvent, Pixels, ScrollHandle, SharedString,
    Size, StyleRefinement, Subscription, Window, bounds, point, size, white,
};
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::button_ui::ButtonUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::{SheetFloatingHeader, SheetFloatingHeaderMetrics};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::gifts::components::gift_card_view::{GiftBackdropColors, GiftPatternLayer};
use crate::modules::gifts::components::gift_cosmetic_playback::GiftCosmeticGeometry;
use crate::modules::gifts::components::gift_cosmetic_preview::{
    GiftBackdropSurface, GiftCosmeticPreview, GiftEquippedBadge,
};
use crate::modules::gifts::components::gift_shop_shelf_grid::GiftShopShelfGrid;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAppearanceExt, RiseGiftInstanceDTO, RiseGiftInstanceExt, RiseGiftItemKind,
};
use crate::modules::gifts::pages::gift_shop_sheet::GiftSheetScrollReader;
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;
use crate::modules::gifts::shared::gift_people::GiftPeople;
use crate::modules::gifts::stores::gift_appearance::gift_appearance_services::gift_appearance_services::GiftAppearanceServicesStore;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::{GiftRecipient, GiftsInteractionsStore};
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::components::post_collection_ui::PostPaginationTriggerModifiers;

const GRID_SPACING: f32 = 10.0;
const WARDROBE_PAGE_THRESHOLD: i64 = 6;

/// state of the sheet, me and revealID and the grid place for scroll to
struct GiftWardrobeSheetState {
    me: Option<GiftRecipient>,
    reveal_id: Option<String>,
    menu_instance_id: Option<String>,
    grid_bounds: Rc<Cell<Option<(Bounds<Pixels>, Pixels)>>>,
    revealed_cell: Rc<RefCell<Option<(String, Bounds<Pixels>, Pixels)>>>,
    scroll: ScrollHandle,
}

/// customization picker of one slot, the viewer own cosmetics of that kind each drawn on the
/// viewer with the worn one marked
/// a tap puts one on and take off empties the slot, the shop opens on the same shelf
#[derive(IntoElement)]
pub struct GiftWardrobeSheetContent;

impl GiftWardrobeSheetContent {
    pub fn new() -> Self {
        Self
    }

    /// scrolls to the worn cell, its own measured place when built else its row in the grid
    /// since the grid builds only rows near the viewport
    /// a frame the scroll only replays runs no measure
    fn reveal(
        worn: &str,
        kind: RiseGiftItemKind,
        state: &Entity<GiftWardrobeSheetState>,
        reader: &Entity<GiftSheetScrollReader>,
        cx: &mut App,
    ) {
        let (grid, revealed, scroll_y) = {
            let state = state.read(cx);
            (state.grid_bounds.get(), state.revealed_cell.borrow().clone(), state.scroll.offset().y)
        };
        let shifted = |place: Bounds<Pixels>, measured_y: Pixels| {
            bounds(point(place.origin.x, place.origin.y + (scroll_y - measured_y)), place.size)
        };
        if let Some((id, cell, measured_y)) = revealed {
            if id == worn {
                let item = shifted(cell, measured_y);
                reader.update(cx, |reader, cx| reader.reveal(item, true, 0.25, cx));
                return;
            }
        }
        let Some((grid, measured_y)) = grid else {
            return;
        };
        let grid = shifted(grid, measured_y);
        let services = GiftsServicesStore::shared(cx);
        let items = &services.read(cx).wardrobe(kind).items;
        let Some(index) = items.iter().position(|instance| instance.id == worn) else {
            return;
        };
        let columns = kind.shop_columns();
        let rows = items.len().div_ceil(columns).max(1);
        let row = index / columns;
        let row_height = (f32::from(grid.size.height) + GRID_SPACING) / rows as f32 - GRID_SPACING;
        let top = f32::from(grid.top()) + row as f32 * (row_height + GRID_SPACING);
        let item = bounds(point(grid.left(), px(top)), size(grid.size.width, px(row_height)));
        reader.update(cx, |reader, cx| reader.reveal(item, true, 0.25, cx));
    }

    fn shelf(
        kind: RiseGiftItemKind,
        state: &Entity<GiftWardrobeSheetState>,
        cx: &mut App,
    ) -> AnyElement {
        let shelf = GiftsServicesStore::shared(cx).read(cx).wardrobe(kind).clone();
        if !shelf.loaded {
            return div()
                .w_full()
                .px(px(16.0))
                .pt(px(8.0))
                .child(
                    PostGridSkeleton::default()
                        .columns(kind.shop_columns() as i64)
                        .spacing(GRID_SPACING)
                        .cell_height(150.0)
                        .rows(2),
                )
                .into_any_element();
        }
        if shelf.items.is_empty() {
            return div()
                .w_full()
                .pt(px(24.0))
                .child(ChatProfileEmptyState::new("sparkles", tr("customization_appearance_empty")))
                .into_any_element();
        }
        let me = state.read(cx).me.clone();
        let template_me = me.clone();
        let template_name = shelf.items.first().map(|instance| instance.name.clone()).unwrap_or_default();
        let item_count = shelf.items.len() as i64;
        let (has_more, is_loading_more) = (shelf.has_more, shelf.loading);
        let items: Vec<(i64, RiseGiftInstanceDTO)> =
            shelf.items.into_iter().enumerate().map(|(index, instance)| (index as i64, instance)).collect();
        let revealer = state.clone();
        let (grid_bounds, grid_scroll) = {
            let state = state.read(cx);
            (state.grid_bounds.clone(), state.scroll.clone())
        };
        div()
            .w_full()
            .px(px(16.0))
            .pt(px(8.0))
            .child(
                div()
                    .relative()
                    .w_full()
                    .child(
                        GiftShopShelfGrid::new(
                            SharedString::from(format!("gift-wardrobe-grid/{}", kind.slug())),
                            items,
                            kind.shop_columns() as i64,
                            move |_, _| GiftWardrobeCell::template(kind, template_name, template_me),
                            move |(index, instance): &(i64, RiseGiftInstanceDTO), window, cx| {
                                let (is_menu_presented, is_revealing) = {
                                    let state = revealer.read(cx);
                                    (
                                        state.menu_instance_id.as_deref() == Some(instance.id.as_str()),
                                        state.reveal_id.as_deref() == Some(instance.id.as_str()),
                                    )
                                };
                                let near_tail = *index >= item_count - WARDROBE_PAGE_THRESHOLD;
                                let props = GiftWardrobeCellProps {
                                    index: *index,
                                    instance: instance.clone(),
                                    person: me.clone(),
                                    item_count: if near_tail { item_count } else { 0 },
                                    has_more: near_tail && has_more,
                                    is_loading_more: near_tail && is_loading_more,
                                    is_menu_presented,
                                    is_revealing,
                                };
                                let host = window.use_keyed_state(
                                    SharedString::from(format!("gift-wardrobe-cell-view/{}", instance.id)),
                                    cx,
                                    |_, _: &mut Context<GiftWardrobeCellView>| GiftWardrobeCellView::new(),
                                );
                                let stale = host.update(cx, |host, _| host.set_props(props, revealer.clone()));
                                if stale {
                                    AnyView::from(host).into_any_element()
                                } else {
                                    host.cached(GiftWardrobeCellArt::layer_style()).replay_when_moved().into_any_element()
                                }
                            },
                        )
                        .spacing(GRID_SPACING)
                        .initial_rows(2),
                    )
                    .child(measure("gift-wardrobe-grid-frame", move |bounds, _, _| {
                        grid_bounds.set(Some((bounds, grid_scroll.offset().y)))
                    })),
            )
            .into_any_element()
    }

    fn cell(
        props: GiftWardrobeCellProps,
        state: Entity<GiftWardrobeSheetState>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let GiftWardrobeCellProps { index, instance, person, item_count, has_more, is_loading_more, .. } = props;
        let instance = &instance;
        let id = instance.id.clone();
        let art = GiftWardrobeCellArt::new(instance, person.clone(), window, cx);
        let tapped = instance.clone();
        let tap_state = state.clone();
        let hold_state = state.clone();
        let reveal_probe = {
            let state = state.read(cx);
            (state.reveal_id.as_deref() == Some(id.as_str()))
                .then(|| (state.revealed_cell.clone(), state.scroll.clone(), id.clone()))
        };
        let held_id = id.clone();
        let menu_get_id = id.clone();
        let menu_set_id = id.clone();
        let is_presented = Binding::entity(
            state,
            move |state: &GiftWardrobeSheetState| state.menu_instance_id.as_deref() == Some(menu_get_id.as_str()),
            move |state: &mut GiftWardrobeSheetState, is_presented: bool, cx: &mut Context<GiftWardrobeSheetState>| {
                if !is_presented && state.menu_instance_id.as_deref() == Some(menu_set_id.as_str()) {
                    state.menu_instance_id = None;
                    cx.notify();
                }
            },
        );
        let listed = instance.clone();
        let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| {
            vec![
                HoldContextMenuAction::new(
                    "toggle_equipped",
                    tr(if listed.is_equipped() { "gift_equip_take_off" } else { "gift_equip_put_on" }),
                )
                .system_image(if listed.is_equipped() { "xmark.circle" } else { "checkmark.circle" }),
                HoldContextMenuAction::new("info", tr("gift_info_open")).system_image("gift"),
            ]
        });
        let acted = instance.clone();
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            let instance = acted.clone();
            if id == "toggle_equipped" {
                GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_equipped(instance, cx));
            } else if id == "info" {
                GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.open_instance(instance, None, cx));
            }
        });
        let source: HoldContextMenuTapSource = Rc::new(|_: &mut Window, _: &mut App| div().into_any_element());
        let button = plain(SharedString::from(format!("gift-wardrobe-cell/{id}")))
            .w_full()
            .on_press(move |_, cx| {
                if tap_state.read(cx).menu_instance_id.is_some() || tapped.is_equipped() {
                    return;
                }
                let instance = tapped.clone();
                tap_state.update(cx, |state, _| state.reveal_id = Some(instance.id.clone()));
                GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.set_equipped(instance, true, None, cx));
            })
            .child(GiftWardrobeCell::new(instance.clone(), person, art));
        div()
            .w_full()
            .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                ImpactFeedbackGenerator::new(ImpactStyle::Medium).impact_occurred();
                let held = held_id.clone();
                hold_state.update(cx, |state, cx| {
                    state.menu_instance_id = Some(held);
                    cx.notify();
                });
            })
            .child(div().relative().w_full().child(button).children(reveal_probe.map(|(revealed_cell, scroll, id)| {
                measure(SharedString::from(format!("gift-wardrobe-reveal/{id}")), move |bounds, _, _| {
                    *revealed_cell.borrow_mut() = Some((id, bounds, scroll.offset().y));
                })
            })))
            .tap_context_menu(
                SharedString::from(format!("gift-wardrobe-menu/{id}")),
                is_presented,
                HoldContextMenuAlignment::Leading,
                false,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            )
            .post_pagination_trigger_threshold(
                SharedString::from(format!("gift-wardrobe-page/{id}")),
                index,
                item_count,
                has_more,
                is_loading_more,
                WARDROBE_PAGE_THRESHOLD,
                |cx: &mut App| {
                    GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_wardrobe(cx));
                },
            )
            .into_any_element()
    }
}

impl Default for GiftWardrobeSheetContent {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for GiftWardrobeSheetContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let interactions = GiftsInteractionsStore::shared(cx);
        let kind = interactions.read(cx).wardrobe_kind;
        let reader = window.use_keyed_state("gift-wardrobe-scroll-reader", cx, |_, _| GiftSheetScrollReader::new());
        reader.update(cx, |reader, _| reader.frame(window));
        let handle = reader.read(cx).handle();
        let scroll = handle.clone();
        let state = window.use_keyed_state("gift-wardrobe-sheet", cx, |_, cx: &mut Context<GiftWardrobeSheetState>| GiftWardrobeSheetState {
            me: GiftPeople::me(cx),
            reveal_id: None,
            menu_instance_id: None,
            grid_bounds: Rc::new(Cell::new(None)),
            revealed_cell: Rc::new(RefCell::new(None)),
            scroll,
        });

        let worn = kind.and_then(|kind| {
            GiftsServicesStore::shared(cx)
                .read(cx)
                .wardrobe(kind)
                .items
                .iter()
                .find(|instance| instance.is_equipped())
                .map(|instance| instance.id.clone())
        });
        let (change_state, change_reader) = (state.clone(), reader);
        on_change("gift-wardrobe-worn", worn, false, window, cx, move |_, worn, window, cx| {
            let Some(worn) = worn.clone() else {
                return;
            };
            if change_state.read(cx).reveal_id.as_deref() != Some(worn.as_str()) {
                return;
            }
            change_state.update(cx, |state, _| state.reveal_id = None);
            let Some(kind) = kind else {
                return;
            };
            window.defer(cx, move |window, cx| {
                window.defer(cx, move |_, cx| {
                    Self::reveal(&worn, kind, &change_state, &change_reader, cx);
                    change_state.read(cx).revealed_cell.borrow_mut().take();
                });
            });
        });

        let title = kind.and_then(|kind| kind.appearance_title_key()).map(tr).unwrap_or_default();
        let closer = interactions;
        let header = SheetFloatingHeader::new("gift-wardrobe-header", title, move |_, cx| {
            closer.update(cx, |store, cx| store.close_wardrobe(cx));
        });
        div()
            .relative()
            .size_full()
            .child(
                scroll_view("gift-wardrobe-scroll").shows_indicators(false).scroll_handle(handle).child(
                    vstack(0.0)
                        .w_full()
                        .pb(px(24.0))
                        .child(div().w_full().flex_none().h(px(SheetFloatingHeaderMetrics::HEIGHT)))
                        .children(kind.map(|kind| Self::shelf(kind, &state, cx))),
                ),
            )
            .child(div().absolute().top_0().left_0().right_0().child(header))
    }
}

/// one of the viewer cosmetics drawn on the viewer over its rolled backdrop with its name
/// under it and a check when worn
#[derive(IntoElement)]
struct GiftWardrobeCell {
    instance: Option<RiseGiftInstanceDTO>,
    art: Option<GiftWardrobeCellArt>,
    kind: RiseGiftItemKind,
    name: String,
}

impl GiftWardrobeCell {
    fn new(instance: RiseGiftInstanceDTO, _person: Option<GiftRecipient>, art: GiftWardrobeCellArt) -> Self {
        Self { kind: instance.kind(), name: instance.name.clone(), instance: Some(instance), art: Some(art) }
    }

    /// hidden cell that gives the grid its row height, nothing plays in it
    fn template(kind: RiseGiftItemKind, name: String, _person: Option<GiftRecipient>) -> Self {
        Self { instance: None, art: None, kind, name }
    }
}

/// what one grid cell is built from, a change renders its cached view again
#[derive(Clone, PartialEq)]
struct GiftWardrobeCellProps {
    index: i64,
    instance: RiseGiftInstanceDTO,
    person: Option<GiftRecipient>,
    item_count: i64,
    has_more: bool,
    is_loading_more: bool,
    is_menu_presented: bool,
    is_revealing: bool,
}

/// one grid cell as its own cached view
/// a wheel step renders the sheet that hosts the grid and each cell only moves and replays
/// its last frame at the new origin
struct GiftWardrobeCellView {
    props: Option<GiftWardrobeCellProps>,
    state: Option<Entity<GiftWardrobeSheetState>>,
}

impl GiftWardrobeCellView {
    fn new() -> Self {
        Self { props: None, state: None }
    }

    fn set_props(&mut self, props: GiftWardrobeCellProps, state: Entity<GiftWardrobeSheetState>) -> bool {
        self.state = Some(state);
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        self.props = Some(props);
        true
    }
}

impl Render for GiftWardrobeCellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cell = self
            .props
            .clone()
            .zip(self.state.clone())
            .map(|(props, state)| GiftWardrobeSheetContent::cell(props, state, window, cx));
        div().absolute().top_0().left_0().right_0().bottom_0().children(cell)
    }
}

/// the two costly layers of a cell, each its own cached view
/// the cosmetic on the viewer and the rolled backdrop with its pattern
/// wheel steps rerendered the host and every visible cell built both again
struct GiftWardrobeCellArt {
    preview: Entity<GiftWardrobePreviewView>,
    backdrop: Entity<GiftWardrobeBackdropView>,
}

/// preview and ground sizes the cells of one kind were last laid out at
/// every cell of a kind has the same box so a scrolled in cell draws at it in its first render
#[derive(Clone, Copy, Default)]
struct GiftWardrobeCellSeeds {
    preview: Option<Size<f32>>,
    backdrop: Option<Size<f32>>,
}

thread_local! {
    static GIFT_WARDROBE_CELL_SEEDS: RefCell<[GiftWardrobeCellSeeds; 4]> = RefCell::new([GiftWardrobeCellSeeds::default(); 4]);
}

impl GiftWardrobeCellSeeds {
    fn slot(kind: RiseGiftItemKind) -> usize {
        match kind {
            RiseGiftItemKind::Item => 0,
            RiseGiftItemKind::AvatarFrame => 1,
            RiseGiftItemKind::Nameplate => 2,
            RiseGiftItemKind::ProfileEffect => 3,
        }
    }

    fn of(kind: RiseGiftItemKind) -> Self {
        GIFT_WARDROBE_CELL_SEEDS.with(|seeds| seeds.borrow()[Self::slot(kind)])
    }

    fn update(kind: RiseGiftItemKind, apply: impl FnOnce(&mut Self)) {
        GIFT_WARDROBE_CELL_SEEDS.with(|seeds| apply(&mut seeds.borrow_mut()[Self::slot(kind)]));
    }

    fn size_of(bounds: Bounds<Pixels>) -> Size<f32> {
        Size { width: f32::from(bounds.size.width), height: f32::from(bounds.size.height) }
    }
}

impl GiftWardrobeCellArt {
    fn new(instance: &RiseGiftInstanceDTO, person: Option<GiftRecipient>, window: &mut Window, cx: &mut App) -> Self {
        let id = instance.id.clone();
        let seeds = GiftWardrobeCellSeeds::of(instance.kind());
        let preview = window.use_keyed_state(
            SharedString::from(format!("gift-wardrobe-preview-view/{id}")),
            cx,
            |_, cx: &mut Context<GiftWardrobePreviewView>| {
                GiftWardrobePreviewView::new(instance.clone(), person.clone(), seeds.preview, cx)
            },
        );
        preview.update(cx, |view, cx| view.set(instance, person, cx));
        let backdrop = window.use_keyed_state(
            SharedString::from(format!("gift-wardrobe-backdrop-view/{id}")),
            cx,
            |_, cx: &mut Context<GiftWardrobeBackdropView>| {
                GiftWardrobeBackdropView::new(instance.clone(), seeds.backdrop, cx)
            },
        );
        backdrop.update(cx, |view, cx| view.set(instance, cx));
        Self { preview, backdrop }
    }

    fn layer_style() -> StyleRefinement {
        StyleRefinement::default().absolute().top_0().left_0().right_0().bottom_0()
    }
}

struct GiftWardrobePreviewView {
    instance: RiseGiftInstanceDTO,
    person: Option<GiftRecipient>,
    size_hint: Option<Size<f32>>,
    _theme: Subscription,
}

impl GiftWardrobePreviewView {
    fn new(
        instance: RiseGiftInstanceDTO,
        person: Option<GiftRecipient>,
        size_hint: Option<Size<f32>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self { instance, person, size_hint, _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()) }
    }

    fn set(&mut self, instance: &RiseGiftInstanceDTO, person: Option<GiftRecipient>, cx: &mut Context<Self>) {
        if self.instance != *instance || self.person != person {
            self.instance = instance.clone();
            self.person = person;
            cx.notify();
        }
    }
}

impl Render for GiftWardrobePreviewView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let instance = &self.instance;
        div().size_full().child(
            GiftCosmeticPreview::new(
                SharedString::from(format!("gift-wardrobe-preview/{}", instance.id)),
                instance.kind(),
                self.person.clone(),
                instance.model().and_then(|model| model.media.clone()),
            )
            .effect(instance.effect().and_then(|effect| effect.media.clone()))
            .size_hint(self.size_hint),
        )
    }
}

struct GiftWardrobeBackdropView {
    instance: RiseGiftInstanceDTO,
    size_hint: Option<Size<f32>>,
    _theme: Subscription,
}

impl GiftWardrobeBackdropView {
    fn new(instance: RiseGiftInstanceDTO, size_hint: Option<Size<f32>>, cx: &mut Context<Self>) -> Self {
        let theme_store = ThemeStore::shared(cx);
        Self { instance, size_hint, _theme: cx.observe(&theme_store, |_, _, cx| cx.notify()) }
    }

    fn set(&mut self, instance: &RiseGiftInstanceDTO, cx: &mut Context<Self>) {
        if self.instance != *instance {
            self.instance = instance.clone();
            cx.notify();
        }
    }
}

impl Render for GiftWardrobeBackdropView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let instance = &self.instance;
        div().size_full().child(
            GiftBackdropSurface::new(
                SharedString::from(format!("gift-wardrobe-backdrop/{}", instance.id)),
                instance.backdrop().cloned(),
            )
            .pattern(instance.pattern().cloned())
            .rarity(instance.rarity_class())
            .pattern_strength(GiftPatternLayer::COSMETIC_STRENGTH)
            .size_hint(self.size_hint),
        )
    }
}

impl RenderOnce for GiftWardrobeCell {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = ThemeStore::shared(cx);
        let theme = store.read(cx).current_theme().clone();
        let shape = Shape::continuous(20.0);
        let instance = self.instance;
        let on_rolled = instance
            .as_ref()
            .is_some_and(|instance| GiftBackdropColors::resolve(instance.backdrop(), instance.rarity_class()).is_rolled);
        let is_equipped = instance.as_ref().is_some_and(|instance| instance.is_equipped());

        let frame = match self.kind {
            RiseGiftItemKind::Nameplate => div().w_full().h(px(44.0)),
            RiseGiftItemKind::ProfileEffect => div().w_full().aspect_ratio(GiftCosmeticGeometry::BANNER_ASPECT),
            RiseGiftItemKind::Item | RiseGiftItemKind::AvatarFrame => div().w_full().aspect_ratio(1.0),
        };
        let inset = match self.kind {
            RiseGiftItemKind::Nameplate | RiseGiftItemKind::ProfileEffect => 8.0,
            RiseGiftItemKind::Item | RiseGiftItemKind::AvatarFrame => 6.0,
        };
        let (preview_view, backdrop_view) = match self.art {
            Some(art) => (Some(art.preview), Some(art.backdrop)),
            None => (None, None),
        };
        let kind = self.kind;
        let seed_key = instance.as_ref().map_or_else(|| "template".to_string(), |instance| instance.id.clone());
        let preview = frame
            .relative()
            .children(preview_view.map(|view| view.cached(GiftWardrobeCellArt::layer_style()).replay_when_moved()))
            .child(measure(SharedString::from(format!("gift-wardrobe-preview-seed/{seed_key}")), move |bounds, _, _| {
                GiftWardrobeCellSeeds::update(kind, |seeds| seeds.preview = Some(GiftWardrobeCellSeeds::size_of(bounds)));
            }));

        shape
            .round(div().relative().w_full().overflow_hidden())
            .child(measure(SharedString::from(format!("gift-wardrobe-backdrop-seed/{seed_key}")), move |bounds, _, _| {
                GiftWardrobeCellSeeds::update(kind, |seeds| seeds.backdrop = Some(GiftWardrobeCellSeeds::size_of(bounds)));
            }))
            .children(backdrop_view.map(|view| {
                view.cached(GiftWardrobeCellArt::layer_style()).replay_when_moved()
            }))
            .child(
                vstack(8.0)
                    .w_full()
                    .pt(px(14.0))
                    .pb(px(12.0))
                    .child(div().w_full().px(px(inset)).child(preview))
                    .child(
                        div().w_full().px(px(8.0)).child(
                            MainText::new(self.name)
                                .px(13.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(if on_rolled { white() } else { theme.text.primary })
                                .line_limit(1),
                        ),
                    ),
            )
            .when(is_equipped, |cell| {
                cell.child(div().absolute().top(px(8.0)).right(px(8.0)).child(GiftEquippedBadge::new().compact(true))).child(
                    shape.round(div().absolute().inset_0()).stroke_border(theme.text.primary, 2.0),
                )
            })
    }
}

/// under the picker, take the worn one off or buy another in the shop
#[derive(IntoElement)]
pub struct GiftWardrobeSheetFooter;

impl GiftWardrobeSheetFooter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GiftWardrobeSheetFooter {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for GiftWardrobeSheetFooter {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let interactions = GiftsInteractionsStore::shared(cx);
        let (wardrobe_kind, is_equipping) = {
            let store = interactions.read(cx);
            (store.wardrobe_kind, store.is_equipping)
        };
        let Some(kind) = wardrobe_kind else {
            return gpui::Empty.into_any_element();
        };
        let theme_store = ThemeStore::shared(cx);
        let theme = theme_store.read(cx).current_theme().clone();
        let own_appearance =
            GiftAppearanceServicesStore::shared(cx).update(cx, |services, cx| services.own_appearance(cx));
        let services = GiftsServicesStore::shared(cx);
        let services = services.read(cx);
        let shelf = services.wardrobe(kind);
        let worn: Option<RiseGiftInstanceDTO> = shelf
            .items
            .iter()
            .find(|instance| instance.is_equipped())
            .or_else(|| services.appearance().instance(own_appearance.as_ref().and_then(|appearance| appearance.worn(kind))))
            .cloned();
        let buy_key = if shelf.items.is_empty() { "gift_market_title_self" } else { "gift_market_buy_more" };

        let taker = interactions.clone();
        let buyer = interactions;
        hstack(10.0)
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .children(worn.map(|worn| {
                div().flex_1().min_w(px(0.0)).child(
                    ButtonUi::new("gift-wardrobe-take-off")
                        .disabled(is_equipping)
                        .background_color(theme.bg._200)
                        .on_press(move |_, cx| {
                            let worn = worn.clone();
                            taker.update(cx, |store, cx| store.set_equipped(worn, false, None, cx));
                        })
                        .child(
                            MainText::new(tr("gift_equip_take_off"))
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.text.primary),
                        ),
                )
            }))
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    ButtonUi::new("gift-wardrobe-buy")
                        .disabled(is_equipping)
                        .on_press(move |_, cx| buyer.update(cx, |store, cx| store.buy_from_wardrobe(cx)))
                        .child(MainText::new(tr(buy_key)).px(16.0).font_weight(FontWeight::SEMIBOLD).color(white())),
                ),
            )
            .into_any_element()
    }
}
