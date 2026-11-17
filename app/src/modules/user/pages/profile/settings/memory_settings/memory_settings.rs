use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Bounds, Context, Entity, FontWeight, Hsla, PathBuilder, Pixels, Subscription, Task, Window, canvas, point, white};
use rise_i18n::tr;
use rise_platform::host_os::HostOs;
use rise_platform::reveal_in_file_manager::{reveal, reveal_label};
use rise_platform::AppPaths;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::liquid_glass_button::{LiquidGlassButtonModifiers, LiquidPressShape};
use rise_ui::lottie_view::LottieView;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::storage::rise_storage_usage_models::RiseStorageBucket;
use crate::core::storage::rise_storage_usage_presentation::RiseStorageUsagePresentation;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::user::stores::user_storage::user_storage_interactions::user_storage_interactions::UserStorageInteractionsStore;
use crate::modules::user::stores::user_storage::user_storage_services::user_storage_services::{
    RiseStorageBucketRow, UserStorageServicesStore,
};

const DONUT_LINE_WIDTH: f32 = 16.0;
const DONUT_STEPS_PER_TURN: f64 = 180.0;
const CELEBRATION_SECONDS: f64 = 2.0;

#[derive(IntoElement)]
struct MemoryDonut {
    segments: Vec<(Hsla, f32)>,
}

impl RenderOnce for MemoryDonut {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let segments = self.segments;
        canvas(|_, _, _| (), move |bounds, _, window, _| paint_memory_donut(bounds, &segments, window)).size_full()
    }
}

fn paint_memory_donut(bounds: Bounds<Pixels>, segments: &[(Hsla, f32)], window: &mut Window) {
    let side = f32::from(bounds.size.width).min(f32::from(bounds.size.height));
    let radius = f64::from(side / 2.0 - DONUT_LINE_WIDTH / 2.0);
    let center_x = f64::from(f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0);
    let center_y = f64::from(f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0);
    let mut start = -PI / 2.0;
    for (segment_color, fraction) in segments {
        let sweep = f64::from(*fraction) * 2.0 * PI;
        let steps = ((f64::from(*fraction) * DONUT_STEPS_PER_TURN).ceil() as usize).max(1);
        let vertices: Vec<_> = (0..=steps)
            .map(|step| {
                let angle = start + sweep * step as f64 / steps as f64;
                point(px((center_x + radius * angle.cos()) as f32), px((center_y + radius * angle.sin()) as f32))
            })
            .collect();
        let mut builder = PathBuilder::stroke(px(DONUT_LINE_WIDTH));
        builder.add_polygon(&vertices, false);
        if let Ok(path) = builder.build() {
            window.paint_path(path, *segment_color);
        }
        start += sweep;
    }
}

enum MemoryBucketStyle {}

impl MemoryBucketStyle {
    fn color(bucket: RiseStorageBucket) -> Hsla {
        match bucket {
            RiseStorageBucket::Videos => SettingsIconPalette::memory_videos(),
            RiseStorageBucket::Images => SettingsIconPalette::memory_images(),
            RiseStorageBucket::Files => SettingsIconPalette::memory_files(),
            RiseStorageBucket::Audio => SettingsIconPalette::memory_audio(),
            RiseStorageBucket::Other => SettingsIconPalette::memory_other(),
        }
    }
}

/// page state plus the storage store
struct MemorySettingsState {
    confirming_clear: bool,
    clear_celebration: bool,
    freed_bytes: i64,
    last_clear_completion_token: u64,
    celebration_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MemorySettingsState {
    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseStorageUsagePresentation::shared(cx);
        let last_clear_completion_token = presentation.read(cx).clear_completion_token();
        Self {
            confirming_clear: false,
            clear_celebration: false,
            freed_bytes: 0,
            last_clear_completion_token,
            celebration_task: None,
            _subscriptions: vec![cx.observe(&presentation, |state, presentation, cx| {
                state.clear_completion_changed(&presentation, cx);
                cx.notify();
            })],
        }
    }

    fn clear_completion_changed(&mut self, presentation: &Entity<RiseStorageUsagePresentation>, cx: &mut Context<Self>) {
        let token = presentation.read(cx).clear_completion_token();
        if token == self.last_clear_completion_token {
            return;
        }
        self.last_clear_completion_token = token;
        let freed = presentation.read(cx).last_clear_freed_bytes();
        if freed <= 0 {
            return;
        }
        self.freed_bytes = freed;
        self.clear_celebration = true;
        self.celebration_task = Some(cx.spawn(async move |state, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(CELEBRATION_SECONDS)).await;
            let _ = state.update(cx, |state, cx| {
                state.clear_celebration = false;
                cx.notify();
            });
        }));
    }
}

