use std::rc::Rc;

use gpui::{App, Context, ElementId, Entity, FontWeight, Hsla, ListAlignment, ListState, SharedString, Subscription, Window, black, list, white};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::pressable_ui::PressableUi;
use rise_ui::swiftui::binding::Binding;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::liquid_glass_button::{LiquidGlassButtonModifiers, LiquidPressShape};
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::media::rise_media_kind_classifier::RiseMediaKind;
use crate::core::storage::rise_storage_usage_models::RiseStorageMediaEntry;
use crate::core::storage::rise_storage_usage_presentation::RiseStorageUsagePresentation;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_under_chrome_box};
use crate::modules::post::pages::posts::post_detail::list_rows_splice_hooks::splice_list_rows;
use crate::modules::story::components::stories_grid::stories_grid_view::{container_width_reader, use_container_width};
use crate::modules::user::stores::user_storage::user_storage_interactions::user_storage_interactions::UserStorageInteractionsStore;
use crate::modules::user::stores::user_storage::user_storage_services::user_storage_services::UserStorageServicesStore;

const MIN_COLUMNS: usize = 3;
const TILE_TARGET_WIDTH: f32 = 120.0;
const MAX_COLUMNS: usize = 12;
const GRID_SPACING: f32 = 2.0;
const SKELETON_TILES: usize = 24;
const LIST_OVERDRAW: f32 = 400.0;
const CLEAR_ROW_BOTTOM_GAP: f32 = 12.0;
const CLEAR_ROW_HEIGHT: f32 =
    GroupedCardMetrics::ICON_SIDE + 2.0 * GroupedCardMetrics::ROW_VERTICAL_PADDING + CLEAR_ROW_BOTTOM_GAP;

/// three columns in a narrow pane, more in a wide one so a tile stays near TILE_TARGET_WIDTH
fn column_count(container_width: f32) -> usize {
    if !container_width.is_finite() {
        return MIN_COLUMNS;
    }
    let fit = ((container_width + GRID_SPACING) / (TILE_TARGET_WIDTH + GRID_SPACING)).round() as usize;
    fit.clamp(MIN_COLUMNS, MAX_COLUMNS)
}

enum CachedMediaTileStyle {}

impl CachedMediaTileStyle {
    fn icon(kind: RiseMediaKind) -> &'static str {
        match kind {
            RiseMediaKind::Image => "photo",
            RiseMediaKind::Video => "play.rectangle.fill",
            RiseMediaKind::Audio => "music.note",
            RiseMediaKind::File => "doc.fill",
        }
    }

    fn i18n_key(kind: RiseMediaKind) -> &'static str {
        match kind {
            RiseMediaKind::Image => "memory_settings_images",
            RiseMediaKind::Video => "memory_settings_videos",
            RiseMediaKind::Audio => "memory_settings_audio",
            RiseMediaKind::File => "memory_settings_files",
        }
    }
}

/// inspected and confirm clear flags,
/// the storage store and the lazy window over header,
/// clear row and grid rows
struct CachedMediaState {
    inspected: Option<RiseStorageMediaEntry>,
    confirming_clear: bool,
    menu_entry_id: Option<String>,
    list: ListState,
    rendered_rows: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl CachedMediaState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseStorageUsagePresentation::shared(cx);
        Self {
            inspected: None,
            confirming_clear: false,
            menu_entry_id: None,
            list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            rendered_rows: Vec::new(),
            _subscriptions: vec![cx.observe(&presentation, |_, _, cx| cx.notify())],
        }
    }

    fn set_inspected(&mut self, inspected: Option<RiseStorageMediaEntry>, cx: &mut Context<Self>) {
        self.inspected = inspected;
        cx.notify();
    }

    fn set_menu_entry_id(&mut self, menu_entry_id: Option<String>, cx: &mut Context<Self>) {
        self.menu_entry_id = menu_entry_id;
        cx.notify();
    }

    fn set_confirming_clear(&mut self, confirming_clear: bool, cx: &mut Context<Self>) {
        self.confirming_clear = confirming_clear;
        cx.notify();
    }
}

