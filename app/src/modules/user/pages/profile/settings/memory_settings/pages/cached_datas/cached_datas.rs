use std::rc::Rc;

use gpui::{App, Context, Entity, FontWeight, Hsla, Subscription, Window, white};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::liquid_glass_button::{LiquidGlassButtonModifiers, LiquidPressShape};
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::storage::rise_storage_usage_presentation::RiseStorageUsagePresentation;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::user::stores::user_storage::user_storage_interactions::user_storage_interactions::UserStorageInteractionsStore;
use crate::modules::user::stores::user_storage::user_storage_services::user_storage_services::{
    RiseStorageSiteRow, UserStorageServicesStore,
};

const SKELETON_LINE_WIDTHS: [f32; 4] = [130.0, 96.0, 160.0, 110.0];

/// confirming flag and the storage store
struct CachedDatasState {
    confirming: Option<RiseStorageSiteRow>,
    _subscriptions: Vec<Subscription>,
}

impl CachedDatasState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseStorageUsagePresentation::shared(cx);
        Self {
            confirming: None,
            _subscriptions: vec![cx.observe(&presentation, |_, _, cx| cx.notify())],
        }
    }

    fn set_confirming(&mut self, confirming: Option<RiseStorageSiteRow>, cx: &mut Context<Self>) {
        self.confirming = confirming;
        cx.notify();
    }
}

fn symbol(name: &str, side: f32, color: Hsla) -> AnyElement {
    let filled = name.ends_with(".fill");
    match IconUi::sized(name, px(side), color, filled).or_else(|| IconUi::sized(name, px(side), color, !filled)) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}

#[derive(IntoElement)]
pub struct CachedDatas;

impl CachedDatas {
    pub fn new() -> Self {
        Self
    }

    fn group_skeleton(rows: usize) -> AnyElement {
        let mut column = vstack(0.0).w_full();
        for index in 0..rows {
            column = column.child(
                hstack(12.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .child(SkeletonBlock::new(29.0, 29.0, 8.0))
                    .child(SkeletonLine::new(SKELETON_LINE_WIDTHS[index % SKELETON_LINE_WIDTHS.len()], 10.0))
                    .child(spacer_min_w(0.0))
                    .child(SkeletonLine::new(52.0, 10.0)),
            );
        }
        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(12.0))
            .child(div().px(px(16.0)).child(SkeletonLine::new(108.0, 9.0)))
            .child(column)
            .into_any_element()
    }

    fn clearable_section(
        rows: Vec<RiseStorageSiteRow>,
        is_clearing: bool,
        state: &Entity<CachedDatasState>,
    ) -> Option<AnyElement> {
        if rows.is_empty() {
            return None;
        }
        let items = rows
            .into_iter()
            .map(|row| {
                let state = state.clone();
                GroupedBtn::new(row.site.raw_value(), tr(row.site.i18n_key()))
                    .icon(row.site.icon_name())
                    .icon_background(SettingsIconPalette::memory())
                    .trailing(RiseFormat::file_size(row.byte_count))
                    .disabled(is_clearing)
                    .callback(move |_, cx| {
                        let row = row.clone();
                        state.update(cx, |state, cx| state.set_confirming(Some(row), cx));
                    })
            })
            .collect();
        Some(
            div()
                .w_full()
                .child(GroupedBtns::new("cached-datas-clearable", items).title(tr("memory_settings_cache_section")))
                .into_any_element(),
        )
    }

    fn reserved_section(rows: Vec<RiseStorageSiteRow>, theme: &AppTheme) -> Option<AnyElement> {
        if rows.is_empty() {
            return None;
        }
        let items = rows
            .into_iter()
            .map(|row| {
                GroupedBtn::new(row.site.raw_value(), tr(row.site.i18n_key()))
                    .icon(row.site.icon_name())
                    .icon_background(theme.text.secondary.opacity(0.35))
                    .trailing(RiseFormat::file_size(row.byte_count))
                    .show_chevron(false)
            })
            .collect();
        Some(
            vstack(8.0)
                .align(HorizontalAlignment::Leading)
                .w_full()
                .child(GroupedBtns::new("cached-datas-reserved", items).title(tr("memory_settings_app_data")))
                .child(div().px(px(24.0)).child(SecondaryText::new(tr("memory_settings_app_data_hint")).px(12.0)))
                .into_any_element(),
        )
    }