/// what the page reads from the storage services store in one pass
struct MemorySettingsSnapshot {
    rows: Vec<RiseStorageBucketRow>,
    total_bytes: i64,
    status: SaiWsSliceStatus,
    is_clearing: bool,
    reserved_bytes: i64,
    clearable_bytes: i64,
    has_clearable_bytes: bool,
    chats_bytes: i64,
    media_bytes: i64,
    other_bytes: i64,
}

impl MemorySettingsSnapshot {
    fn read(cx: &mut App) -> Self {
        UserStorageServicesStore::shared(cx).update(cx, |storage, cx| {
            let snapshot = storage.snapshot(cx);
            Self {
                rows: storage.occupied_buckets(cx),
                total_bytes: snapshot.total_bytes(),
                status: storage.status(cx),
                is_clearing: storage.is_clearing(cx),
                reserved_bytes: storage.reserved_bytes(cx),
                clearable_bytes: storage.clearable_bytes(cx),
                has_clearable_bytes: storage.has_clearable_bytes(cx),
                chats_bytes: storage.chat_cache_bytes(cx),
                media_bytes: UserStorageServicesStore::MEDIA_SITES.iter().map(|site| snapshot.bytes_site(*site)).sum(),
                other_bytes: storage.other_data_sites(cx).iter().map(|site| site.byte_count).sum(),
            }
        })
    }

    fn ring_segments(&self, theme: &AppTheme) -> Vec<(Hsla, f32)> {
        if self.total_bytes <= 0 {
            return Vec::new();
        }
        let total = self.total_bytes as f32;
        let mut segments: Vec<(Hsla, f32)> =
            self.rows.iter().map(|row| (MemoryBucketStyle::color(row.bucket), row.byte_count as f32 / total)).collect();
        if self.reserved_bytes > 0 {
            segments.push((theme.text.secondary.opacity(0.35), self.reserved_bytes as f32 / total));
        }
        segments
    }

    fn total_label(&self) -> String {
        match self.status {
            SaiWsSliceStatus::Idle | SaiWsSliceStatus::Pending => tr("memory_settings_calculating"),
            SaiWsSliceStatus::Rejected | SaiWsSliceStatus::Fulfilled => RiseFormat::file_size(self.total_bytes),
        }
    }
}

fn trailing_size(row: GroupedBtn, bytes: i64) -> GroupedBtn {
    if bytes > 0 { row.trailing(RiseFormat::file_size(bytes)) } else { row }
}

fn push_route(nav_store: &Option<Entity<crate::app::router::navigation_store::NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::default(), cx));
    }
}

fn reveal_cache_folder(cx: &mut App) {
    let Ok(paths) = AppPaths::resolve(AppEnvironment::compiled().canon()) else {
        return;
    };
    if paths.create_all().is_err() {
        return;
    }
    let _ = reveal(cx, paths.cache_dir());
}

#[derive(IntoElement)]
pub struct MemorySettings;

impl MemorySettings {
    pub fn new() -> Self {
        Self
    }

    fn donut_card(snapshot: &MemorySettingsSnapshot, theme: &AppTheme) -> impl IntoElement {
        let segments = snapshot.ring_segments(theme);
        div()
            .relative()
            .w_full()
            .child(
                vstack(12.0).w_full().py(px(18.0)).bg(theme.bg._200).clip_shape(Shape::continuous(14.0)).child(
                    zstack(Alignment::Center)
                        .size(px(168.0))
                        .child(
                            div().size(px(168.0)).child(MemoryDonut { segments }),
                        )
                        .child(
                            vstack(2.0)
                                .child(
                                    MainText::new(tr("memory_settings_total_label")).px(12.0).color(theme.text.secondary),
                                )
                                .child(MainText::new(snapshot.total_label()).px(22.0).font_weight(FontWeight::SEMIBOLD)),
                        ),
                ),
            )
            .child(centred_stroke(theme.border._200.opacity(0.55), 0.5, Shape::continuous(14.0)))
    }