enum CachedMediaRow {
    Header,
    ClearButton,
    Tiles(Vec<RiseStorageMediaEntry>),
    BottomSpacer,
}

/// what a row of the window shows at this render, owned by the list closure
struct CachedMediaRows {
    rows: Vec<CachedMediaRow>,
    columns: usize,
    total_bytes: i64,
    shown: usize,
    total_count: usize,
    is_clearing: bool,
    state: Entity<CachedMediaState>,
    theme: AppTheme,
}

fn row_id(name: &str, id: &str) -> ElementId {
    SharedString::from(format!("{name}-{id}")).into()
}

fn symbol(name: &str, side: f32, color: Hsla) -> AnyElement {
    match IconUi::sized(name, px(side), color, name.ends_with(".fill")) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}

#[derive(IntoElement)]
pub struct CachedMedia;

impl CachedMedia {
    pub fn new() -> Self {
        Self
    }

    fn header(rows: &CachedMediaRows) -> AnyElement {
        let mut column = vstack(2.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(16.0))
            .pt(px(12.0))
            .pb(px(10.0))
            .child(MainText::new(RiseFormat::file_size(rows.total_bytes)).px(20.0).font_weight(FontWeight::SEMIBOLD));
        if rows.total_count > rows.shown {
            column = column.child(
                SecondaryText::new(
                    tr("memory_settings_media_showing")
                        .replace("{shown}", &rows.shown.to_string())
                        .replace("{total}", &rows.total_count.to_string()),
                )
                .px(12.0),
            );
        }
        column.into_any_element()
    }

    /// fill paints outside its box so the tile is clipped to its square or wide pictures bleed
    /// over neighbours
    fn tile(entry: &RiseStorageMediaEntry, rows: &CachedMediaRows) -> AnyElement {
        let theme = &rows.theme;
        let picture = match &entry.preview_source {
            Some(source) => CleverImageUi::new(SharedString::from(source.clone()))
                .id(row_id("cached-media-picture", &entry.id))
                .content_mode(ContentMode::Fill)
                .persist_to_disk(true)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(320_i64)
                .shows_failure_icon(false)
                .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.bg._300)
                .child(symbol(CachedMediaTileStyle::icon(entry.kind), 22.0, theme.text.secondary))
                .into_any_element(),
        };
        let is_right_to_left = LayoutDirection::current().is_right_to_left();
        let size_badge = div()
            .absolute()
            .bottom(px(4.0))
            .when(is_right_to_left, |badge| badge.right(px(4.0)))
            .when(!is_right_to_left, |badge| badge.left(px(4.0)))
            .px(px(5.0))
            .py(px(2.0))
            .bg(black().opacity(0.45))
            .capsule()
            .child(MainText::new(RiseFormat::file_size(entry.byte_count)).px(10.0).font_weight(FontWeight::SEMIBOLD).color(white()));

