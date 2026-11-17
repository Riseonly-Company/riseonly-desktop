use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, ElementId, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, ObjectFit, PathBuilder, Pixels,
    Point, RenderImage, ScrollWheelEvent, SharedString, Stateful, Subscription, Window, black, canvas, fill, img, point, size,
    uniform_list, white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::date_format::date_formatter::Style;
use rise_i18n::{AppDateFormatter, FormatArg, format_printf, tr, tr_plural};
use rise_theme::AppTheme;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize, LoaderUiType};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{ease_out, linear};
use rise_ui::swiftui::prelude::*;
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::core::lib::canon_format::RiseFormat;
use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers, MediaViewerPresentationState, media_viewer_presentation,
};
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerAccessoryAction, MediaViewerChrome, MediaViewerGalleryItem, MediaViewerGalleryItemKind,
    MediaViewerHeroSkin,
};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::{
    HoldContextMenuActionsProvider, HoldContextMenuContainer, HoldContextMenuSourceContent,
};
use crate::modules::map::engine::core::rise_map_photo_upload::MapPhotoURL;
use crate::modules::map::engine::rise_map_engine_models::RiseMapLoadState;
use crate::modules::map::engine::rise_map_photos_repository::RiseMapOpenGridSnapshot;
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::shared::map_gallery_export::MapGalleryThumbnail;
use crate::modules::map::shared::map_gallery_scout::MapGalleryDraft;
use crate::modules::map::shared::map_grid_uploads::{
    MapGridPreview, MapGridUpload, MapGridUploadCenter, MapUploadProgressBox,
};
use crate::modules::map::stores::map_actions::map_photos_types::{MapMediaKind, MapPhoto, MapPhotoGrid};

pub type MapPhotoGridCardAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type MapPhotoGridCardConfirmed = Rc<dyn Fn(&mut App)>;
pub type MapPhotoGridCardDeletePhoto = Rc<dyn Fn(String, &mut App)>;
pub type MapPhotoGridCardPublishDrafts = Rc<dyn Fn(Vec<MapGalleryDraft>, &mut App)>;
pub type MapPhotoGridCardDiscardDrafts = Rc<dyn Fn(Vec<String>, &mut App)>;
pub type MapPhotoGridCardCancelUpload = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// grid of every photo and short video taken at one map spot, newest first
/// click opens the chat media viewer with a hero transition
/// secondary click offers removal
#[derive(IntoElement)]
pub struct MapPhotoGridCard {
    pub grid: MapPhotoGrid,
    pub open: Option<RiseMapOpenGridSnapshot>,
    pub drafts: Vec<MapGalleryDraft>,
    pub publishing_drafts: HashSet<String>,
    pub uploads: Vec<MapGridUpload>,
    pub is_own: bool,
    pub upload_progress: Option<f64>,
    pub on_add: MapPhotoGridCardAction,
    pub on_delete_photo: MapPhotoGridCardDeletePhoto,
    pub on_delete_grid: MapPhotoGridCardConfirmed,
    pub on_publish_drafts: MapPhotoGridCardPublishDrafts,
    pub on_discard_drafts: MapPhotoGridCardDiscardDrafts,
    pub on_discard_all_drafts: MapPhotoGridCardConfirmed,
    pub on_cancel_upload: MapPhotoGridCardCancelUpload,
    pub on_load_more: MapPhotoGridCardAction,
    pub on_retry: MapPhotoGridCardAction,
    pub on_close: MapPhotoGridCardAction,
    pub on_edit: Option<MapPhotoGridCardAction>,
    pub docked: bool,
}

/// the cell the delete key acts on and the viewer whose hero state the cells follow
struct MapPhotoGridCardState {
    selected_slot: Option<String>,
    is_grid_menu_presented: bool,
    focus_handle: FocusHandle,
    grid_width: Option<f32>,
    viewer_presentation_id: Option<Uuid>,
    viewer_load_requested_at: Option<usize>,
    _viewer_observation: Subscription,
}

#[derive(Clone)]
struct MapPhotoGridCardFrame {
    theme: AppTheme,
    presenter: Entity<MediaViewerPresentationState>,
    state: Entity<MapPhotoGridCardState>,
    photos: Rc<Vec<MapPhoto>>,
}

impl MapPhotoGridCard {
    pub const GRID_HEIGHT: f32 = 252.0;

    const CORNER_RADIUS: f32 = 28.0;
    const CELL_SPACING: f32 = 2.0;
    const COLUMNS: usize = 3;
    const TAIL_FAILURE_HEIGHT: f32 = 44.0;
    const DELETE_ACTION_ID: &'static str = "map.photo.delete";
    const PUBLISH_DRAFT_ACTION_ID: &'static str = "map.draft.publish";
    const DISCARD_DRAFT_ACTION_ID: &'static str = "map.draft.discard";

    fn id_for(&self, part: &str) -> ElementId {
        ElementId::Name(format!("map-photo-grid-card-{}-{part}", self.grid.id).into())
    }

    fn matching_open(&self) -> Option<&RiseMapOpenGridSnapshot> {
        self.open.as_ref().filter(|open| open.grid_id == self.grid.id)
    }

    fn waiting_drafts(&self) -> Vec<MapGalleryDraft> {
        self.drafts.iter().filter(|draft| !self.publishing_drafts.contains(&draft.id)).cloned().collect()
    }