    fn buckets_skeleton() -> impl IntoElement {
        const WIDTHS: [f32; 4] = [90.0, 120.0, 70.0, 100.0];
        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .px(px(12.0))
            .child(div().px(px(16.0)).child(SkeletonLine::new(96.0, 9.0)))
            .child(vstack(0.0).w_full().children(WIDTHS.into_iter().map(|width| {
                hstack(12.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .child(SkeletonBlock::new(29.0, 29.0, 8.0))
                    .child(SkeletonLine::new(width, 10.0))
                    .child(spacer_min_w(0.0))
                    .child(SkeletonLine::new(54.0, 10.0))
            })))
            .skeleton_shimmer(None)
    }

    fn clear_sheet_inner(theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
        let snapshot = MemorySettingsSnapshot::read(cx);
        vstack(10.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .px(px(20.0))
            .pt(px(8.0))
            .child(MainText::new(tr("memory_settings_clear_dialog_title")).px(18.0).font_weight(FontWeight::SEMIBOLD))
            .child(
                SecondaryText::new(
                    tr("memory_settings_clear_dialog_body")
                        .replace("{size}", &RiseFormat::file_size(snapshot.clearable_bytes)),
                )
                .px(14.0),
            )
            .when(snapshot.reserved_bytes > 0, |stack| {
                stack.child(
                    SecondaryText::new(format!(
                        "{}: {}",
                        tr("memory_settings_app_data"),
                        RiseFormat::file_size(snapshot.reserved_bytes)
                    ))
                    .px(13.0)
                    .color(theme.text.secondary.opacity(0.75)),
                )
            })
            .child(spacer_min_h(0.0))
    }

    fn clear_sheet_footer(state: &Entity<MemorySettingsState>, theme: &AppTheme) -> impl IntoElement + use<> {
        let state = state.clone();
        div().w_full().px(px(16.0)).py(px(12.0)).child(
            hstack(8.0)
                .w_full()
                .justify_center()
                .py(px(15.0))
                .bg(theme.semantic.error_100)
                .rounded_full()
                .children(IconUi::sized("trash", px(16.0), white(), false))
                .child(
                    MainText::new(tr("memory_settings_clear_confirm"))
                        .px(16.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(white()),
                )
                .liquid_press(
                    "memory-settings-clear-confirm",
                    LiquidPressShape::RoundedRect(16.0),
                    true,
                    false,
                    false,
                    Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                        state.update(cx, |state, cx| {
                            state.confirming_clear = false;
                            cx.notify();
                        });
                        UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_everything(cx));
                    })),
                ),
        )
    }

    fn celebration(freed_bytes: i64) -> impl IntoElement {
        zstack(Alignment::Center)
            .absolute()
            .size_full()
            .bg(gpui::black().opacity(0.35))
            .child(
                vstack(4.0)
                    .child(LottieView::new("party").size(160.0).looping(false))
                    .child(
                        MainText::new(tr("memory_settings_freed").replace("{size}", &RiseFormat::file_size(freed_bytes)))
                            .px(15.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white()),
                    ),
            )
    }
}