        let inspected_state = rows.state.clone();
        let inspected_entry = entry.clone();
        let opener = rows.state.clone();
        let opener_entry = entry.clone();
        let presented_id = entry.id.clone();
        let dismissed_id = entry.id.clone();
        let is_menu_presented = Binding::entity(
            rows.state.clone(),
            move |state: &CachedMediaState| state.menu_entry_id.as_deref() == Some(presented_id.as_str()),
            move |state: &mut CachedMediaState, is_presented: bool, cx| {
                if !is_presented && state.menu_entry_id.as_deref() == Some(dismissed_id.as_str()) {
                    state.set_menu_entry_id(None, cx);
                }
            },
        );
        let actions: HoldContextMenuPresentationActions = Rc::new(|_: &App| {
            vec![HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true)]
        });
        let deleted_entry = entry.clone();
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |_: String, _: &mut Window, cx: &mut App| {
            let deleted_entry = deleted_entry.clone();
            UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_media_entry(deleted_entry, cx));
        });
        let source: HoldContextMenuTapSource = Rc::new(|_: &mut Window, _: &mut App| div().into_any_element());
        div()
            .flex_1()
            .min_w_0()
            .child(
                PressableUi::new(row_id("cached-media-tile", &entry.id))
                    .tap_scale(1.0)
                    .long_press_scale(1.0)
                    .long_press_duration(HoldContextMenuMetrics::CONFIRM_DURATION as f32)
                    .on_press(move |_, cx| {
                        if inspected_state.read(cx).menu_entry_id.is_some() {
                            return;
                        }
                        let entry = inspected_entry.clone();
                        inspected_state.update(cx, |state, cx| state.set_inspected(Some(entry), cx));
                    })
                    .on_long_press(move |_, cx| {
                        if !opener_entry.is_deletable() {
                            return;
                        }
                        let _ = ImpactFeedbackGenerator::new(ImpactStyle::Medium).impact_occurred();
                        let id = opener_entry.id.clone();
                        opener.update(cx, |state, cx| state.set_menu_entry_id(Some(id), cx));
                    })
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .aspect_ratio(1.0)
                            .overflow_hidden()
                            .child(div().absolute().inset_0().child(picture))
                            .child(size_badge),
                    )
                    .tap_context_menu(
                        row_id("cached-media-tile-menu", &entry.id),
                        is_menu_presented,
                        HoldContextMenuAlignment::Leading,
                        false,
                        false,
                        false,
                        None,
                        actions,
                        on_action,
                        source,
                    ),
            )
            .into_any_element()
    }

    fn tiles_row(entries: &[RiseStorageMediaEntry], rows: &CachedMediaRows) -> AnyElement {
        let mut row = hstack(GRID_SPACING).w_full().pb(px(GRID_SPACING));
        for entry in entries {
            row = row.child(Self::tile(entry, rows));
        }
        for _ in entries.len()..rows.columns {
            row = row.child(div().flex_1().min_w_0());
        }
        row.into_any_element()
    }

    fn grid_skeleton(columns: usize) -> AnyElement {
        let mut column = vstack(GRID_SPACING).w_full().pt(px(44.0 + CLEAR_ROW_HEIGHT));
        for _ in 0..SKELETON_TILES.div_ceil(columns) {
            let mut row = hstack(GRID_SPACING).w_full();
            for _ in 0..columns {
                row = row.child(
                    div().relative().flex_1().min_w_0().aspect_ratio(1.0).child(
                        div().absolute().inset_0().child(SkeletonBlock::new(None, None, 0.0)),
                    ),
                );
            }
            column = column.child(row);
        }
        scroll_view("cached-media-loading")
            .size_full()
            .scroll_disabled(true)
            .child(column.skeleton_shimmer(None))
            .into_any_element()
    }

    fn clear_button(rows: &CachedMediaRows) -> AnyElement {
        let state = rows.state.clone();
        let text = if rows.is_clearing { tr("memory_settings_clearing") } else { tr("memory_settings_clear_media") };
        let mut button = GroupedBtn::new("clear_media", text)
            .icon("trash")
            .destructive(true)
            .disabled(rows.is_clearing || rows.total_bytes == 0)
            .show_chevron(false)
            .callback(move |_, cx| state.update(cx, |state, cx| state.set_confirming_clear(true, cx)));
        if rows.total_bytes > 0 {
            button = button.trailing(RiseFormat::file_size(rows.total_bytes));
        }
        div()
            .w_full()
            .pb(px(CLEAR_ROW_BOTTOM_GAP))
            .child(GroupedBtns::new("cached-media-clear", vec![button]))
            .into_any_element()
    }

    fn clear_sheet_inner(total_bytes: i64) -> AnyElement {
        vstack(10.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(20.0))
            .pt(px(8.0))
            .child(MainText::new(tr("memory_settings_clear_dialog_title")).px(18.0).font_weight(FontWeight::SEMIBOLD))
            .child(
                SecondaryText::new(tr("memory_settings_clear_dialog_body").replace("{size}", &RiseFormat::file_size(total_bytes)))
                    .px(14.0),
            )
            .child(spacer_min_h(0.0))
            .into_any_element()
    }

    fn footer_label(label: String) -> Div {
        hstack(8.0)
            .w_full()
            .justify_center()
            .py(px(15.0))
            .capsule()
            .child(symbol("trash", 16.0, white()))
            .child(MainText::new(label).px(16.0).font_weight(FontWeight::SEMIBOLD).color(white()))
    }

    fn clear_sheet_footer(state: Entity<CachedMediaState>, theme: &AppTheme) -> AnyElement {
        let action: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            state.update(cx, |state, cx| state.set_confirming_clear(false, cx));
            UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_all_media(cx));
        });
        div()
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(
                Self::footer_label(tr("memory_settings_clear_confirm")).bg(theme.semantic.error_100).liquid_press(
                    "cached-media-clear-confirm",
                    LiquidPressShape::RoundedRect(16.0),
                    true,
                    false,
                    false,
                    Some(action),
                ),
            )
            .into_any_element()
    }

    fn entry_sheet_inner(entry: &RiseStorageMediaEntry, theme: &AppTheme) -> AnyElement {
        let preview = match &entry.preview_source {
            Some(source) => CleverImageUi::new(SharedString::from(source.clone()))
                .id(row_id("cached-media-preview", &entry.id))
                .content_mode(ContentMode::Fit)
                .persist_to_disk(true)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(1_024_i64)
                .into_any_element(),
            None => symbol(CachedMediaTileStyle::icon(entry.kind), 44.0, theme.text.secondary),
        };
        vstack(12.0)
            .w_full()
            .px(px(20.0))
            .pt(px(8.0))
            .child(
                div()
                    .w_full()
                    .h(px(160.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .clip_shape(Shape::continuous(14.0))
                    .child(preview),
            )
            .child(
                vstack(2.0)
                    .child(MainText::new(tr(CachedMediaTileStyle::i18n_key(entry.kind))).px(16.0).font_weight(FontWeight::SEMIBOLD))
                    .child(SecondaryText::new(RiseFormat::file_size(entry.byte_count)).px(13.0)),
            )
            .child(spacer_min_h(0.0))
            .into_any_element()
    }

    fn entry_sheet_footer(
        entry: Option<RiseStorageMediaEntry>,
        is_clearing: bool,
        state: Entity<CachedMediaState>,
        theme: &AppTheme,
    ) -> AnyElement {
        let deletable = entry.as_ref().is_some_and(RiseStorageMediaEntry::is_deletable);
        let action: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            let target = entry.clone();
            state.update(cx, |state, cx| state.set_inspected(None, cx));
            let Some(target) = target.filter(RiseStorageMediaEntry::is_deletable) else {
                return;
            };
            UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_media_entry(target, cx));
        });
        div()
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(
                Self::footer_label(tr("ctx_delete"))
                    .bg(if deletable { theme.semantic.error_100 } else { theme.bg._300 })
                    .liquid_press(
                        "cached-media-entry-delete",
                        LiquidPressShape::RoundedRect(16.0),
                        true,
                        !deletable || is_clearing,
                        false,
                        Some(action),
                    ),
            )
            .into_any_element()
    }
}