    fn header(self: &Rc<Self>, frame: &MapPhotoGridCardFrame, waiting: Vec<MapGalleryDraft>) -> AnyElement {
        let theme = &frame.theme;
        let title = if self.grid.is_draft {
            tr_plural("map_drafts_count", i64::try_from(self.drafts.len()).unwrap_or(i64::MAX))
        } else {
            self.grid.display_title()
        };
        let subtitle = if self.grid.is_draft {
            tr("map_drafts_hint")
        } else if self.grid.description.is_empty() {
            Self::date_text(self.grid.updated_at)
        } else {
            self.grid.description.clone()
        };

        let mut row = hstack(8.0)
            .w_full()
            .px(px(4.0))
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .min_w_0()
                    .child(MainText::new(title).px(16.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                    .child(MainText::new(subtitle).px(13.0).color(theme.text.secondary).number_of_lines(1)),
            )
            .child(spacer_min_w(8.0));
        if self.is_own {
            if !self.grid.is_draft {
                row = row.child(self.round_button(theme, "plus", self.upload_progress));
            }
            row = row.child(self.more_menu(frame, waiting));
        }
        let on_close = self.on_close.clone();
        row.child(
            plain(self.id_for("close"))
                .on_press(move |window, cx| on_close(window, cx))
                .child(self.round_label(theme, "xmark", "close-label", tr("map_card_close"))),
        )
        .into_any_element()
    }

    fn more_menu(self: &Rc<Self>, frame: &MapPhotoGridCardFrame, waiting: Vec<MapGalleryDraft>) -> AnyElement {
        let is_grid_menu_presented = Binding::entity(
            frame.state.clone(),
            |state: &MapPhotoGridCardState| state.is_grid_menu_presented,
            |state: &mut MapPhotoGridCardState, value: bool, cx| {
                state.is_grid_menu_presented = value;
                cx.notify();
            },
        );
        let opener = frame.state.clone();
        let actions: HoldContextMenuPresentationActions = {
            let card = self.clone();
            let waiting = waiting.clone();
            Rc::new(move |_: &App| card.grid_menu_actions(&waiting))
        };
        let on_action: HoldContextMenuPresentationOnAction = {
            let card = self.clone();
            Rc::new(move |id: String, window: &mut Window, cx: &mut App| card.grid_menu_action(&id, &waiting, window, cx))
        };
        let source: HoldContextMenuTapSource = {
            let card = self.clone();
            let theme = frame.theme.clone();
            Rc::new(move |_: &mut Window, _: &mut App| {
                card.round_label(&theme, "ellipsis", "more-source", tr("map_grid_more")).into_any_element()
            })
        };
        plain(self.id_for("more"))
            .on_press(move |_, cx| {
                opener.update(cx, |state, cx| {
                    state.is_grid_menu_presented = true;
                    cx.notify();
                });
            })
            .child(self.round_label(&frame.theme, "ellipsis", "more-label", tr("map_grid_more")))
            .tap_context_menu(
                self.id_for("more-menu"),
                is_grid_menu_presented,
                HoldContextMenuAlignment::Trailing,
                false,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            )
            .guide_anchor((!self.grid.is_draft).then_some(RiseGuideAnchor::MapGridMore))
            .into_any_element()
    }

    fn grid_menu_actions(&self, waiting: &[MapGalleryDraft]) -> Vec<HoldContextMenuAction> {
        let mut actions: Vec<HoldContextMenuAction> = Vec::new();
        if !self.grid.is_draft && self.on_edit.is_some() {
            actions.push(HoldContextMenuAction::new("edit", tr("edit_action")).system_image("pencil"));
        }
        if !waiting.is_empty() {
            actions.push(
                HoldContextMenuAction::new("publish_drafts", tr("map_drafts_add_all")).system_image("checkmark.circle"),
            );
            actions.push(
                HoldContextMenuAction::new("clear_drafts", tr("map_drafts_clear"))
                    .system_image("xmark.bin")
                    .destructive(true),
            );
        }
        actions.push(
            HoldContextMenuAction::new("clear_all_drafts", tr("map_drafts_clear_all"))
                .system_image("trash.slash")
                .destructive(true),
        );
        if !self.grid.is_draft {
            actions.push(
                HoldContextMenuAction::new("delete_grid", tr("map_grid_delete")).system_image("trash").destructive(true),
            );
        }
        actions
    }

    fn grid_menu_action(&self, id: &str, waiting: &[MapGalleryDraft], window: &mut Window, cx: &mut App) {
        match id {
            "edit" => {
                if let Some(on_edit) = self.on_edit.clone() {
                    on_edit(window, cx);
                }
            }
            "publish_drafts" => (self.on_publish_drafts)(waiting.to_vec(), cx),
            "clear_drafts" => {
                let on_discard_drafts = self.on_discard_drafts.clone();
                let ids: Vec<String> = waiting.iter().map(|draft| draft.id.clone()).collect();
                NativeConfirmAlert::present_delete(
                    &tr("map_drafts_clear_title"),
                    &tr("map_drafts_clear_text"),
                    Some(tr("map_drafts_clear")),
                    None,
                    false,
                    None,
                    None,
                    move |cx| on_discard_drafts(ids, cx),
                    cx,
                );
            }
            "clear_all_drafts" => {
                let on_discard_all_drafts = self.on_discard_all_drafts.clone();
                NativeConfirmAlert::present_delete(
                    &tr("map_drafts_clear_all_title"),
                    &tr("map_drafts_clear_all_text"),
                    Some(tr("map_drafts_clear_all")),
                    None,
                    false,
                    None,
                    None,
                    move |cx| on_discard_all_drafts(cx),
                    cx,
                );
            }
            "delete_grid" => {
                let on_delete_grid = self.on_delete_grid.clone();
                NativeConfirmAlert::present_delete(
                    &tr("map_grid_delete_title"),
                    &tr("map_grid_delete_text"),
                    Some(tr("map_grid_delete")),
                    None,
                    false,
                    None,
                    None,
                    move |cx| on_delete_grid(cx),
                    cx,
                );
            }
            _ => {}
        }
    }

    fn content(self: &Rc<Self>, frame: &MapPhotoGridCardFrame, cx: &mut App) -> AnyElement {
        let theme = &frame.theme;
        let state = if self.grid.is_draft {
            RiseMapLoadState::Loaded
        } else {
            self.matching_open().map_or(RiseMapLoadState::Loading, |open| open.load_state)
        };
        let fills = self.docked;
        let is_empty = frame.photos.is_empty() && self.drafts.is_empty() && self.uploads.is_empty();

        if is_empty && state == RiseMapLoadState::Failed {
            let on_retry = self.on_retry.clone();
            return vstack(10.0)
                .w_full()
                .when(fills, |column| column.flex_1().min_h_0())
                .when(!fills, |column| column.h(px(Self::GRID_HEIGHT)))
                .justify_center()
                .child(MainText::new(tr("map_grid_failed")).px(14.0).color(theme.text.secondary))
                .child(
                    ButtonUi::new(self.id_for("retry"))
                        .height(38.0)
                        .fit_content(true)
                        .padding_horizontal(16.0)
                        .on_press(move |window, cx| on_retry(window, cx))
                        .child(
                            MainText::new(tr("common_retry"))
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(white())
                                .number_of_lines(1),
                        ),
                )
                .into_any_element();
        }
        let grid_width = frame.state.read(cx).grid_width;
        let columns = grid_width.map_or(Self::COLUMNS, Self::columns);
        if is_empty && state != RiseMapLoadState::Loaded {
            let count = self.grid.photo_count.clamp(3, 6);
            let cells = (0..count).map(|_| {
                div().w_full().aspect_square().child(SkeletonBlock::new(None::<f32>, None::<f32>, 0.0)).skeleton_shimmer(None)
            });
            return div()
                .relative()
                .w_full()
                .when(fills, |cells| cells.flex_1().min_h_0())
                .when(!fills, |cells| cells.h(px(Self::GRID_HEIGHT)))
                .overflow_hidden()
                .child(div().w_full().grid().grid_cols(columns as u16).gap(px(Self::CELL_SPACING)).children(cells))
                .child(self.width_reader(&frame))
                .into_any_element();
        }

        let tail_failed = state == RiseMapLoadState::Failed;
        let slots = Rc::new(self.slots(&frame.photos, cx));
        let rows = slots.len().div_ceil(columns);
        Self::trace_x1221_slots(&slots, columns, rows, grid_width, &frame, cx);
        let card = self.clone();
        let list_frame = frame.clone();
        let list_slots = slots.clone();
        let row_height = grid_width.map_or(Self::GRID_HEIGHT, |width| Self::row_height(width, columns));
        let list = uniform_list(self.id_for("cells"), rows, move |range, _window, cx| {
            range
                .map(|row| {
                    let start = row * columns;
                    let end = (start + columns).min(list_slots.len());
                    let mut cells = hstack(Self::CELL_SPACING)
                        .align(VerticalAlignment::Top)
                        .w_full()
                        .h(px(row_height))
                        .pb(px(Self::CELL_SPACING));
                    for slot in &list_slots[start..end] {
                        cells = cells.child(card.slot_cell(slot, &list_frame, list_slots.last(), cx));
                    }
                    for _ in end..start + columns {
                        cells = cells.child(div().flex_1());
                    }
                    cells
                })
                .collect::<Vec<_>>()
        })
        .w_full()
        .when(fills || tail_failed, |list| list.flex_1().min_h_0())
        .when(!fills && !tail_failed, |list| list.h(px(Self::GRID_HEIGHT)));

        let card = self.clone();
        let key_frame = frame.clone();
        let focus_handle = frame.state.read(cx).focus_handle.clone();
        div()
            .id(self.id_for("cells-focus"))
            .relative()
            .w_full()
            .when(fills, |cells| cells.flex_1().min_h_0().flex().flex_col())
            .when(!fills, |cells| cells.h(px(Self::GRID_HEIGHT)))
            .when(!fills && tail_failed, |cells| cells.flex().flex_col())
            .overflow_hidden()
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                card.delete_key_down(event, &slots, &key_frame, cx);
            })
            .child(list)
            .when(tail_failed, |cells| cells.child(self.tail_failure(theme)))
            .child(self.width_reader(&frame))
            .into_any_element()
    }

    fn tail_failure(&self, theme: &AppTheme) -> AnyElement {
        let on_load_more = self.on_load_more.clone();
        hstack(10.0)
            .w_full()
            .flex_none()
            .h(px(Self::TAIL_FAILURE_HEIGHT))
            .justify_center()
            .child(MainText::new(tr("map_grid_failed")).px(14.0).color(theme.text.secondary).number_of_lines(1))
            .child(
                ButtonUi::new(self.id_for("tail-retry"))
                    .height(30.0)
                    .fit_content(true)
                    .padding_horizontal(12.0)
                    .on_press(move |window, cx| on_load_more(window, cx))
                    .child(
                        MainText::new(tr("common_retry"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white())
                            .number_of_lines(1),
                    ),
            )
            .into_any_element()
    }

    fn trace_x1221_slots(
        slots: &[MapGridSlot],
        columns: usize,
        rows: usize,
        grid_width: Option<f32>,
        frame: &MapPhotoGridCardFrame,
        cx: &mut App,
    ) {
        thread_local! {
            static LAST: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
        }
        let center = MapGridUploadCenter::shared(cx);
        let center = center.read(cx);
        let kinds: Vec<String> = slots
            .iter()
            .map(|slot| {
                let registry_id = slot
                    .photo
                    .as_ref()
                    .map(Self::registry_id)
                    .or_else(|| slot.draft.as_ref().map(Self::draft_registry_id))
                    .unwrap_or_else(|| format!("map-upload-{}", slot.id));
                let cell_frame = MediaCellScreenFrameRegistry::frame(&registry_id).map(|bounds| {
                    (
                        f32::from(bounds.origin.x).round(),
                        f32::from(bounds.origin.y).round(),
                        f32::from(bounds.size.width).round(),
                        f32::from(bounds.size.height).round(),
                    )
                });
                format!(
                    "{}[frame={cell_frame:?} photo={:?} pending={:?} url={:?} draft={} upload={} preview={}]",
                    slot.id,
                    slot.photo.as_ref().map(|photo| photo.id.as_str()),
                    slot.photo.as_ref().map(|photo| photo.is_pending),
                    slot.photo.as_ref().map(|photo| !photo.display_url().is_empty()),
                    slot.draft.is_some(),
                    slot.upload.is_some(),
                    center.preview(&slot.id).is_some(),
                )
            })
            .collect();
        let selected = frame.state.read(cx).selected_slot.clone();
        let viewer = {
            let presenter = frame.presenter.read(cx);
            format!(
                "presented={} lift={:?} cell_hidden={}",
                presenter.is_presented(),
                presenter.hero_lift_id(),
                presenter.bubble_cell_hidden()
            )
        };
        let line = format!(
            "slots={} columns={columns} rows={rows} width={grid_width:?} photos={} selected={selected:?} {viewer} {}",
            slots.len(),
            frame.photos.len(),
            kinds.join(" ")
        );
        let changed = LAST.with(|last| {
            let mut last = last.borrow_mut();
            if *last == line {
                return false;
            }
            last.clone_from(&line);
            true
        });
        if changed {
            tracing::warn!(target: "riseonly::X1221", "grid {line}");
        }
    }

    fn width_reader(&self, frame: &MapPhotoGridCardFrame) -> AnyElement {
        let width_state = frame.state.clone();
        measure(self.id_for("cells-width"), move |bounds, _, cx| {
            let width = f32::from(bounds.size.width);
            width_state.update(cx, |state, cx| {
                if state.grid_width != Some(width) {
                    state.grid_width = Some(width);
                    cx.notify();
                }
            });
        })
        .into_any_element()
    }

    fn columns(width: f32) -> usize {
        let largest_cell = (Self::GRID_HEIGHT - Self::CELL_SPACING) / 2.0;
        let fitting = ((width + Self::CELL_SPACING) / (largest_cell + Self::CELL_SPACING)).ceil();
        if fitting.is_finite() && fitting > Self::COLUMNS as f32 { fitting as usize } else { Self::COLUMNS }
    }

    /// one row of equal squares plus the spacing under it
    fn row_height(width: f32, columns: usize) -> f32 {
        let gaps = Self::CELL_SPACING * (columns as f32 - 1.0);
        ((width - gaps) / columns as f32).max(1.0) + Self::CELL_SPACING
    }

    fn delete_key_down(
        self: &Rc<Self>,
        event: &KeyDownEvent,
        slots: &[MapGridSlot],
        frame: &MapPhotoGridCardFrame,
        cx: &mut App,
    ) {
        if !matches!(event.keystroke.key.as_str(), "delete" | "backspace") {
            return;
        }
        let selected = frame.state.read(cx).selected_slot.clone();
        let Some(slot) = selected.and_then(|selected| slots.iter().find(|slot| slot.id == selected)) else {
            return;
        };
        let is_sending = self.is_sending(slot);
        if is_sending || slot.photo.as_ref().is_some_and(|photo| photo.is_pending) {
            return;
        }
        let actions = self.hold_actions(slot.photo.as_ref(), slot.draft.as_ref(), is_sending);
        if let Some(action) = actions.iter().find(|action| action.destructive) {
            self.perform_slot_action(&action.id, slot, frame, cx);
            cx.stop_propagation();
        }
    }

    fn is_sending(&self, slot: &MapGridSlot) -> bool {
        slot.upload.is_some() || slot.draft.as_ref().is_some_and(|draft| self.publishing_drafts.contains(&draft.id))
    }

    /// picked file cells first in pick order, then other uploads and drafts
    /// then photos newest first
    /// a photo keeps the key of the draft it came from and wins while both coexist
    fn slots(&self, photos: &[MapPhoto], cx: &mut App) -> Vec<MapGridSlot> {
        let center = MapGridUploadCenter::shared(cx);
        let center = center.read(cx);
        let mut seen: HashSet<String> = HashSet::new();
        let mut photo_slots: Vec<MapGridSlot> = Vec::with_capacity(photos.len());
        for photo in photos {
            let id = center.slot_id(&photo.id);
            if !seen.insert(id.clone()) {
                continue;
            }
            photo_slots.push(MapGridSlot { id, photo: Some(photo.clone()), draft: None, upload: None });
        }
        let mut waiting: Vec<MapGridSlot> = Vec::new();
        for upload in &self.uploads {
            if seen.insert(upload.id.clone()) {
                waiting.push(MapGridSlot {
                    id: upload.id.clone(),
                    photo: None,
                    draft: None,
                    upload: Some(upload.clone()),
                });
            }
        }
        for draft in &self.drafts {
            if seen.insert(draft.id.clone()) {
                waiting.push(MapGridSlot {
                    id: draft.id.clone(),
                    photo: None,
                    draft: Some(draft.clone()),
                    upload: None,
                });
            }
        }
        waiting.extend(photo_slots);
        let pinned = center.pinned();
        if pinned.is_empty() {
            return waiting;
        }
        let rank: HashMap<&str, usize> = pinned.iter().enumerate().map(|(index, id)| (id.as_str(), index)).collect();
        let (mut front, rest): (Vec<MapGridSlot>, Vec<MapGridSlot>) =
            waiting.into_iter().partition(|slot| rank.contains_key(slot.id.as_str()));
        front.sort_by_key(|slot| rank[slot.id.as_str()]);
        front.extend(rest);
        front
    }

    fn slot_cell(
        self: &Rc<Self>,
        slot: &MapGridSlot,
        frame: &MapPhotoGridCardFrame,
        last_slot: Option<&MapGridSlot>,
        cx: &mut App,
    ) -> AnyElement {
        let is_sending = self.is_sending(slot);
        let registry_id = slot
            .photo
            .as_ref()
            .map(Self::registry_id)
            .or_else(|| slot.draft.as_ref().map(Self::draft_registry_id))
            .unwrap_or_else(|| format!("map-upload-{}", slot.id));
        let is_lifted = {
            let presenter = frame.presenter.read(cx);
            presenter.is_presented()
                && presenter.hero_lift_id() == Some(registry_id.as_str())
                && presenter.bubble_cell_hidden()
        };
        let center = MapGridUploadCenter::shared(cx);
        let preview = center
            .read(cx)
            .preview(&slot.id)
            .or_else(|| slot.draft.as_ref().map(|draft| MapGridPreview::Asset(draft.id.clone())));
        let is_video = match (&slot.photo, &slot.draft, &slot.upload) {
            (Some(photo), _, _) => photo.media_kind == MapMediaKind::Video,
            (None, Some(draft), _) => draft.media_kind == MapMediaKind::Video,
            (None, None, Some(upload)) => upload.is_video,
            (None, None, None) => false,
        };
        let duration_ms = slot
            .photo
            .as_ref()
            .map(|photo| photo.duration_ms)
            .or_else(|| slot.draft.as_ref().map(|draft| draft.duration_ms))
            .or_else(|| slot.upload.as_ref().map(|upload| upload.duration_ms))
            .unwrap_or(0);

        let actions_provider: HoldContextMenuActionsProvider = {
            let actions = self.hold_actions(slot.photo.as_ref(), slot.draft.as_ref(), is_sending);
            Rc::new(move |_: &App| actions.clone())
        };

        let (is_selected, focus_handle) = {
            let state = frame.state.read(cx);
            (state.selected_slot.as_deref() == Some(slot.id.as_str()), state.focus_handle.clone())
        };
        let accent = frame.theme.primary._100;
        let marker_group = SharedString::from(format!("map-grid-cell-{}", slot.id));
        let face_group = marker_group.clone();
        let face_slot = slot.clone();
        let face_card = self.clone();
        let background = frame.theme.bg._300;
        let content: HoldContextMenuSourceContent = Rc::new(move |_: &mut Window, _: &mut App| {
            let photo_source = face_slot.photo.as_ref().and_then(|photo| MapPhotoURL::absolute(photo.display_url()));
            let cancel_card = face_card.clone();
            let cancel_slot = face_slot.id.clone();
            div()
                .relative()
                .w_full()
                .aspect_square()
                .overflow_hidden()
                .bg(background)
                .opacity(if is_lifted { 0.0 } else { 1.0 })
                .child(WindowScreenFrameReader::new(registry_id.clone()))
                .children(preview.clone().map(|preview| MapGridLocalPreview {
                    id: format!("map-grid-preview-{}", face_slot.id),
                    preview,
                }))
                .children(photo_source.map(|source| {
                    div().absolute().size_full().child(
                        CleverImageUi::new(source)
                            .id(ElementId::Name(format!("map-grid-image-{}", face_slot.id).into()))
                            .content_mode(ContentMode::Fill)
                            .corner_radius(0.0)
                            .persist_to_disk(false)
                            .maximum_decode_pixel_dimension(512_i64)
                            .cache_policy(CleverImageCachePolicy::MemoryOnly),
                    )
                }))
                .children(is_video.then(|| {
                    div().absolute().left(px(5.0)).bottom(px(5.0)).child(Self::duration_badge(duration_ms))
                }))
                .children((face_slot.draft.is_some() && !is_sending).then(|| {
                    div()
                        .absolute()
                        .top(px(5.0))
                        .right(px(5.0))
                        .p(px(4.0))
                        .rounded_full()
                        .bg(black().opacity(0.5))
                        .children(IconUi::sized("wifi.slash", px(11.0), white(), false))
                }))
                .children(is_sending.then(|| MapGridUploadOverlay {
                    slot_id: cancel_slot.clone(),
                    on_cancel: Rc::new(move |window: &mut Window, cx: &mut App| {
                        (cancel_card.on_cancel_upload)(cancel_slot.clone(), window, cx);
                    }),
                }))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .when(is_selected, |marker| marker.stroke_border(accent, 2.0))
                        .group_hover(face_group.clone(), |marker| marker.bg(white().opacity(0.08))),
                )
                .into_any_element()
        });

        let action_card = self.clone();
        let action_slot = slot.clone();
        let action_frame = frame.clone();
        let tap_card = self.clone();
        let tap_slot = slot.clone();
        let tap_frame = frame.clone();
        let container = HoldContextMenuContainer::new(
            ElementId::Name(format!("map-grid-slot-{}", slot.id).into()),
            HoldContextMenuAlignment::Leading,
            actions_provider,
            content,
        )
        .show_emoji_list(false)
        .disabled(is_sending || slot.photo.as_ref().is_some_and(|photo| photo.is_pending))
        .source_identity(Some(slot.id.clone()))
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            action_card.perform_slot_action(&id, &action_slot, &action_frame, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            if is_sending {
                return;
            }
            if let Some(photo) = &tap_slot.photo {
                tap_card.open_viewer(photo, &tap_frame, cx);
            } else if let Some(draft) = &tap_slot.draft {
                tap_card.open_draft_viewer(draft, &tap_frame, cx);
            }
        }));

        let hover_state = frame.state.clone();
        let hover_slot = slot.id.clone();
        let hover_focus = focus_handle.clone();
        let press_state = frame.state.clone();
        let press_slot = slot.id.clone();
        let cell = div()
            .id(ElementId::Name(format!("map-grid-cell-{}", slot.id).into()))
            .group(marker_group)
            .relative()
            .flex_1()
            .min_w_0()
            .child(container)
            .on_hover(move |hovered, window, cx| {
                if !*hovered {
                    return;
                }
                hover_state.update(cx, |state, cx| {
                    state.selected_slot = Some(hover_slot.clone());
                    cx.notify();
                });
                if ScreenFocus::is_idle(window, cx) {
                    window.focus(&hover_focus, cx);
                }
            })
            .capture_any_mouse_down(move |_, window, cx| {
                press_state.update(cx, |state, cx| {
                    state.selected_slot = Some(press_slot.clone());
                    cx.notify();
                });
                window.focus(&focus_handle, cx);
            });

        if slot.photo.is_some() && last_slot.is_some_and(|last| last.id == slot.id) {
            let on_load_more = self.on_load_more.clone();
            return cell
                .on_appear(
                    ElementId::Name(format!("map-grid-last-{}", slot.id).into()),
                    move |window, cx| on_load_more(window, cx),
                )
                .into_any_element();
        }
        cell.into_any_element()
    }

    fn perform_slot_action(
        self: &Rc<Self>,
        id: &str,
        slot: &MapGridSlot,
        frame: &MapPhotoGridCardFrame,
        cx: &mut App,
    ) {
        if id == Self::DELETE_ACTION_ID
            && let Some(photo) = &slot.photo
        {
            self.confirm_delete_photo(photo.id.clone(), false, &frame.presenter, cx);
        }
        if id == Self::PUBLISH_DRAFT_ACTION_ID
            && let Some(draft) = &slot.draft
        {
            (self.on_publish_drafts)(vec![draft.clone()], cx);
        }
        if id == Self::DISCARD_DRAFT_ACTION_ID
            && let Some(draft) = &slot.draft
        {
            let on_discard_drafts = self.on_discard_drafts.clone();
            let draft_id = draft.id.clone();
            NativeConfirmAlert::present_delete(
                &tr("map_draft_discard"),
                &tr("map_drafts_clear_text"),
                None,
                None,
                false,
                None,
                None,
                move |cx| on_discard_drafts(vec![draft_id], cx),
                cx,
            );
        }
    }

    fn hold_actions(
        &self,
        photo: Option<&MapPhoto>,
        draft: Option<&MapGalleryDraft>,
        is_sending: bool,
    ) -> Vec<HoldContextMenuAction> {
        if let Some(photo) = photo {
            if !self.is_own || photo.is_pending {
                return Vec::new();
            }
            return vec![
                HoldContextMenuAction::new(Self::DELETE_ACTION_ID, tr("map_photo_delete"))
                    .system_image("trash")
                    .destructive(true),
            ];
        }
        if draft.is_none() || is_sending {
            return Vec::new();
        }
        vec![
            HoldContextMenuAction::new(Self::PUBLISH_DRAFT_ACTION_ID, tr("map_draft_add")).system_image("plus.circle"),
            HoldContextMenuAction::new(Self::DISCARD_DRAFT_ACTION_ID, tr("map_draft_discard"))
                .system_image("xmark.bin")
                .destructive(true),
        ]
    }

    fn open_draft_viewer(self: &Rc<Self>, draft: &MapGalleryDraft, frame: &MapPhotoGridCardFrame, cx: &mut App) {
        let items: Vec<MediaViewerGalleryItem> = self.drafts.iter().map(Self::gallery_item).collect();
        let registry_id = Self::draft_registry_id(draft);
        let Some(index) = items.iter().position(|item| item.frame_registry_id == registry_id) else {
            return;
        };
        let skin = MediaViewerHeroSkin::new(draft.display_source(), false);
        let source_frame = MediaCellScreenFrameRegistry::frame(&registry_id).unwrap_or_default();
        let by_id: Rc<HashMap<String, MapGalleryDraft>> =
            Rc::new(self.drafts.iter().map(|draft| (draft.id.clone(), draft.clone())).collect());
        let publish = self.on_publish_drafts.clone();
        let presentation = RiseMapPresentation::shared(cx);
        let drafts = presentation.read(cx).drafts.clone();
        let applies_by_id = by_id.clone();
        let handlers = MediaViewerPresentationHandlers {
            accessory: Some(MediaViewerAccessoryAction {
                system_image: "plus".to_owned(),
                accessibility_label: tr("map_draft_add"),
                applies: Rc::new(move |item: &MediaViewerGalleryItem, cx: &App| {
                    applies_by_id.contains_key(&item.id) && !drafts.read(cx).snapshot().publishing.contains(&item.id)
                }),
                perform: Rc::new(move |item: &MediaViewerGalleryItem, cx: &mut App| {
                    if let Some(picked) = by_id.get(&item.id) {
                        publish(vec![picked.clone()], cx);
                    }
                }),
            }),
            ..Default::default()
        };
        Self::present_viewer(frame.presenter.clone(), items, index, source_frame, registry_id, skin, handlers, None, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn present_viewer(
        presenter: Entity<MediaViewerPresentationState>,
        items: Vec<MediaViewerGalleryItem>,
        index: usize,
        source_frame: Bounds<Pixels>,
        registry_id: String,
        skin: MediaViewerHeroSkin,
        handlers: MediaViewerPresentationHandlers,
        paged_by: Option<Entity<MapPhotoGridCardState>>,
        cx: &mut App,
    ) {
        let scope = if paged_by.is_some() { MediaViewerGalleryScope::Paged } else { MediaViewerGalleryScope::Complete };
        let opened = presenter.clone();
        cx.spawn(async move |cx| {
            MediaViewerOpen::present(
                presenter,
                items,
                index,
                source_frame,
                registry_id,
                skin,
                MediaViewerChrome { shows_bottom_bar: true, can_report: false, ..Default::default() },
                scope,
                handlers,
                cx,
            )
            .await;
            let Some(state) = paged_by else { return };
            cx.update(|cx| {
                let presentation_id = opened.read(cx).presentation_id();
                state.update(cx, |state, cx| {
                    state.viewer_presentation_id = presentation_id;
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn draft_registry_id(draft: &MapGalleryDraft) -> String {
        format!("map-draft-{}", draft.id)
    }

    fn gallery_item(draft: &MapGalleryDraft) -> MediaViewerGalleryItem {
        MediaViewerGalleryItem {
            id: draft.id.clone(),
            kind: MediaViewerGalleryItemKind::Image,
            url: draft.display_source(),
            thumbnail_url: None,
            width: draft.width as f32,
            height: draft.height as f32,
            duration: None,
            sender_id: String::new(),
            sender_name: String::new(),
            sender_logo: None,
            created_at: draft.taken_at as TimeInterval / 1_000.0,
            caption_title: String::new(),
            caption: String::new(),
            message_id: draft.id.clone(),
            index_in_message: 0,
            media_item_id: draft.id.clone(),
            uses_fit_letterbox: false,
            frame_registry_id: Self::draft_registry_id(draft),
            is_own: true,
            can_delete: false,
            can_save: false,
            can_forward: false,
            can_reply: false,
            can_show_in_context: false,
            can_delete_for_me: false,
            can_delete_for_everyone: false,
            can_set_as_main: false,
            is_album_row: false,
            display_source_override: None,
            persist_media_to_disk: false,
            deletes_from_favourites: false,
            post_download: None,
            image_cache_policy: CleverImageCachePolicy::MemoryOnly,
        }
    }

    fn duration_badge(duration_ms: i64) -> AnyElement {
        hstack(3.0)
            .px(px(6.0))
            .py(px(3.0))
            .capsule()
            .bg(black().opacity(0.55))
            .text_color(white())
            .children(IconUi::sized("play.fill", px(8.0), white(), true))
            .child(
                div()
                    .swiftui_font(Font::system_weight(10.0, FontWeight::SEMIBOLD).monospaced_digit())
                    .child(SharedString::from(RiseFormat::duration_milliseconds(duration_ms))),
            )
            .into_any_element()
    }

    fn round_button(&self, theme: &AppTheme, system_image: &str, progress: Option<f64>) -> AnyElement {
        let on_add = self.on_add.clone();
        let face = div().size(px(34.0)).flex().items_center().justify_center().rounded_full().bg(theme.primary._100);
        let hint = tr("map_photo_add");
        let face = face.id(self.id_for("add-face")).tooltip_label(hint);
        let face = match progress {
            Some(progress) => face.child(MapPhotoGridCardProgressRing { id: self.id_for("add-progress"), progress }),
            None => face.children(IconUi::sized(system_image, px(14.0), theme.content_on(theme.primary._100), false)),
        };
        plain(self.id_for("add"))
            .disabled(progress.is_some())
            .on_press(move |window, cx| on_add(window, cx))
            .child(face)
            .into_any_element()
    }

    fn round_label(&self, theme: &AppTheme, system_image: &str, part: &str, hint: String) -> Stateful<Div> {
        div()
            .id(self.id_for(part))
            .size(px(34.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme.text.primary.opacity(0.1))
            .tooltip_label(hint)
            .children(IconUi::sized(system_image, px(13.0), theme.text.primary, false))
    }

    fn open_viewer(self: &Rc<Self>, photo: &MapPhoto, frame: &MapPhotoGridCardFrame, cx: &mut App) {
        if photo.is_pending {
            return;
        }
        let items = Self::viewer_items(&frame.photos);
        let registry_id = Self::registry_id(photo);
        let Some(index) = items.iter().position(|item| item.frame_registry_id == registry_id) else {
            return;
        };
        let skin = MediaViewerHeroSkin::new(items[index].display_source(), false);
        let source_frame = MediaCellScreenFrameRegistry::frame(&registry_id).unwrap_or_default();
        let accessory = self.is_own.then(|| {
            let card = self.clone();
            let presenter = frame.presenter.clone();
            MediaViewerAccessoryAction {
                system_image: "trash".to_owned(),
                accessibility_label: tr("map_photo_delete"),
                applies: Rc::new(|_: &MediaViewerGalleryItem, _: &App| true),
                perform: Rc::new(move |item: &MediaViewerGalleryItem, cx: &mut App| {
                    card.confirm_delete_photo(item.id.clone(), true, &presenter, cx);
                }),
            }
        });
        let handlers = MediaViewerPresentationHandlers { accessory, ..Default::default() };
        frame.state.update(cx, |state, _| {
            state.viewer_presentation_id = None;
            state.viewer_load_requested_at = None;
        });
        Self::present_viewer(
            frame.presenter.clone(),
            items,
            index,
            source_frame,
            registry_id,
            skin,
            handlers,
            Some(frame.state.clone()),
            cx,
        );
    }

    /// published photos as the viewer pages them, pending uploads left out
    fn viewer_items(photos: &[MapPhoto]) -> Vec<MediaViewerGalleryItem> {
        photos
            .iter()
            .filter(|photo| !photo.is_pending)
            .filter_map(|photo| Self::gallery_item_can_delete(photo, false))
            .collect()
    }

    fn sync_viewer(&self, frame: &MapPhotoGridCardFrame, window: &mut Window, cx: &mut App) {
        const LOAD_AHEAD: usize = 6;
        let (presentation_id, load_requested_at) = {
            let state = frame.state.read(cx);
            (state.viewer_presentation_id, state.viewer_load_requested_at)
        };
        let Some(presentation_id) = presentation_id else {
            return;
        };
        if !frame.presenter.read(cx).is_active_presentation(presentation_id) {
            frame.state.update(cx, |state, _| {
                state.viewer_presentation_id = None;
                state.viewer_load_requested_at = None;
            });
            return;
        }
        let Some(open) = self.matching_open() else {
            return;
        };
        let items = Self::viewer_items(&frame.photos);
        if items.is_empty() {
            return;
        }
        let differs = {
            let shown = frame.presenter.read(cx).items();
            shown.len() != items.len() || shown.iter().zip(items.iter()).any(|(shown, next)| shown.id != next.id)
        };
        let loaded = items.len();
        if differs {
            frame.presenter.update(cx, |presenter, cx| presenter.replace_items(items, None, cx));
        }
        let (index, current_id, position, total, counter_id) = {
            let presenter = frame.presenter.read(cx);
            let index = presenter.current_index();
            (
                index,
                presenter.items().get(index).map(|item| item.id.clone()),
                presenter.gallery_position(),
                presenter.gallery_total(),
                presenter.gallery_counter_item_id().map(str::to_owned),
            )
        };
        let next_position = i64::try_from(index + 1).unwrap_or(i64::MAX);
        let loaded_count = i64::try_from(loaded).unwrap_or(i64::MAX);
        let next_total = if open.has_more { self.grid.photo_count.max(loaded_count) } else { loaded_count };
        if position != Some(next_position) || total != Some(next_total) || counter_id != current_id {
            frame.presenter.update(cx, |presenter, cx| {
                presenter.set_gallery_counter_follows_index(true);
                presenter.set_gallery_counter(Some(next_position), Some(next_total), cx)
            });
        }
        if open.has_more && index + LOAD_AHEAD >= loaded && load_requested_at != Some(loaded) {
            frame.state.update(cx, |state, _| state.viewer_load_requested_at = Some(loaded));
            (self.on_load_more)(window, cx);
        }
    }

    fn confirm_delete_photo(
        &self,
        id: String,
        dismiss_viewer: bool,
        presenter: &Entity<MediaViewerPresentationState>,
        cx: &mut App,
    ) {
        if !self.is_own {
            return;
        }
        let on_delete_photo = self.on_delete_photo.clone();
        let presenter = presenter.clone();
        NativeConfirmAlert::present_delete(
            &tr("map_photo_delete_title"),
            &tr("map_photo_delete_text"),
            Some(tr("map_photo_delete")),
            None,
            false,
            None,
            None,
            move |cx| {
                on_delete_photo(id, cx);
                if dismiss_viewer {
                    presenter.update(cx, |presenter, cx| presenter.request_animated_dismissal(cx));
                }
            },
            cx,
        );
    }

    fn registry_id(photo: &MapPhoto) -> String {
        format!("map-grid-{}", photo.id)
    }

    fn gallery_item_can_delete(photo: &MapPhoto, can_delete: bool) -> Option<MediaViewerGalleryItem> {
        let url = MapPhotoURL::absolute(&photo.url)?;
        let stamp = if photo.taken_at > 0 { photo.taken_at } else { photo.created_at };
        let is_video = photo.media_kind == MapMediaKind::Video;
        Some(MediaViewerGalleryItem {
            id: photo.id.clone(),
            kind: if is_video { MediaViewerGalleryItemKind::Video } else { MediaViewerGalleryItemKind::Image },
            url,
            thumbnail_url: if photo.thumbnail_url.is_empty() {
                None
            } else {
                MapPhotoURL::absolute(&photo.thumbnail_url)
            },
            width: photo.width as f32,
            height: photo.height as f32,
            duration: is_video.then(|| photo.duration_ms as TimeInterval / 1_000.0),
            sender_id: photo.owner_id.clone(),
            sender_name: String::new(),
            sender_logo: None,
            created_at: stamp as TimeInterval / 1_000.0,
            caption_title: String::new(),
            caption: photo.caption.clone(),
            message_id: photo.id.clone(),
            index_in_message: 0,
            media_item_id: photo.id.clone(),
            uses_fit_letterbox: false,
            frame_registry_id: Self::registry_id(photo),
            is_own: can_delete,
            can_delete,
            can_save: true,
            can_forward: false,
            can_reply: false,
            can_show_in_context: false,
            can_delete_for_me: false,
            can_delete_for_everyone: can_delete,
            can_set_as_main: false,
            is_album_row: false,
            display_source_override: None,
            persist_media_to_disk: false,
            deletes_from_favourites: false,
            post_download: None,
            image_cache_policy: CleverImageCachePolicy::MemoryOnly,
        })
    }

    fn date_text(stamp_ms: i64) -> String {
        if stamp_ms <= 0 {
            return String::new();
        }
        let mut formatter = AppDateFormatter::make(None, None, None);
        formatter.set_date_style(Style::Medium);
        formatter.set_time_style(Style::Short);
        formatter.string_from(&Date::from_time_interval_since_1970(stamp_ms as TimeInterval / 1_000.0))
    }
}

impl RenderOnce for MapPhotoGridCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let presenter = media_viewer_presentation(cx);
        let observed = presenter.clone();
        let state = window.use_keyed_state(self.id_for("state"), cx, move |_, cx| MapPhotoGridCardState {
            selected_slot: None,
            is_grid_menu_presented: false,
            focus_handle: cx.focus_handle(),
            grid_width: None,
            viewer_presentation_id: None,
            viewer_load_requested_at: None,
            _viewer_observation: cx.observe(&observed, |_, _, cx| cx.notify()),
        });
        let photos = Rc::new(self.matching_open().map_or_else(Vec::new, |open| open.photos.clone()));
        let waiting = self.waiting_drafts();
        let card = Rc::new(self);
        let frame = MapPhotoGridCardFrame { theme, presenter, state, photos };
        card.sync_viewer(&frame, window, cx);
        let header = card.header(&frame, waiting);
        let content = card.content(&frame, cx);
        if card.docked {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .occlude()
                .child(vstack(10.0).align(HorizontalAlignment::Leading).size_full().child(header).child(content))
                .on_scroll_wheel(|_: &ScrollWheelEvent, _, cx| cx.stop_propagation())
                .into_any_element();
        }
        div()
            .w_full()
            .p(px(12.0))
            .occlude()
            .child(vstack(10.0).align(HorizontalAlignment::Leading).w_full().child(header).child(content))
            .on_scroll_wheel(|_: &ScrollWheelEvent, _, cx| cx.stop_propagation())
            .liquid_chrome_rounded_rectangle(MapPhotoGridCard::CORNER_RADIUS)
    }
}

#[derive(Clone)]
struct MapGridSlot {
    id: String,
    photo: Option<MapPhoto>,
    draft: Option<MapGalleryDraft>,
    upload: Option<MapGridUpload>,
}

#[derive(IntoElement)]
struct MapGridLocalPreview {
    id: String,
    preview: MapGridPreview,
}

impl RenderOnce for MapGridLocalPreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let image: Entity<Option<Arc<RenderImage>>> =
            window.use_keyed_state(ElementId::Name(format!("{}-image", self.id).into()), cx, |_, _| None);
        let target = image.clone();
        let preview = self.preview.clone();
        use_task(
            ElementId::Name(format!("{}-load", self.id).into()),
            self.preview,
            window,
            cx,
            async move |cx| {
                let loaded = match &preview {
                    MapGridPreview::Asset(local_identifier) => {
                        MapGalleryThumbnail::load_local_identifier(local_identifier, 140.0).await
                    }
                    MapGridPreview::File(url) => MapGalleryThumbnail::load_file_url(url, 140.0).await,
                };
                target.update(cx, |image, cx| {
                    *image = loaded;
                    cx.notify();
                });
            },
        );
        div()
            .absolute()
            .size_full()
            .children(image.read(cx).clone().map(|image| img(image).size_full().object_fit(ObjectFit::Cover)))
    }
}

struct MapGridUploadObservation {
    observed: Entity<MapUploadProgressBox>,
    _subscription: Subscription,
}

#[derive(IntoElement)]
struct MapGridUploadOverlay {
    slot_id: String,
    on_cancel: MapPhotoGridCardAction,
}

impl MapGridUploadOverlay {
    const RING_SIDE: f32 = 40.0;

    fn child_id(&self, part: &str) -> ElementId {
        ElementId::Name(format!("map-grid-upload-{}-{part}", self.slot_id).into())
    }
}

impl RenderOnce for MapGridUploadOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let center = MapGridUploadCenter::shared(cx);
        let progress = center.update(cx, |center, cx| center.box_for(&self.slot_id, cx));
        let observed = progress.clone();
        let observation = window.use_keyed_state(self.child_id("observation"), cx, move |_, cx| {
            let subscription = cx.observe(&observed, |_, _, cx| cx.notify());
            MapGridUploadObservation { observed, _subscription: subscription }
        });
        let current = progress.clone();
        observation.update(cx, |observation, cx| {
            if observation.observed != current {
                observation._subscription = cx.observe(&current, |_, _, cx| cx.notify());
                observation.observed = current;
            }
        });
        let (is_active, fraction, total_bytes) = {
            let progress = progress.read(cx);
            (progress.is_active, progress.fraction, progress.total_bytes)
        };

        let target = if is_active { fraction.clamp(0.06, 1.0) as f32 } else { 0.3 };
        let now = RiseClock::monotonic();
        let ring = window.use_keyed_state(self.child_id("ring"), cx, |_, _| Animated::new(target));
        ring.update(cx, |ring, cx| ring.set_in(target, linear(0.15), now, cx));
        let shown = ring.read(cx).frame_value(now, window);

        let bytes = (is_active && total_bytes > 0).then(|| {
            format_printf(
                &tr("chat_media_upload_progress"),
                &[
                    FormatArg::Text(RiseFormat::file_size_unit_of((total_bytes as f64 * fraction) as i64, total_bytes).into()),
                    FormatArg::Text(RiseFormat::file_size(total_bytes).into()),
                ],
            )
        });
        let on_cancel = self.on_cancel.clone();
        let hint = tr("map_grid_upload_cancel");
        div()
            .absolute()
            .size_full()
            .bg(black().opacity(if is_active { 0.3 } else { 0.22 }))
            .children(bytes.map(|bytes| {
                div()
                    .absolute()
                    .top(px(4.0))
                    .left(px(4.0))
                    .right(px(4.0))
                    .flex()
                    .child(
                        div().max_w_full().flex_none().px(px(5.0)).py(px(2.5)).capsule().bg(black().opacity(0.45)).child(
                            MainText::new(bytes)
                                .px(9.5)
                                .font_weight(FontWeight::SEMIBOLD)
                                .monospaced_digits(true)
                                .color(white().opacity(0.95))
                                .number_of_lines(1)
                                .minimum_scale_factor(0.7),
                        ),
                    )
            }))
            .child(
                div().absolute().size_full().flex().items_center().justify_center().child(
                    plain(self.child_id("cancel")).on_press(move |window, cx| on_cancel(window, cx)).child(
                        div()
                            .id(self.child_id("cancel-face"))
                            .size(px(Self::RING_SIDE + 12.0))
                            .tooltip_label(hint)
                            .child(
                                zstack(Alignment::Center)
                                    .size_full()
                                    .child(div().size(px(Self::RING_SIDE)).rounded_full().bg(black().opacity(0.48)))
                                    .child(
                                        LoaderUi::new()
                                            .size(LoaderUiSize::Custom(Self::RING_SIDE))
                                            .progress(f64::from(shown))
                                            .r#type(LoaderUiType::Progress)
                                            .thickness(2.2)
                                            .track_color(Some(white().opacity(0.28)))
                                            .ring_animated(true)
                                            .speed(3.0)
                                            .gap(3.0),
                                    )
                                    .children(IconUi::sized("xmark", px(13.0), white(), false)),
                            ),
                    ),
                ),
            )
    }
}

#[derive(IntoElement)]
struct MapPhotoGridCardProgressRing {
    id: ElementId,
    progress: f64,
}

impl RenderOnce for MapPhotoGridCardProgressRing {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let target = self.progress.clamp(0.04, 1.0) as f32;
        let now = RiseClock::monotonic();
        let trim = window.use_keyed_state(self.id, cx, |_, _| Animated::new(target));
        trim.update(cx, |trim, cx| trim.set_in(target, ease_out(0.2), now, cx));
        let stroke_end = trim.read(cx).frame_value(now, window);
        canvas(|_, _, _| (), move |bounds, (), window, _| paint_progress(bounds, stroke_end, white(), window)).size_full()
    }
}

fn paint_progress(bounds: Bounds<Pixels>, stroke_end: f32, tint: Hsla, window: &mut Window) {
    const INSET: f32 = 9.0;
    const LINE_WIDTH: f32 = 2.5;
    let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0 - INSET;
    if radius <= 0.0 {
        return;
    }
    let centre = bounds.center();
    let sweep = 2.0 * PI * stroke_end.clamp(0.0, 1.0);
    let steps = ((sweep / (2.0 * PI)) * 96.0).ceil().max(2.0) as usize;
    let arc: Vec<Point<Pixels>> = (0..=steps)
        .map(|step| {
            let angle = -PI / 2.0 + sweep * step as f32 / steps as f32;
            point(centre.x + px(radius * angle.cos()), centre.y + px(radius * angle.sin()))
        })
        .collect();
    let mut builder = PathBuilder::stroke(px(LINE_WIDTH));
    builder.add_polygon(&arc, false);
    if let Ok(path) = builder.build() {
        window.paint_path(path, tint);
    }
    let half = LINE_WIDTH / 2.0;
    for end in [arc.first(), arc.last()].into_iter().flatten() {
        let dot = Bounds::new(point(end.x - px(half), end.y - px(half)), size(px(LINE_WIDTH), px(LINE_WIDTH)));
        window.paint_quad(fill(dot, tint).corner_radii(px(half)));
    }
}