impl RenderOnce for MemorySettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state("memory-settings-state", cx, |_, cx| MemorySettingsState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let snapshot = MemorySettingsSnapshot::read(cx);
        let (clear_celebration, freed_bytes) = {
            let state = state.read(cx);
            (state.clear_celebration, state.freed_bytes)
        };

        let mut sections = vstack(20.0)
            .w_full()
            .pt(px(12.0))
            .child(div().w_full().px(px(12.0)).child(Self::donut_card(&snapshot, &theme)));

        if snapshot.status != SaiWsSliceStatus::Fulfilled && snapshot.rows.is_empty() {
            sections = sections.child(Self::buckets_skeleton());
        } else if snapshot.rows.is_empty() {
            sections = sections.child(
                div()
                    .w_full()
                    .py(px(24.0))
                    .child(SecondaryText::new(tr("memory_settings_nothing_cached")).px(14.0).tac(TextAlignT::Center).w_full()),
            );
        } else {
            sections = sections.child(
                GroupedBtns::new(
                    "memory-buckets-group",
                    snapshot
                        .rows
                        .iter()
                        .map(|row| {
                            GroupedBtn::new(row.bucket.raw_value(), tr(row.bucket.i18n_key()))
                                .icon(row.bucket.icon_name())
                                .icon_background(MemoryBucketStyle::color(row.bucket))
                                .trailing(RiseFormat::file_size(row.byte_count))
                                .show_chevron(false)
                        })
                        .collect(),
                )
                .title(tr("memory_settings_cache_section")),
            );
        }

        if snapshot.reserved_bytes > 0 {
            sections = sections.child(GroupedBtns::new(
                "memory-reserved-group",
                vec![
                    GroupedBtn::new("reserved", tr("memory_settings_app_data"))
                        .icon("externaldrive.fill")
                        .icon_background(theme.text.secondary.opacity(0.35))
                        .subtitle(tr("memory_settings_app_data_hint"))
                        .trailing(RiseFormat::file_size(snapshot.reserved_bytes))
                        .show_chevron(false),
                ],
            ));
        }

        let chats_nav = nav_store.clone();
        let media_nav = nav_store.clone();
        let datas_nav = nav_store.clone();
        sections = sections.child(GroupedBtns::new(
            "memory-cached-group",
            vec![
                trailing_size(
                    GroupedBtn::new("cchats", tr("memory_settings_cached_chats"))
                        .icon("bubble.left.and.bubble.right.fill")
                        .icon_background(SettingsIconPalette::privacy_friends()),
                    snapshot.chats_bytes,
                )
                .callback(move |_, cx| push_route(&chats_nav, RootRoute::CachedChats, cx)),
                trailing_size(
                    GroupedBtn::new("cmedia", tr("memory_settings_cached_media"))
                        .icon("photo.on.rectangle.angled")
                        .icon_background(SettingsIconPalette::memory_images()),
                    snapshot.media_bytes,
                )
                .callback(move |_, cx| push_route(&media_nav, RootRoute::CachedMedia, cx)),
                trailing_size(
                    GroupedBtn::new("cdatas", tr("memory_settings_cached_datas"))
                        .icon("externaldrive.fill")
                        .icon_background(SettingsIconPalette::memory()),
                    snapshot.other_bytes,
                )
                .callback(move |_, cx| push_route(&datas_nav, RootRoute::CachedDatas, cx)),
                GroupedBtn::new("creveal", tr(reveal_label(HostOs::current()).key))
                    .icon("folder.fill")
                    .icon_background(SettingsIconPalette::memory())
                    .show_chevron(false)
                    .callback(|_, cx| reveal_cache_folder(cx)),
            ],
        ));

        let confirm_state = state.clone();
        let clear_row = GroupedBtn::new(
            "clear",
            if snapshot.is_clearing { tr("memory_settings_clearing") } else { tr("memory_settings_clear_all") },
        )
        .icon("trash")
        .destructive(true)
        .disabled(snapshot.is_clearing || !snapshot.has_clearable_bytes)
        .show_chevron(false)
        .callback(move |_, cx| {
            confirm_state.update(cx, |state, cx| {
                state.confirming_clear = true;
                cx.notify();
            });
        });
        let clear_row = if snapshot.has_clearable_bytes {
            clear_row.trailing(RiseFormat::file_size(snapshot.clearable_bytes))
        } else {
            clear_row
        };
        sections = sections
            .child(GroupedBtns::new("memory-clear-group", vec![clear_row]))
            .child(div().flex_none().h(px(80.0)));

        let page = SettingsWrapperUi::new(tr("settings_memory_title"), move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view("memory-settings-scroll")
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .child(sections),
        )
        .on_appear("memory-settings-appear", |_, cx| {
            UserStorageInteractionsStore::shared(cx).update(cx, |store, cx| store.open_storage_settings(cx));
        });

        let inner_theme = theme.clone();
        let footer_theme = theme;
        let footer_state = state.clone();
        let sheet = BottomSheetUi::new(
            "memory-settings-clear-sheet",
            Binding::entity(
                state,
                |state: &MemorySettingsState| state.confirming_clear,
                |state: &mut MemorySettingsState, value, _| state.confirming_clear = value,
            ),
            move |_, cx| Self::clear_sheet_inner(&inner_theme, cx),
            move |_, _| Self::clear_sheet_footer(&footer_state, &footer_theme),
        )
        .snap_fractions(vec![0.42])
        .footer_shows_top_border(false)
        .content_scrollable(false)
        .fits_content(true);

        zstack(Alignment::Center)
            .size_full()
            .child(page)
            .when(clear_celebration, |stack| stack.child(Self::celebration(freed_bytes)))
            .child(sheet)
    }
}