impl Default for CachedMedia {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for CachedMedia {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("cached-media-state", cx, |_, cx| CachedMediaState::new(cx));
        let theme = rise_ui::theme(cx).clone();
        let width_state = use_container_width("cached-media-width".into(), window, cx);
        let columns = column_count(*width_state.read(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let storage = UserStorageServicesStore::shared(cx);
        let (media_status, media, total_count, is_clearing, snapshot) = storage.update(cx, |storage, cx| {
            (
                storage.media_status(cx),
                storage.media(cx),
                storage.media_total_count(cx),
                storage.is_clearing(cx),
                storage.snapshot(cx),
            )
        });
        let total_bytes: i64 = UserStorageServicesStore::MEDIA_SITES.iter().map(|site| snapshot.bytes_site(*site)).sum();
        let is_empty = media.is_empty();
        let shown = media.len();

        let mut rows = Vec::with_capacity(shown / columns + 4);
        rows.push(CachedMediaRow::Header);
        rows.push(CachedMediaRow::ClearButton);
        rows.extend(media.chunks(columns).map(|entries| CachedMediaRow::Tiles(entries.to_vec())));
        rows.push(CachedMediaRow::BottomSpacer);
        let row_ids: Vec<String> = rows
            .iter()
            .map(|row| match row {
                CachedMediaRow::Header => format!("header:{}", total_count > shown),
                CachedMediaRow::ClearButton => "clear".to_string(),
                CachedMediaRow::Tiles(entries) => {
                    entries.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>().join("|")
                }
                CachedMediaRow::BottomSpacer => "spacer".to_string(),
            })
            .collect();
        let list_state = state.update(cx, |state, _| {
            splice_list_rows(&state.list, &mut state.rendered_rows, &row_ids);
            state.list.clone()
        });

        let rows = Rc::new(CachedMediaRows {
            rows,
            columns,
            total_bytes,
            shown,
            total_count,
            is_clearing,
            state: state.clone(),
            theme: theme.clone(),
        });
        let content = move |_: &mut Window, _: &mut App| {
            list(list_state, move |index, _, _| match rows.rows.get(index) {
                Some(CachedMediaRow::Header) => Self::header(&rows),
                Some(CachedMediaRow::ClearButton) => Self::clear_button(&rows),
                Some(CachedMediaRow::Tiles(entries)) => Self::tiles_row(entries, &rows),
                Some(CachedMediaRow::BottomSpacer) | None => div().w_full().h(px(80.0)).into_any_element(),
            })
            .size_full()
        };

        let inspected_reader = state.clone();
        let inspected_writer = state.clone();
        let inspected_content = state.clone();
        let inspected_footer = state.clone();
        let confirming_reader = state.clone();
        let confirming_writer = state.clone();
        let clear_footer = state;

        div()
            .relative()
            .size_full()
            .child(
                SettingsWrapperUi::new(tr("memory_settings_cached_media"), move |_, cx| {
                    if let Some(nav_store) = &nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .content(
                    list_shell_under_chrome_box().size_full().bg(theme.bg._100).child(
                        div()
                            .relative()
                            .size_full()
                            .child(
                                SaiDataRender::new(media_status, content)
                                    .id("cached-media-data")
                                    .is_empty(is_empty)
                                    .no_data_text(tr("memory_settings_nothing_cached"))
                                    .sai_data_render_loading(move |_, _| Self::grid_skeleton(columns)),
                            )
                            .child(container_width_reader("cached-media-width-reader".into(), width_state)),
                    ),
                )
                .on_appear("cached-media-appear", |_, cx| {
                    UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.open_cached_media(cx));
                }),
            )
            .child(
                BottomSheetUi::new(
                    "cached-media-entry",
                    Binding::new(
                        move |cx| inspected_reader.read(cx).inspected.is_some(),
                        move |presented, _, cx| {
                            if !presented {
                                inspected_writer.update(cx, |state, cx| state.set_inspected(None, cx));
                            }
                        },
                    ),
                    move |_, cx| {
                        let theme = rise_ui::theme(cx).clone();
                        match &inspected_content.read(cx).inspected {
                            Some(entry) => Self::entry_sheet_inner(entry, &theme),
                            None => div().into_any_element(),
                        }
                    },
                    move |_, cx| {
                        let theme = rise_ui::theme(cx).clone();
                        let entry = inspected_footer.read(cx).inspected.clone();
                        let storage = UserStorageServicesStore::shared(cx);
                        let is_clearing = storage.update(cx, |storage, cx| storage.is_clearing(cx));
                        Self::entry_sheet_footer(entry, is_clearing, inspected_footer.clone(), &theme)
                    },
                )
                .snap_fractions(vec![0.5])
                .footer_shows_top_border(false)
                .content_scrollable(false),
            )
            .child(
                BottomSheetUi::new(
                    "cached-media-clear-sheet",
                    Binding::new(
                        move |cx| confirming_reader.read(cx).confirming_clear,
                        move |presented, _, cx| {
                            confirming_writer.update(cx, |state, cx| state.set_confirming_clear(presented, cx));
                        },
                    ),
                    move |_, _| Self::clear_sheet_inner(total_bytes),
                    move |_, cx| {
                        let theme = rise_ui::theme(cx).clone();
                        Self::clear_sheet_footer(clear_footer.clone(), &theme)
                    },
                )
                .snap_fractions(vec![0.42])
                .footer_shows_top_border(false)
                .content_scrollable(false)
                .fits_content(true),
            )
    }
}