    fn confirm_inner(row: &RiseStorageSiteRow) -> AnyElement {
        vstack(10.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .px(px(20.0))
            .pt(px(8.0))
            .child(MainText::new(tr("memory_settings_clear_dialog_title")).px(18.0).font_weight(FontWeight::SEMIBOLD))
            .child(
                SecondaryText::new(
                    tr("memory_settings_clear_dialog_body").replace("{size}", &RiseFormat::file_size(row.byte_count)),
                )
                .px(14.0),
            )
            .child(spacer_min_h(0.0))
            .into_any_element()
    }

    fn confirm_footer(state: Entity<CachedDatasState>, theme: &AppTheme) -> AnyElement {
        let action: ButtonAction = Rc::new(move |_: &mut Window, cx: &mut App| {
            let row = state.read(cx).confirming.clone();
            state.update(cx, |state, cx| state.set_confirming(None, cx));
            let Some(row) = row else {
                return;
            };
            UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_site(row.site, cx));
        });
        div()
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(
                hstack(8.0)
                    .w_full()
                    .justify_center()
                    .py(px(15.0))
                    .bg(theme.semantic.error_100)
                    .capsule()
                    .child(symbol("trash", 16.0, white()))
                    .child(
                        MainText::new(tr("memory_settings_clear_confirm"))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white()),
                    )
                    .liquid_press("cached-datas-clear-confirm", LiquidPressShape::RoundedRect(16.0), true, false, false, Some(action)),
            )
            .into_any_element()
    }
}

impl Default for CachedDatas {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for CachedDatas {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("cached-datas-state", cx, |_, cx| CachedDatasState::new(cx));
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let storage = UserStorageServicesStore::shared(cx);
        let (status, sites, is_clearing) =
            storage.update(cx, |storage, cx| (storage.status(cx), storage.other_data_sites(cx), storage.is_clearing(cx)));
        let is_empty = sites.is_empty();
        let (clearable, reserved): (Vec<RiseStorageSiteRow>, Vec<RiseStorageSiteRow>) =
            sites.into_iter().partition(|row| row.is_clearable);

        let content_state = state.clone();
        let content_theme = theme.clone();
        let content = move |_: &mut Window, _: &mut App| {
            vstack(20.0)
                .w_full()
                .pt(px(12.0))
                .children(Self::clearable_section(clearable, is_clearing, &content_state))
                .children(Self::reserved_section(reserved, &content_theme))
                .child(div().min_h(px(80.0)))
        };

        let presented_state = state.clone();
        let setter_state = state.clone();
        let sheet_state = state.clone();
        let footer_state = state;

        div()
            .relative()
            .size_full()
            .child(
                SettingsWrapperUi::new(tr("memory_settings_cached_datas"), move |_, cx| {
                    if let Some(nav_store) = &nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .content(
                    scroll_view("cached-datas-scroll")
                        .size_full()
                        .bg(theme.bg._100)
                        .list_shell_scroll_under_chrome(cx)
                        .child(
                            SaiDataRender::new(status, content)
                                .id("cached-datas-data")
                                .is_empty(is_empty)
                                .no_data_text(tr("memory_settings_nothing_cached"))
                                .sai_data_render_loading(|_, _| {
                                    vstack(20.0)
                                        .w_full()
                                        .pt(px(12.0))
                                        .child(Self::group_skeleton(4))
                                        .child(Self::group_skeleton(2))
                                        .skeleton_shimmer(None)
                                }),
                        ),
                )
                .on_appear("cached-datas-appear", |_, cx| {
                    UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.open_other_data(cx));
                }),
            )
            .child(
                BottomSheetUi::new(
                    "cached-datas-confirm",
                    Binding::new(
                        move |cx| presented_state.read(cx).confirming.is_some(),
                        move |presented, _, cx| {
                            if !presented {
                                setter_state.update(cx, |state, cx| state.set_confirming(None, cx));
                            }
                        },
                    ),
                    move |_, cx| match &sheet_state.read(cx).confirming {
                        Some(row) => Self::confirm_inner(row),
                        None => div().into_any_element(),
                    },
                    move |_, cx| {
                        let theme = rise_ui::theme(cx).clone();
                        Self::confirm_footer(footer_state.clone(), &theme)
                    },
                )
                .snap_fractions(vec![0.36])
                .footer_shows_top_border(false)
                .content_scrollable(false)
                .fits_content(true),
            )
    }
}
