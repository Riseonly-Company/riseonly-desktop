#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyView, App, BoxShadow, Context, ElementId, Entity, FontWeight, Hsla, ListAlignment, ListOffset, ListState,
    PromptButton, PromptLevel, RenderOnce, SharedString, Window, black, list, point, rgb, white,
};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::keyboard_dismiss::KeyboardDismiss;
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::repeating_timer::RepeatingTimer;
use rise_ui::swiftui::rounded_gradient::rounded_gradient_fill;
use rise_ui::swiftui::segmented_picker::SegmentedPicker;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::UnitPoint;

use super::sai_api_docs_panel::SaiApiDocsPanel;
use super::sai_bottom_sheet_gallery_panel::{SaiBottomSheetGalleryPanel, SaiBottomSheetPreview};
use super::sai_clever_image_cache_debug_panel::SaiCleverImageCacheDebugPanel;
use super::sai_debug_storage_panel_views::{
    SaiDebugBodyFormat, SaiDebugExpandableMonospaceBlock, SaiDebugInspectorMetrics, SaiDebugStorageCardView,
    SaiDebugStorageEntry, SaiDebugUserDefaultsInspector, SaiDebugUserDefaultsPanelKind, SaiWsCacheDebugCardView,
};
use super::sai_debugger_overlay_state::SaiDebuggerOverlayState;
use super::sai_dev_host_panel::{SaiDevHostPanel, SaiDevHostSwitcher};
use super::sai_http_trace_card_view::SaiHttpTraceCardView;
use super::sai_http_trace_entry::SaiHttpTraceEntry;
use super::sai_http_trace_store::SaiHttpTraceStore;
use super::sai_music_playback_cache_debug_panel::SaiMusicPlaybackCacheDebugPanel;
use super::sai_performance_hud::{
    SaiFloatingDebugInitialPlacement, SaiFloatingDebugSurface, SaiPerformanceHUD, SaiPerformanceShareItem,
    SaiPerformanceShareSheet,
};
use super::sai_requester_panel::SaiRequesterPanel;
use super::sai_requester_store::SaiRequesterStore;
use super::sai_skeleton_gallery_panel::SaiSkeletonGalleryPanel;
use super::sai_voice_cache_debug_panel::SaiVoiceCacheDebugPanel;
use super::sai_ws_deferred_debug_panel::SaiWsDeferredDebugPanel;
use super::sai_ws_event_card_view::SaiWsEventCardView;
use super::sai_ws_event_store::{SaiWsEventEntry, SaiWsEventStore};
use super::sai_ws_ignored_requests_debug::SaiWsIgnoredRequestsPanel;
use crate::core::lib::sai::core::logger::sai_log_entry::SaiLogEntry;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::logger::sai_logger::SaiLogger;
use crate::core::lib::sai::core::sai_dev_host_override::{SaiDevHostEnvironment, SaiDevHostOverride, SaiDevHostSlot};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{
    SaiWebSocketAutoReconnectStatus, SaiWebSocketManager, SaiWebSocketManagerPhase,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::stores::http::http_service::HttpService;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationKind, BottomSheetDebugRegistrationModule,
};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;

const ORB_SIZE: f32 = 54.;
const ORB_ICON_SIDE: f32 = 22.;
const ORB_LED_SIDE: f32 = 12.;
const ORB_LED_OFFSET: f32 = 17.;
const LIST_OVERDRAW: f32 = 200.;
const LIST_PADDING: f32 = 14.;
const PANEL_MAX_HEIGHT: f32 = 500.;
const LOG_PINNED_SLACK: f32 = 28.;
const ROW_ICON_SIDE: f32 = 14.;

const DESTRUCTIVE_RED: u32 = 0xe11d48;
const LINK_BLUE: u32 = 0x29b6f6;
const PERF_GREEN: u32 = 0x4ade80;
const CACHE_AMBER: u32 = 0xffb020;
const CACHE_PURPLE: u32 = 0x9c27b0;
const LED_GREEN: u32 = 0x00db0f;
const LED_RED: u32 = 0xff4444;
const LED_GRAY: u32 = 0x888888;

thread_local! {
    static LOG_ENTRY_TIME_FORMATTER: DateFormatter = {
        let mut formatter = DateFormatter::new();
        formatter.set_date_format("HH:mm:ss.SSS");
        formatter
    };
    static CONNECTED_SINCE_FORMATTER: DateFormatter = {
        let mut formatter = DateFormatter::new();
        formatter.set_time_style(Style::Medium);
        formatter
    };
}

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn label(text: impl Into<SharedString>, font: Font, color: Hsla) -> Div {
    div().child(text.into()).swiftui_font(font).text_color(color)
}

fn caption_semibold() -> Font {
    Font::caption().weight(FontWeight::SEMIBOLD)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SaiDebuggerLogClearMode {
    All,
    SingleKind(SaiLogKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingStorageKeyDelete {
    kind: SaiDebugUserDefaultsPanelKind,
    identifier: String,
}

struct SaiDebuggerLogBottomPreferenceKey;

impl SaiDebuggerLogBottomPreferenceKey {
    const DEFAULT_VALUE: f32 = f32::MAX;

    fn reduce(value: &mut f32, next_value: impl FnOnce() -> f32) {
        *value = next_value();
    }
}

/// panel of the open tab, a tab switch drops its state
struct SaiDebuggerTabPanel {
    view: AnyView,
    set_props: Box<dyn Fn(AppTheme, i64, &mut App)>,
}

impl SaiDebuggerTabPanel {
    fn mount<V: Render>(
        entity: Entity<V>,
        set_props: impl Fn(&mut V, AppTheme, i64, &mut Context<V>) + 'static,
    ) -> Self {
        let target = entity.clone();
        Self {
            view: entity.into(),
            set_props: Box::new(move |theme, revision, cx| {
                target.update(cx, |view, cx| set_props(view, theme, revision, cx));
            }),
        }
    }
}

/// the orb, the performance hud and the debugger sheet above the whole app
pub struct SaiDebuggerHost {
    sheet_open: bool,
    pending_sheet_preview: Option<BottomSheetDebugRegistration>,
    active_sheet_preview: Option<BottomSheetDebugRegistration>,
    gallery_module: Option<BottomSheetDebugRegistrationModule>,
    gallery_query: String,
    gallery_kind: Option<BottomSheetDebugRegistrationKind>,
    overlay_state: Entity<SaiDebuggerOverlayState>,
    logger: Entity<SaiLogger>,
    http_trace: Entity<SaiHttpTraceStore>,
    ws_events: Entity<SaiWsEventStore>,
    requester: Entity<SaiRequesterStore>,
    socket: Entity<SaiWebSocketManager>,
    observed_tab_jump_tick: i64,
    tab_index: i64,
    storage_snapshot_tick: i64,
    ws_cache_snapshot_tick: i64,
    clever_image_cache_snapshot_tick: i64,
    pending_storage_key_delete: Option<PendingStorageKeyDelete>,
    clear_all_confirmation_kind: Option<SaiDebugUserDefaultsPanelKind>,
    http_trace_row_delete_id: Option<Uuid>,
    http_trace_clear_all_confirm: bool,
    shows_all_fetch_activity: bool,
    ws_event_row_delete_id: Option<Uuid>,
    ws_event_clear_all_confirm: bool,
    ws_cache_key_delete: Option<String>,
    ws_cache_clear_all_confirm: bool,
    clever_image_cache_delete_digest: Option<String>,
    clever_image_cache_clear_all_confirm: bool,
    music_cache_snapshot_tick: i64,
    music_cache_track_delete_id: Option<String>,
    music_cache_clear_all_confirm: bool,
    voice_cache_snapshot_tick: i64,
    voice_cache_message_delete_id: Option<String>,
    voice_cache_chat_delete_id: Option<String>,
    voice_cache_clear_all_confirm: bool,
    log_entry_delete_id: Option<Uuid>,
    log_clear_all_pending: Option<SaiDebuggerLogClearMode>,
    log_list_pinned_to_bottom: bool,
    performance_share_item: Option<SaiPerformanceShareItem>,
    pending_host_switch: Option<SaiDevHostSlot>,
    host_switch_revision: i64,

    sheet_header: Option<Entity<SaiDebuggerSheetHeader>>,
    active_panel: Option<SaiDebuggerTabPanel>,
    visible_http_entries: Rc<Vec<SaiHttpTraceEntry>>,
    http_trace_total_count: usize,
    http_trace_network_count: usize,
    ws_event_entries: Rc<Vec<SaiWsEventEntry>>,
    ws_cache_keys: Rc<Vec<String>>,
    storage_entries: Rc<Vec<SaiDebugStorageEntry>>,
    log_items: Rc<Vec<SaiLogEntry>>,
    fetches_list: ListState,
    events_list: ListState,
    ws_cache_list: ListState,
    storage_list: ListState,
    logs_list: ListState,
}

impl SaiDebuggerHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let overlay_state = SaiDebuggerOverlayState::shared(cx);
        let logger = SaiLogger::shared(cx);
        let http_trace = SaiHttpTraceStore::shared(cx);
        let ws_events = SaiWsEventStore::shared(cx);
        let requester = SaiRequesterStore::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        let theme_store = ThemeStore::shared(cx);

        cx.observe(&overlay_state, |host, overlay_state, cx| {
            if !overlay_state.read(cx).is_orb_visible() {
                host.cancel_sheet_preview();
                host.sheet_open = false;
            }
            cx.notify();
        })
        .detach();
        cx.observe(&logger, |host, _, cx| {
            if host.sheet_open && host.tab_index == 9 {
                host.sync_logs(false, cx);
                cx.notify();
            }
        })
        .detach();
        cx.observe(&http_trace, |host, _, cx| {
            if host.sheet_open && host.tab_index == 0 {
                host.sync_fetches(cx);
                cx.notify();
            }
        })
        .detach();
        cx.observe(&ws_events, |host, _, cx| {
            if host.sheet_open && host.tab_index == 8 {
                host.sync_events(cx);
                cx.notify();
            }
        })
        .detach();
        cx.observe(&requester, |host, _, cx| {
            let tick = host.requester.read(cx).tab_jump_tick();
            if host.observed_tab_jump_tick != tick {
                host.observed_tab_jump_tick = tick;
                host.set_tab_index(12, cx);
            }
        })
        .detach();
        cx.observe(&socket, |host, _, cx| {
            if host.overlay_state.read(cx).is_orb_visible() {
                cx.notify();
            }
        })
        .detach();
        cx.observe(&theme_store, |host, _, cx| {
            host.propagate_panel_props(cx);
            cx.notify();
        })
        .detach();

        let logs_list = ListState::new(0, ListAlignment::Bottom, px(LIST_OVERDRAW));
        let weak_host = cx.weak_entity();
        logs_list.set_scroll_handler(move |_, _, cx| {
            let weak_host = weak_host.clone();
            cx.defer(move |cx| {
                weak_host.update(cx, |host, _| host.update_log_list_pinned()).ok();
            });
        });

        let observed_tab_jump_tick = requester.read(cx).tab_jump_tick();

        Self {
            sheet_open: false,
            pending_sheet_preview: None,
            active_sheet_preview: None,
            gallery_module: None,
            gallery_query: String::new(),
            gallery_kind: None,
            overlay_state,
            logger,
            http_trace,
            ws_events,
            requester,
            socket,
            observed_tab_jump_tick,
            tab_index: 0,
            storage_snapshot_tick: 0,
            ws_cache_snapshot_tick: 0,
            clever_image_cache_snapshot_tick: 0,
            pending_storage_key_delete: None,
            clear_all_confirmation_kind: None,
            http_trace_row_delete_id: None,
            http_trace_clear_all_confirm: false,
            shows_all_fetch_activity: false,
            ws_event_row_delete_id: None,
            ws_event_clear_all_confirm: false,
            ws_cache_key_delete: None,
            ws_cache_clear_all_confirm: false,
            clever_image_cache_delete_digest: None,
            clever_image_cache_clear_all_confirm: false,
            music_cache_snapshot_tick: 0,
            music_cache_track_delete_id: None,
            music_cache_clear_all_confirm: false,
            voice_cache_snapshot_tick: 0,
            voice_cache_message_delete_id: None,
            voice_cache_chat_delete_id: None,
            voice_cache_clear_all_confirm: false,
            log_entry_delete_id: None,
            log_clear_all_pending: None,
            log_list_pinned_to_bottom: true,
            performance_share_item: None,
            pending_host_switch: None,
            host_switch_revision: 0,
            sheet_header: None,
            active_panel: None,
            visible_http_entries: Rc::new(Vec::new()),
            http_trace_total_count: 0,
            http_trace_network_count: 0,
            ws_event_entries: Rc::new(Vec::new()),
            ws_cache_keys: Rc::new(Vec::new()),
            storage_entries: Rc::new(Vec::new()),
            log_items: Rc::new(Vec::new()),
            fetches_list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            events_list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            ws_cache_list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            storage_list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            logs_list,
        }
    }

    fn debugger_tabs() -> [(i64, SharedString); 16] {
        [
            (0, "Fetches".into()),
            (1, "LocalStorage".into()),
            (2, "DiskCache".into()),
            (3, "Images".into()),
            (4, "Music".into()),
            (5, "VoiceCache".into()),
            (6, "WS cache".into()),
            (7, "WS debt".into()),
            (8, "Events".into()),
            (9, "Logs".into()),
            (10, "Ignored WS".into()),
            (11, "Docs".into()),
            (12, "Requester".into()),
            (13, "Skeletons".into()),
            (14, "Host".into()),
            (15, tr("debug_sheet_title").into()),
        ]
    }

    fn cancel_sheet_preview(&mut self) {
        self.pending_sheet_preview = None;
        if let Some(entry) = self.active_sheet_preview.take() {
            (entry.cancel)();
        }
    }

    fn set_tab_index(&mut self, index: i64, cx: &mut Context<Self>) {
        self.tab_index = index;
        self.active_panel = None;
        self.sync_tab_snapshot(cx);
        cx.notify();
    }

    fn sync_tab_snapshot(&mut self, cx: &mut Context<Self>) {
        match self.tab_index {
            0 => self.sync_fetches(cx),
            1 => self.sync_storage(SaiDebugUserDefaultsPanelKind::LocalStorage),
            2 => self.sync_storage(SaiDebugUserDefaultsPanelKind::ApplicationSupportWs),
            6 => self.sync_ws_cache(cx),
            8 => self.sync_events(cx),
            9 => self.sync_logs(true, cx),
            _ => {}
        }
    }

    fn sync_fetches(&mut self, cx: &mut Context<Self>) {
        let entries = self.http_trace.read(cx).entries();
        let network: Vec<SaiHttpTraceEntry> =
            entries.iter().filter(|entry| entry.is_network_request()).cloned().collect();
        self.http_trace_total_count = entries.len();
        self.http_trace_network_count = network.len();
        let next = if self.shows_all_fetch_activity { entries.to_vec() } else { network };
        reset_list_keeping_anchor(&self.fetches_list, &self.visible_http_entries, &next, |entry| entry.id);
        self.visible_http_entries = Rc::new(next);
    }

    fn sync_events(&mut self, cx: &mut Context<Self>) {
        let next = self.ws_events.read(cx).entries().to_vec();
        reset_list_keeping_anchor(&self.events_list, &self.ws_event_entries, &next, |entry| entry.id);
        self.ws_event_entries = Rc::new(next);
    }

    fn sync_ws_cache(&mut self, cx: &mut Context<Self>) {
        let keys = SaiWsResponseCache::debug_all_listed_cache_keys(cx);
        self.ws_cache_list.reset(keys.len());
        self.ws_cache_keys = Rc::new(keys);
    }

    fn sync_storage(&mut self, kind: SaiDebugUserDefaultsPanelKind) {
        let entries = SaiDebugUserDefaultsInspector::entries(kind);
        self.storage_list.reset(entries.len());
        self.storage_entries = Rc::new(entries);
    }

    fn sync_logs(&mut self, appeared: bool, cx: &mut Context<Self>) {
        let mut next = self.logger.read(cx).entries();
        next.reverse();
        if appeared {
            self.log_list_pinned_to_bottom = true;
        }
        if self.log_list_pinned_to_bottom {
            self.logs_list.reset(next.len());
            self.logs_list.scroll_to_end();
        } else {
            reset_list_keeping_anchor(&self.logs_list, &self.log_items, &next, |entry| entry.id);
        }
        self.log_items = Rc::new(next);
    }

    fn update_log_list_pinned(&mut self) {
        let viewport_height = f32::from(self.logs_list.viewport_bounds().size.height);
        let max_offset = f32::from(self.logs_list.max_offset_for_scrollbar().y);
        let scrolled = -f32::from(self.logs_list.scroll_px_offset_for_scrollbar().y);
        let mut bottom_y = SaiDebuggerLogBottomPreferenceKey::DEFAULT_VALUE;
        SaiDebuggerLogBottomPreferenceKey::reduce(&mut bottom_y, || {
            viewport_height + (max_offset - scrolled) - LIST_PADDING
        });
        self.log_list_pinned_to_bottom = bottom_y <= viewport_height + LOG_PINNED_SLACK;
    }

    fn propagate_panel_props(&mut self, cx: &mut Context<Self>) {
        let theme = rise_ui::theme(cx).clone();
        let revision = self.host_switch_revision;
        if let Some(header) = &self.sheet_header {
            header.update(cx, |header, cx| header.set_props(theme.clone(), revision, cx));
        }
        if let Some(panel) = &self.active_panel {
            (panel.set_props)(theme, revision, cx);
        }
    }

    /// sessions follow the host so a switch is a single tap
    /// production always asks since a mis-tap writes to real accounts
    fn request_host_switch(&mut self, slot: SaiDevHostSlot, window: &mut Window, cx: &mut Context<Self>) {
        if !slot.is_live() {
            self.apply_host_switch(slot, cx);
            return;
        }
        self.pending_host_switch = Some(slot);
        let message = Self::host_switch_message(slot, cx);
        self.present_confirmation_dialog(
            "Point this build at production?",
            Some(message),
            "Switch to production",
            |host, cx| {
                if let Some(slot) = host.pending_host_switch {
                    host.apply_host_switch(slot, cx);
                }
            },
            |host, cx| host.cancel_host_switch(cx),
            window,
            cx,
        );
    }

    fn host_switch_message(slot: SaiDevHostSlot, cx: &mut App) -> String {
        let mut paragraphs = vec![
            HttpService::debug_api_base_url(slot).absolute_string().to_owned(),
            "This is the live backend. Anything this build sends from here lands on real accounts, and a debug build sends more than a shipped one does.".to_owned(),
        ];

        let backend_id = SaiDevHostSwitcher::backend_id(slot);
        if let Some(account) = AuthServicesStore::debug_first_account(&backend_id) {
            let handle = account
                .user
                .tag
                .map(|tag| format!("@{tag}"))
                .or(account.user.name)
                .unwrap_or(account.user.id);
            paragraphs.push(format!("Continues as {handle}, the most recent account issued there."));
        } else if AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            paragraphs.push(
                "No account stored for it: the sign-in screen opens and the current session stays parked for the way back."
                    .to_owned(),
            );
        }

        paragraphs.join("\n\n")
    }

    fn apply_host_switch(&mut self, slot: SaiDevHostSlot, cx: &mut Context<Self>) {
        SaiDevHostSwitcher::apply(slot, cx);
        self.pending_host_switch = None;
        self.bump_host_switch_revision(cx);
    }

    fn cancel_host_switch(&mut self, cx: &mut Context<Self>) {
        self.pending_host_switch = None;
        self.bump_host_switch_revision(cx);
    }

    fn bump_host_switch_revision(&mut self, cx: &mut Context<Self>) {
        self.host_switch_revision += 1;
        self.propagate_panel_props(cx);
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn present_confirmation_dialog(
        &mut self,
        title: &str,
        message: Option<String>,
        confirm_title: impl Into<SharedString>,
        on_confirm: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        on_cancel: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
        let answer = window.prompt(
            PromptLevel::Warning,
            title,
            message.as_deref(),
            &[PromptButton::Ok(confirm_title.into()), PromptButton::Cancel("Cancel".into())],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let choice = answer.await;
            this.update(cx, |host, cx| {
                if choice == Ok(0) {
                    on_confirm(host, cx);
                } else {
                    on_cancel(host, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn request_storage_key_delete(
        &mut self,
        kind: SaiDebugUserDefaultsPanelKind,
        identifier: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = match kind {
            SaiDebugUserDefaultsPanelKind::LocalStorage => identifier.clone(),
            SaiDebugUserDefaultsPanelKind::ApplicationSupportWs => format!("SHA256 file id: {identifier}"),
        };
        self.pending_storage_key_delete = Some(PendingStorageKeyDelete { kind, identifier });
        self.present_confirmation_dialog(
            "Remove this entry?",
            Some(message),
            "Remove",
            |host, cx| {
                if let Some(pending) = host.pending_storage_key_delete.take() {
                    SaiDebugUserDefaultsInspector::remove_key(&pending.identifier, pending.kind, cx);
                    host.storage_snapshot_tick += 1;
                    if pending.kind == SaiDebugUserDefaultsPanelKind::ApplicationSupportWs {
                        host.ws_cache_snapshot_tick += 1;
                    }
                    host.sync_tab_snapshot(cx);
                }
            },
            |host, _| host.pending_storage_key_delete = None,
            window,
            cx,
        );
    }

    fn request_storage_clear_all(
        &mut self,
        kind: SaiDebugUserDefaultsPanelKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_all_confirmation_kind = Some(kind);
        let count = SaiDebugUserDefaultsInspector::filtered_keys(kind).len();
        self.present_confirmation_dialog(
            "Clear all in this tab?",
            Some(Self::storage_clear_all_message(kind).to_owned()),
            format!("Remove {count} entries"),
            |host, cx| {
                if let Some(kind) = host.clear_all_confirmation_kind.take() {
                    SaiDebugUserDefaultsInspector::remove_all_keys(kind, cx);
                    host.storage_snapshot_tick += 1;
                    if kind == SaiDebugUserDefaultsPanelKind::ApplicationSupportWs {
                        host.ws_cache_snapshot_tick += 1;
                    }
                    host.sync_tab_snapshot(cx);
                }
            },
            |host, _| host.clear_all_confirmation_kind = None,
            window,
            cx,
        );
    }

    fn request_http_trace_row_delete(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.http_trace_row_delete_id = Some(id);
        self.present_confirmation_dialog(
            "Delete this fetch row?",
            Some("Removes this row from Fetches.".to_owned()),
            "Delete",
            |host, cx| {
                if let Some(id) = host.http_trace_row_delete_id.take() {
                    host.http_trace.update(cx, |store, cx| store.remove_entry(id, cx));
                }
            },
            |host, _| host.http_trace_row_delete_id = None,
            window,
            cx,
        );
    }

    fn request_http_trace_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.http_trace_clear_all_confirm {
            return;
        }
        self.http_trace_clear_all_confirm = true;
        self.present_confirmation_dialog(
            "Clear all fetches?",
            Some("Removes every HTTP / WebSocket trace row.".to_owned()),
            "Clear all",
            |host, cx| {
                host.http_trace.update(cx, |store, cx| store.clear(cx));
                host.http_trace_clear_all_confirm = false;
            },
            |host, _| host.http_trace_clear_all_confirm = false,
            window,
            cx,
        );
    }

    fn request_ws_event_row_delete(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.ws_event_row_delete_id = Some(id);
        self.present_confirmation_dialog(
            "Delete this event row?",
            Some("Removes this row from Events.".to_owned()),
            "Delete",
            |host, cx| {
                if let Some(id) = host.ws_event_row_delete_id.take() {
                    host.ws_events.update(cx, |store, cx| store.remove_entry(id, cx));
                }
            },
            |host, _| host.ws_event_row_delete_id = None,
            window,
            cx,
        );
    }

    fn request_ws_event_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ws_event_clear_all_confirm {
            return;
        }
        self.ws_event_clear_all_confirm = true;
        self.present_confirmation_dialog(
            "Clear all events?",
            Some("Removes every received WebSocket event.".to_owned()),
            "Clear all",
            |host, cx| {
                host.ws_events.update(cx, |store, cx| store.clear(cx));
                host.ws_event_clear_all_confirm = false;
            },
            |host, _| host.ws_event_clear_all_confirm = false,
            window,
            cx,
        );
    }

    fn request_ws_cache_key_delete(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        self.ws_cache_key_delete = Some(key);
        self.present_confirmation_dialog(
            "Remove this WS cache entry?",
            Some("Clears memory + on-disk payload for this cache key.".to_owned()),
            "Remove",
            |host, cx| {
                if let Some(key) = host.ws_cache_key_delete.take() {
                    SaiWsResponseCache::remove(&key, SaiCacheCleanLayer::Everywhere, cx);
                    host.ws_cache_snapshot_tick += 1;
                    host.sync_ws_cache(cx);
                }
            },
            |host, _| host.ws_cache_key_delete = None,
            window,
            cx,
        );
    }

    fn request_ws_cache_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ws_cache_clear_all_confirm {
            return;
        }
        self.ws_cache_clear_all_confirm = true;
        self.present_confirmation_dialog(
            "Clear entire WS cache?",
            Some("Clears in-memory WS response cache and all Application Support payload files.".to_owned()),
            "Clear all",
            |host, cx| {
                SaiWsResponseCache::clear_all_for_debug(cx);
                host.ws_cache_clear_all_confirm = false;
                host.ws_cache_snapshot_tick += 1;
                host.sync_ws_cache(cx);
            },
            |host, _| host.ws_cache_clear_all_confirm = false,
            window,
            cx,
        );
    }

    fn request_clever_image_cache_delete(&mut self, digest: String, window: &mut Window, cx: &mut Context<Self>) {
        self.clever_image_cache_delete_digest = Some(digest);
        self.present_confirmation_dialog(
            "Remove this image cache entry?",
            Some("Deletes the on-disk file and evicts memory for this URL.".to_owned()),
            "Remove",
            |host, _| {
                if let Some(hex) = host.clever_image_cache_delete_digest.take() {
                    let user_id = SaiAccountCacheScope::active_user_id();
                    CleverImageDiskCache::remove_digest_hex(&hex, user_id.as_deref(), None);
                    host.clever_image_cache_snapshot_tick += 1;
                }
            },
            |host, _| host.clever_image_cache_delete_digest = None,
            window,
            cx,
        );
    }

    fn request_clever_image_cache_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clever_image_cache_clear_all_confirm {
            return;
        }
        self.clever_image_cache_clear_all_confirm = true;
        self.present_confirmation_dialog(
            "Clear entire CleverImage disk cache?",
            Some("Removes every file under Application Support / CleverImageCache.".to_owned()),
            "Clear all",
            |host, _| {
                let user_id = SaiAccountCacheScope::active_user_id();
                CleverImageDiskCache::clear_all(user_id.as_deref());
                host.clever_image_cache_clear_all_confirm = false;
                host.clever_image_cache_snapshot_tick += 1;
            },
            |host, _| host.clever_image_cache_clear_all_confirm = false,
            window,
            cx,
        );
    }

    fn request_music_cache_track_delete(&mut self, track_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.music_cache_track_delete_id = Some(track_id);
        self.present_confirmation_dialog(
            "Remove this music cache entry?",
            Some("Deletes the cached audio file for this track id.".to_owned()),
            "Remove",
            |host, _| {
                if let Some(track_id) = host.music_cache_track_delete_id.take() {
                    let user_id = SaiAccountCacheScope::active_user_id();
                    MusicPlaybackDiskCache::clear_track(&track_id, user_id.as_deref());
                    host.music_cache_snapshot_tick += 1;
                }
            },
            |host, _| host.music_cache_track_delete_id = None,
            window,
            cx,
        );
    }

    fn request_music_cache_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.music_cache_clear_all_confirm {
            return;
        }
        self.music_cache_clear_all_confirm = true;
        self.present_confirmation_dialog(
            "Clear entire music playback cache?",
            Some("Removes every file under Application Support / MusicPlaybackCache.".to_owned()),
            "Clear all",
            |host, _| {
                let user_id = SaiAccountCacheScope::active_user_id();
                MusicPlaybackDiskCache::clear_all(user_id.as_deref());
                host.music_cache_clear_all_confirm = false;
                host.music_cache_snapshot_tick += 1;
            },
            |host, _| host.music_cache_clear_all_confirm = false,
            window,
            cx,
        );
    }

    fn request_voice_cache_message_delete(&mut self, message_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.voice_cache_message_delete_id = Some(message_id);
        self.present_confirmation_dialog(
            "Remove this voice cache entry?",
            Some("Deletes the cached voice file for this message id.".to_owned()),
            "Remove",
            |host, _| {
                if let Some(message_id) = host.voice_cache_message_delete_id.take() {
                    VoiceMessageDiskCache::clear_message(&message_id);
                    host.voice_cache_snapshot_tick += 1;
                }
            },
            |host, _| host.voice_cache_message_delete_id = None,
            window,
            cx,
        );
    }

    fn request_voice_cache_chat_delete(&mut self, chat_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.voice_cache_chat_delete_id = Some(chat_id);
        self.present_confirmation_dialog(
            "Remove all voice cache for this chat?",
            Some("Deletes every cached voice message for this chat id.".to_owned()),
            "Remove chat",
            |host, _| {
                if let Some(chat_id) = host.voice_cache_chat_delete_id.take() {
                    VoiceMessageDiskCache::clear_chat(&chat_id);
                    host.voice_cache_snapshot_tick += 1;
                }
            },
            |host, _| host.voice_cache_chat_delete_id = None,
            window,
            cx,
        );
    }

    fn request_voice_cache_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.voice_cache_clear_all_confirm {
            return;
        }
        self.voice_cache_clear_all_confirm = true;
        self.present_confirmation_dialog(
            "Clear entire voice message cache?",
            Some("Removes every file under Application Support / VoiceMessageCache.".to_owned()),
            "Clear all",
            |host, _| {
                let user_id = SaiAccountCacheScope::active_user_id();
                VoiceMessageDiskCache::clear_all(user_id.as_deref());
                host.voice_cache_clear_all_confirm = false;
                host.voice_cache_snapshot_tick += 1;
            },
            |host, _| host.voice_cache_clear_all_confirm = false,
            window,
            cx,
        );
    }

    fn request_log_entry_delete(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.log_entry_delete_id = Some(id);
        self.present_confirmation_dialog(
            "Delete this log row?",
            Some("Removes one line from the ring buffer.".to_owned()),
            "Delete",
            |host, cx| {
                if let Some(id) = host.log_entry_delete_id.take() {
                    host.logger.update(cx, |logger, cx| logger.remove_entry(id, cx));
                }
            },
            |host, _| host.log_entry_delete_id = None,
            window,
            cx,
        );
    }

    fn request_log_clear_all(&mut self, mode: SaiDebuggerLogClearMode, window: &mut Window, cx: &mut Context<Self>) {
        let message = Self::log_clear_all_dialog_message(Some(&mode));
        self.log_clear_all_pending = Some(mode);
        self.present_confirmation_dialog(
            "Clear logs in this tab?",
            Some(message),
            "Clear all",
            |host, cx| {
                if let Some(mode) = host.log_clear_all_pending.take() {
                    host.logger.update(cx, |logger, cx| match &mode {
                        SaiDebuggerLogClearMode::All => logger.clear(cx),
                        SaiDebuggerLogClearMode::SingleKind(kind) => logger.remove_all(kind, cx),
                    });
                }
            },
            |host, _| host.log_clear_all_pending = None,
            window,
            cx,
        );
    }

    fn storage_empty_message(kind: SaiDebugUserDefaultsPanelKind) -> &'static str {
        match kind {
            SaiDebugUserDefaultsPanelKind::LocalStorage => "No matching keys",
            SaiDebugUserDefaultsPanelKind::ApplicationSupportWs => {
                "No WS payload JSON files yet. `persistCacheToDisk` writes here (not UserDefaults)."
            }
        }
    }

    fn storage_clear_all_message(kind: SaiDebugUserDefaultsPanelKind) -> &'static str {
        match kind {
            SaiDebugUserDefaultsPanelKind::LocalStorage => {
                "Deletes only keys listed under LocalStorage (not ws_cache / not system keys)."
            }
            SaiDebugUserDefaultsPanelKind::ApplicationSupportWs => {
                "Deletes every `SaiWsDiskCache` JSON payload and the digest index under Application Support (same as WS cache \u{2192} Clear all). In-memory WS entries are cleared on the main actor."
            }
        }
    }

    fn log_clear_all_dialog_message(mode: Option<&SaiDebuggerLogClearMode>) -> String {
        match mode {
            None => String::new(),
            Some(SaiDebuggerLogClearMode::All) => "Removes every row in the Logs ring buffer.".to_owned(),
            Some(SaiDebuggerLogClearMode::SingleKind(kind)) => {
                format!("Removes all entries with kind \u{201c}{}\u{201d}.", kind.short_label())
            }
        }
    }

    fn open_sheet(&mut self, cx: &mut Context<Self>) {
        self.cancel_sheet_preview();
        self.sheet_open = true;
        self.sync_tab_snapshot(cx);
        cx.notify();
    }

    pub fn is_sheet_open(&self) -> bool {
        self.sheet_open
    }

    pub fn toggle_from_shortcut(&mut self, cx: &mut Context<Self>) {
        if self.sheet_open {
            self.sheet_open = false;
            cx.notify();
            return;
        }
        self.overlay_state.update(cx, |state, cx| state.show_orb(cx));
        self.open_sheet(cx);
    }

    fn orb(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.weak_entity();
        let menu_items = self.socket_context_menu_entries(cx);
        let led_color = sai_debugger_led_color(self.socket.read(cx));
        let [gradient_start, gradient_end] = sai_debugger_orb_colors(SaiDevHostOverride::slot());
        let led_left = ORB_SIZE / 2. + ORB_LED_OFFSET - ORB_LED_SIDE / 2.;

        let open_this = this.clone();
        let button = plain("sai-debugger-orb-button")
            .on_press(move |_, cx| {
                open_this.update(cx, |host, cx| host.open_sheet(cx)).ok();
            })
            .size(px(ORB_SIZE))
            .child(
                div()
                    .relative()
                    .size(px(ORB_SIZE))
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .rounded_full()
                            .shadow(vec![BoxShadow {
                                color: black().opacity(0.35),
                                offset: point(px(0.), px(4.)),
                                blur_radius: px(10.),
                                spread_radius: px(0.),
                                inset: false,
                            }])
                            .child(rounded_gradient_fill(
                                LinearGradient::colors(
                                    [gradient_start, gradient_end],
                                    UnitPoint::TOP_LEADING,
                                    UnitPoint::BOTTOM_TRAILING,
                                ),
                                ORB_SIZE / 2.,
                            )),
                    )
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized("ant.fill", px(ORB_ICON_SIDE), white().opacity(0.95), true)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(led_left))
                            .top(px(led_left))
                            .size(px(ORB_LED_SIDE))
                            .rounded_full()
                            .bg(led_color)
                            .child(centred_stroke(white().opacity(0.9), 1.5, Shape::Circle)),
                    ),
            )
            .context_menu("sai-debugger-orb-menu", menu_items);

        let dismiss_this = this;
        SaiFloatingDebugSurface::new(
            "sai-debugger-orb",
            (ORB_SIZE, ORB_SIZE),
            SaiFloatingDebugInitialPlacement::BottomLeading,
            ORB_SIZE / 2.,
            move |_, cx| {
                dismiss_this
                    .update(cx, |host, cx| {
                        host.sheet_open = false;
                        host.overlay_state.update(cx, |state, cx| state.dismiss_orb(cx));
                        cx.notify();
                    })
                    .ok();
            },
            div()
                .id("sai-debugger-orb-label")
                .occlude()
                .size(px(ORB_SIZE))
                .tooltip_label("Open Sai debugger")
                .child(button),
        )
        .into_any_element()
    }

    fn socket_context_menu_entries(&mut self, cx: &mut Context<Self>) -> Vec<ContextMenuItem> {
        let socket = self.socket.read(cx);
        let display_url = socket.active_url_display_string();
        let is_connected = matches!(socket.phase(), SaiWebSocketManagerPhase::Connected(..));
        let status_lines: Vec<String> = match socket.phase() {
            SaiWebSocketManagerPhase::Connected(_, since) => {
                let since = CONNECTED_SINCE_FORMATTER.with(|formatter| formatter.string_from(since));
                vec![format!("Connected since {since}"), display_url.clone()]
            }
            SaiWebSocketManagerPhase::Connecting(url) => {
                vec!["Connecting\u{2026}".to_owned(), url.absolute_string().to_owned()]
            }
            SaiWebSocketManagerPhase::Failed(message) => vec![format!("Error: {message}")],
            _ => vec!["Idle / disconnected".to_owned(), display_url.clone()],
        };

        let mut items: Vec<ContextMenuItem> = Vec::new();
        for (index, line) in status_lines.into_iter().enumerate() {
            items.push(
                ContextMenuItem::new(index as i64, line, None, format!("status-{index}"), |_, _| {}).disabled(true),
            );
        }

        if SaiDevHostOverride::is_effective() {
            let current = SaiDevHostOverride::slot();
            items.push(
                ContextMenuItem::new(
                    10,
                    format!("Host: {}", current.title()),
                    Some("globe".to_owned()),
                    "host-label",
                    |_, _| {},
                )
                .disabled(true),
            );
            for (index, slot) in SaiDevHostSlot::ALL_CASES.into_iter().enumerate() {
                if !SaiDevHostSwitcher::is_reachable(slot) {
                    continue;
                }
                let this = cx.weak_entity();
                let icon = if current == slot { "largecircle.fill.circle" } else { "circle" };
                items.push(ContextMenuItem::new(
                    11 + index as i64,
                    slot.title(),
                    Some(icon.to_owned()),
                    slot.raw_value(),
                    move |window, cx| {
                        this.update(cx, |host, cx| host.request_host_switch(slot, window, cx)).ok();
                    },
                ));
            }
        }

        items.push(ContextMenuItem::new(
            20,
            "Copy WebSocket URL",
            Some("doc.on.doc".to_owned()),
            "copy-url",
            move |_, cx| UIPasteboard::general().set_string(&display_url, cx),
        ));

        let socket = self.socket.clone();
        if is_connected {
            items.push(ContextMenuItem::new(
                21,
                "Disconnect",
                Some("bolt.slash".to_owned()),
                "disconnect",
                move |_, cx| socket.update(cx, |socket, cx| socket.disconnect(cx)),
            ));
        } else {
            items.push(ContextMenuItem::new(
                21,
                "Connect",
                Some("bolt.horizontal".to_owned()),
                "connect",
                move |_, cx| {
                    let token = AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone();
                    socket.update(cx, |socket, cx| socket.connect_if_needed(Some(&token), cx));
                },
            ));
        }
        items
    }

    fn sheet_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        vstack(0.)
            .size_full()
            .child(self.sheet_header(&theme, cx))
            .child(self.performance_launcher(&theme, cx))
            .child(div().w_full().h(px(1.)).flex_none().bg(theme.border._200))
            .child(self.debugger_tab_bar(&theme, cx))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.debugger_tab_body(&theme, cx)),
            )
            .bg(theme.bg._200)
            .into_any_element()
    }

    fn sheet_header(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        if let Some(header) = &self.sheet_header {
            return header.clone().into_any_element();
        }
        let theme = theme.clone();
        let revision = self.host_switch_revision;
        let select_this = cx.weak_entity();
        let close_this = cx.weak_entity();
        let header = cx.new(|cx| {
            SaiDebuggerSheetHeader::new(
                theme,
                revision,
                Rc::new(move |environment: SaiDevHostEnvironment, window: &mut Window, cx: &mut App| {
                    select_this
                        .update(cx, |host, cx| host.request_host_switch(environment.slot(), window, cx))
                        .ok();
                }),
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    close_this
                        .update(cx, |host, cx| {
                            host.sheet_open = false;
                            cx.notify();
                        })
                        .ok();
                }),
                cx,
            )
        });
        self.sheet_header = Some(header.clone());
        header.into_any_element()
    }

    fn debugger_tab_bar(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tabs = Self::debugger_tabs().into_iter().map(|(id, title)| {
            let active = self.tab_index == id;
            let this = cx.weak_entity();
            plain(("sai-debugger-tab", id as usize))
                .flex_none()
                .on_press(move |_, cx| {
                    this.update(cx, |host, cx| host.set_tab_index(id, cx)).ok();
                })
                .child(
                    div()
                        .child(title)
                        .swiftui_font(Font::caption().weight(if active {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        }))
                        .text_color(if active { theme.text.primary } else { theme.text.secondary })
                        .whitespace_nowrap()
                        .px(px(10.))
                        .py(px(6.))
                        .bg(if active { theme.bg._300 } else { theme.bg._200.opacity(0.6) })
                        .rounded_full(),
                )
        });
        let tabs: Vec<_> = tabs.collect();

        div()
            .id("sai-debugger-tab-bar")
            .w_full()
            .flex_none()
            .flex()
            .overflow_x_scroll()
            .bg(theme.bg._300.opacity(0.35))
            .child(hstack(6.).flex_none().px(px(8.)).py(px(6.)).children(tabs))
    }

    fn debugger_tab_body(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        match self.tab_index {
            0 => self.fetches_panel(theme, cx),
            1 => self.user_defaults_cards_panel("LocalStorage", SaiDebugUserDefaultsPanelKind::LocalStorage, theme, cx),
            2 => self.user_defaults_cards_panel(
                "DiskCache",
                SaiDebugUserDefaultsPanelKind::ApplicationSupportWs,
                theme,
                cx,
            ),
            3 => {
                let delete_this = cx.weak_entity();
                let clear_this = cx.weak_entity();
                SaiCleverImageCacheDebugPanel::new(
                    theme.clone(),
                    Binding::entity(
                        cx.entity(),
                        |host: &Self| host.clever_image_cache_snapshot_tick,
                        |host, tick, cx| {
                            host.clever_image_cache_snapshot_tick = tick;
                            cx.notify();
                        },
                    ),
                    move |digest, window, cx| {
                        delete_this
                            .update(cx, |host, cx| host.request_clever_image_cache_delete(digest, window, cx))
                            .ok();
                    },
                    move |window, cx| {
                        clear_this
                            .update(cx, |host, cx| host.request_clever_image_cache_clear_all(window, cx))
                            .ok();
                    },
                )
                .into_any_element()
            }
            4 => {
                let delete_this = cx.weak_entity();
                let clear_this = cx.weak_entity();
                SaiMusicPlaybackCacheDebugPanel::new(
                    "sai-debugger-music-cache",
                    theme.clone(),
                    Binding::entity(
                        cx.entity(),
                        |host: &Self| host.music_cache_snapshot_tick,
                        |host, tick, cx| {
                            host.music_cache_snapshot_tick = tick;
                            cx.notify();
                        },
                    ),
                    move |track_id, window, cx| {
                        delete_this
                            .update(cx, |host, cx| host.request_music_cache_track_delete(track_id, window, cx))
                            .ok();
                    },
                    move |window, cx| {
                        clear_this.update(cx, |host, cx| host.request_music_cache_clear_all(window, cx)).ok();
                    },
                )
                .into_any_element()
            }
            6 => self.ws_response_cache_panel(theme, cx),
            5 | 7 | 10 | 11 | 12 | 13 | 14 => self.mounted_tab_panel(theme, cx),
            8 => self.events_panel(theme, cx),
            9 => self.log_entries_panel(theme, "Empty", Some(SaiDebuggerLogClearMode::All), cx),
            15 => {
                let select_this = cx.weak_entity();
                SaiBottomSheetGalleryPanel::new(
                    theme.clone(),
                    Binding::entity(
                        cx.entity(),
                        |host: &Self| host.gallery_module,
                        |host, module, cx| {
                            host.gallery_module = module;
                            cx.notify();
                        },
                    ),
                    Binding::entity(
                        cx.entity(),
                        |host: &Self| host.gallery_query.clone(),
                        |host, query, cx| {
                            host.gallery_query = query;
                            cx.notify();
                        },
                    ),
                    Binding::entity(
                        cx.entity(),
                        |host: &Self| host.gallery_kind,
                        |host, kind, cx| {
                            host.gallery_kind = kind;
                            cx.notify();
                        },
                    ),
                    move |entry, _, cx| {
                        select_this
                            .update(cx, |host, cx| {
                                if host.pending_sheet_preview.is_some() {
                                    return;
                                }
                                KeyboardDismiss::perform(cx);
                                host.active_sheet_preview = None;
                                host.pending_sheet_preview = Some(entry.clone());
                                host.sheet_open = false;
                                cx.notify();
                            })
                            .ok();
                    },
                )
                .into_any_element()
            }
            _ => div().into_any_element(),
        }
    }

    fn mounted_tab_panel(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        if let Some(panel) = &self.active_panel {
            return panel.view.clone().into_any_element();
        }
        let theme = theme.clone();
        let panel = match self.tab_index {
            5 => {
                let host = cx.entity();
                let message_this = cx.weak_entity();
                let chat_this = cx.weak_entity();
                let clear_this = cx.weak_entity();
                SaiDebuggerTabPanel::mount(
                    cx.new(|cx| {
                        SaiVoiceCacheDebugPanel::new(
                            theme,
                            Binding::entity(
                                host,
                                |host: &Self| host.voice_cache_snapshot_tick,
                                |host, tick, cx| {
                                    host.voice_cache_snapshot_tick = tick;
                                    cx.notify();
                                },
                            ),
                            Rc::new(move |message_id: String, window: &mut Window, cx: &mut App| {
                                message_this
                                    .update(cx, |host, cx| host.request_voice_cache_message_delete(message_id, window, cx))
                                    .ok();
                            }),
                            Rc::new(move |chat_id: String, window: &mut Window, cx: &mut App| {
                                chat_this
                                    .update(cx, |host, cx| host.request_voice_cache_chat_delete(chat_id, window, cx))
                                    .ok();
                            }),
                            Rc::new(move |window: &mut Window, cx: &mut App| {
                                clear_this.update(cx, |host, cx| host.request_voice_cache_clear_all(window, cx)).ok();
                            }),
                            cx,
                        )
                    }),
                    |view, theme, _, cx| view.set_theme(theme, cx),
                )
            }
            7 => SaiDebuggerTabPanel::mount(cx.new(|cx| SaiWsDeferredDebugPanel::new(theme, cx)), |view, theme, _, cx| {
                view.set_theme(theme, cx)
            }),
            10 => SaiDebuggerTabPanel::mount(
                cx.new(|cx| SaiWsIgnoredRequestsPanel::new(theme, cx)),
                |view, theme, _, cx| view.set_theme(theme, cx),
            ),
            11 => SaiDebuggerTabPanel::mount(cx.new(|cx| SaiApiDocsPanel::new(theme, cx)), |view, theme, _, cx| {
                view.set_theme(theme, cx)
            }),
            12 => SaiDebuggerTabPanel::mount(cx.new(|cx| SaiRequesterPanel::new(theme, cx)), |view, theme, _, cx| {
                view.set_theme(theme, cx)
            }),
            13 => SaiDebuggerTabPanel::mount(
                cx.new(|cx| SaiSkeletonGalleryPanel::new(theme, cx)),
                |view, theme, _, cx| view.set_theme(theme, cx),
            ),
            _ => {
                let revision = self.host_switch_revision;
                let this = cx.weak_entity();
                SaiDebuggerTabPanel::mount(
                    cx.new(|cx| {
                        SaiDevHostPanel::new(
                            theme,
                            revision,
                            Rc::new(move |slot: SaiDevHostSlot, window: &mut Window, cx: &mut App| {
                                this.update(cx, |host, cx| host.request_host_switch(slot, window, cx)).ok();
                            }),
                            cx,
                        )
                    }),
                    |view, theme, revision, cx| view.set_props(theme, revision, cx),
                )
            }
        };
        let view = panel.view.clone();
        self.active_panel = Some(panel);
        view.into_any_element()
    }

    fn performance_launcher(&self, theme: &AppTheme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let this = cx.weak_entity();
        div()
            .id("sai-debugger-performance-label")
            .w_full()
            .flex_none()
            .bg(theme.bg._200)
            .tooltip_label("Open performance monitor")
            .child(
                plain("sai-debugger-performance")
                    .w_full()
                    .on_press(move |_, cx| {
                        this.update(cx, |host, cx| {
                            host.sheet_open = false;
                            host.overlay_state.update(cx, |state, cx| state.show_performance_hud(cx));
                            cx.notify();
                        })
                        .ok();
                        let _ = impact(ImpactStyle::Light);
                    })
                    .child(
                        hstack(12.)
                            .w_full()
                            .px(px(14.))
                            .py(px(9.))
                            .child(
                                div()
                                    .size(px(34.))
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .bg(hex(PERF_GREEN).opacity(0.13))
                                    .rounded_full()
                                    .children(IconUi::sized(
                                        "gauge.with.dots.needle.67percent",
                                        px(18.),
                                        hex(PERF_GREEN),
                                        false,
                                    )),
                            )
                            .child(
                                vstack(2.)
                                    .align(HorizontalAlignment::Leading)
                                    .min_w_0()
                                    .child(label(
                                        "Perf",
                                        Font::subheadline().weight(FontWeight::BOLD),
                                        theme.text.primary,
                                    ))
                                    .child(
                                        label(
                                            "Live FPS, hitches, memory and shareable trace",
                                            Font::caption2(),
                                            theme.text.secondary,
                                        )
                                        .line_clamp(1),
                                    ),
                            )
                            .child(spacer_min_w(0.))
                            .children(IconUi::sized(
                                "arrow.up.left.and.arrow.down.right",
                                px(12.),
                                theme.text.secondary,
                                false,
                            )),
                    ),
            )
    }

    fn panel_header(theme: &AppTheme, title: &str, count: usize, trailing: Vec<AnyElement>) -> Div {
        hstack(None::<f32>)
            .w_full()
            .flex_none()
            .child(label(title.to_owned(), caption_semibold(), theme.text.secondary))
            .child(label(
                format!("({count})"),
                Font::caption2().monospaced_digit(),
                theme.text.secondary.opacity(0.75),
            ))
            .child(spacer_min_w(0.))
            .children(trailing)
            .px(px(14.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.35))
    }

    fn text_action(
        id: &'static str,
        title: &'static str,
        color: u32,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> AnyElement {
        plain(id)
            .on_press(action)
            .child(label(title, caption_semibold(), hex(color)))
            .into_any_element()
    }

    fn panel_empty(text: impl Into<SharedString>, theme: &AppTheme, top: f32) -> AnyElement {
        div()
            .w_full()
            .flex()
            .justify_center()
            .p(px(LIST_PADDING))
            .pt(px(LIST_PADDING + top))
            .child(label(text, Font::caption(), theme.text.secondary))
            .into_any_element()
    }

    fn fetches_panel(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let mut trailing = Vec::new();
        if self.http_trace_total_count > 0 {
            let this = cx.weak_entity();
            trailing.push(Self::text_action("sai-fetches-clear", "Clear all", DESTRUCTIVE_RED, move |window, cx| {
                this.update(cx, |host, cx| host.request_http_trace_clear_all(window, cx)).ok();
            }));
        }

        let picker_this = cx.weak_entity();
        let picker = SegmentedPicker::new(
            "sai-fetches-activity",
            vec![
                "HTTP \u{b7} WS".into(),
                format!("{} ({})", tr("all_notify_title"), self.http_trace_total_count).into(),
            ],
            usize::from(self.shows_all_fetch_activity),
            move |index, _, cx| {
                picker_this
                    .update(cx, |host, cx| {
                        host.shows_all_fetch_activity = index == 1;
                        host.sync_fetches(cx);
                        cx.notify();
                    })
                    .ok();
            },
        );

        let legend = self.shows_all_fetch_activity.then(|| {
            hstack(12.)
                .w_full()
                .flex_none()
                .justify_center()
                .px(px(14.))
                .pb(px(6.))
                .child(Self::cache_legend_dot(hex(CACHE_AMBER), "memory cache", theme))
                .child(Self::cache_legend_dot(hex(CACHE_PURPLE), "disk / RiseStore", theme))
        });

        let body = if self.visible_http_entries.is_empty() {
            Self::panel_empty(tr("no_data_yet"), theme, 32.)
        } else {
            let entries = self.visible_http_entries.clone();
            let row_theme = theme.clone();
            let this = cx.weak_entity();
            list(self.fetches_list.clone(), move |index, _, _| {
                let Some(entry) = entries.get(index).cloned() else {
                    return div().into_any_element();
                };
                let id = entry.id;
                let this = this.clone();
                lazy_row(
                    index,
                    entries.len(),
                    14.,
                    SaiHttpTraceCardView::new(entry, row_theme.clone()).on_delete(move |window, cx| {
                        this.update(cx, |host, cx| host.request_http_trace_row_delete(id, window, cx)).ok();
                    }),
                )
            })
            .size_full()
            .into_any_element()
        };

        vstack(0.)
            .size_full()
            .child(Self::panel_header(theme, "HTTP \u{b7} WS", self.http_trace_network_count, trailing))
            .child(div().w_full().flex_none().px(px(14.)).py(px(6.)).child(picker))
            .children(legend)
            .child(div().w_full().flex_1().min_h_0().bg(theme.bg._100).child(body))
            .into_any_element()
    }

    fn cache_legend_dot(color: Hsla, text: &'static str, theme: &AppTheme) -> Div {
        hstack(5.)
            .child(
                div()
                    .relative()
                    .size(px(14.))
                    .child(centred_stroke(color, 2., Shape::continuous(2.))),
            )
            .child(label(text, Font::caption2(), theme.text.secondary.opacity(0.85)))
    }

    fn events_panel(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let mut trailing = Vec::new();
        if !self.ws_event_entries.is_empty() {
            let this = cx.weak_entity();
            trailing.push(Self::text_action("sai-events-clear", "Clear all", DESTRUCTIVE_RED, move |window, cx| {
                this.update(cx, |host, cx| host.request_ws_event_clear_all(window, cx)).ok();
            }));
        }

        let body = if self.ws_event_entries.is_empty() {
            Self::panel_empty("No WebSocket events received yet", theme, 32.)
        } else {
            let entries = self.ws_event_entries.clone();
            let row_theme = theme.clone();
            let this = cx.weak_entity();
            list(self.events_list.clone(), move |index, _, _| {
                let Some(entry) = entries.get(index).cloned() else {
                    return div().into_any_element();
                };
                let id = entry.id;
                let this = this.clone();
                lazy_row(
                    index,
                    entries.len(),
                    14.,
                    SaiWsEventCardView::new(entry, row_theme.clone()).on_delete(move |window, cx| {
                        this.update(cx, |host, cx| host.request_ws_event_row_delete(id, window, cx)).ok();
                    }),
                )
            })
            .size_full()
            .into_any_element()
        };

        vstack(0.)
            .size_full()
            .child(Self::panel_header(theme, "Events", self.ws_event_entries.len(), trailing))
            .child(div().w_full().flex_1().min_h_0().bg(theme.bg._100).child(body))
            .into_any_element()
    }

    fn ws_response_cache_panel(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let mut trailing = Vec::new();
        if !self.ws_cache_keys.is_empty() {
            let this = cx.weak_entity();
            trailing.push(Self::text_action("sai-ws-cache-clear", "Clear all", DESTRUCTIVE_RED, move |window, cx| {
                this.update(cx, |host, cx| host.request_ws_cache_clear_all(window, cx)).ok();
            }));
        }
        let refresh_this = cx.weak_entity();
        trailing.push(Self::text_action("sai-ws-cache-refresh", "Refresh", LINK_BLUE, move |_, cx| {
            refresh_this
                .update(cx, |host, cx| {
                    host.ws_cache_snapshot_tick += 1;
                    host.sync_ws_cache(cx);
                    cx.notify();
                })
                .ok();
        }));

        let body = if self.ws_cache_keys.is_empty() {
            Self::panel_empty("No WS response cache yet (memory + Application Support).", theme, 28.)
        } else {
            let keys = self.ws_cache_keys.clone();
            let row_theme = theme.clone();
            let this = cx.weak_entity();
            list(self.ws_cache_list.clone(), move |index, _, cx| {
                let Some(key) = keys.get(index).cloned() else {
                    return div().into_any_element();
                };
                let layers = SaiWsResponseCache::debug_layers_present(&key, cx);
                let delete_key = key.clone();
                let this = this.clone();
                lazy_row(
                    index,
                    keys.len(),
                    14.,
                    SaiWsCacheDebugCardView::new(key, layers, row_theme.clone()).on_delete(move |window, cx| {
                        let key = delete_key.clone();
                        this.update(cx, |host, cx| host.request_ws_cache_key_delete(key, window, cx)).ok();
                    }),
                )
            })
            .size_full()
            .into_any_element()
        };

        vstack(0.)
            .id(("sai-debugger-ws-cache-panel", self.ws_cache_snapshot_tick as u64))
            .size_full()
            .child(Self::panel_header(theme, "WS cache", self.ws_cache_keys.len(), trailing))
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .max_h(px(PANEL_MAX_HEIGHT))
                    .bg(theme.bg._100)
                    .child(body),
            )
            .into_any_element()
    }

    fn user_defaults_cards_panel(
        &mut self,
        title: &str,
        kind: SaiDebugUserDefaultsPanelKind,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut trailing = Vec::new();
        if !self.storage_entries.is_empty() {
            let this = cx.weak_entity();
            trailing.push(Self::text_action("sai-storage-clear", "Clear all", DESTRUCTIVE_RED, move |window, cx| {
                this.update(cx, |host, cx| host.request_storage_clear_all(kind, window, cx)).ok();
            }));
        }
        let refresh_this = cx.weak_entity();
        trailing.push(Self::text_action("sai-storage-refresh", "Refresh", LINK_BLUE, move |_, cx| {
            refresh_this
                .update(cx, |host, cx| {
                    host.storage_snapshot_tick += 1;
                    host.sync_storage(kind);
                    cx.notify();
                })
                .ok();
        }));

        let folder = (kind == SaiDebugUserDefaultsPanelKind::ApplicationSupportWs).then(|| {
            div()
                .w_full()
                .flex_none()
                .flex()
                .justify_center()
                .px(px(12.))
                .pb(px(6.))
                .child(
                    label(
                        SaiWsDiskCache::debug_folder_path_for_inspector(),
                        Font::caption2(),
                        theme.text.secondary.opacity(0.78),
                    )
                    .text_center()
                    .line_clamp(3),
                )
        });

        let body = if self.storage_entries.is_empty() {
            Self::panel_empty(Self::storage_empty_message(kind), theme, 28.)
        } else {
            let entries = self.storage_entries.clone();
            let row_theme = theme.clone();
            let this = cx.weak_entity();
            list(self.storage_list.clone(), move |index, _, _| {
                let Some(entry) = entries.get(index).cloned() else {
                    return div().into_any_element();
                };
                let identifier = entry.id.clone();
                let this = this.clone();
                lazy_row(
                    index,
                    entries.len(),
                    14.,
                    SaiDebugStorageCardView::new(entry, kind, row_theme.clone()).on_delete(move |window, cx| {
                        let identifier = identifier.clone();
                        this.update(cx, |host, cx| host.request_storage_key_delete(kind, identifier, window, cx))
                            .ok();
                    }),
                )
            })
            .size_full()
            .into_any_element()
        };

        vstack(0.)
            .id(("sai-debugger-storage-panel", self.storage_snapshot_tick as u64))
            .size_full()
            .child(Self::panel_header(theme, title, self.storage_entries.len(), trailing))
            .children(folder)
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .max_h(px(PANEL_MAX_HEIGHT))
                    .bg(theme.bg._100)
                    .child(body),
            )
            .into_any_element()
    }

    fn log_entries_panel(
        &mut self,
        theme: &AppTheme,
        empty_label: &'static str,
        clear_all_mode: Option<SaiDebuggerLogClearMode>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut trailing = Vec::new();
        if let Some(mode) = clear_all_mode
            && !self.log_items.is_empty()
        {
            let this = cx.weak_entity();
            trailing.push(Self::text_action("sai-logs-clear", "Clear all", DESTRUCTIVE_RED, move |window, cx| {
                let mode = mode.clone();
                this.update(cx, |host, cx| host.request_log_clear_all(mode, window, cx)).ok();
            }));
        }

        let body = if self.log_items.is_empty() {
            Self::panel_empty(empty_label, theme, 28.)
        } else {
            let items = self.log_items.clone();
            let row_theme = theme.clone();
            let this = cx.weak_entity();
            list(self.logs_list.clone(), move |index, _, _| {
                let Some(entry) = items.get(index).cloned() else {
                    return div().into_any_element();
                };
                let id = entry.id;
                let this = this.clone();
                lazy_row(
                    index,
                    items.len(),
                    12.,
                    Self::log_entry_row(entry, &row_theme, move |window, cx| {
                        this.update(cx, |host, cx| host.request_log_entry_delete(id, window, cx)).ok();
                    }),
                )
            })
            .size_full()
            .into_any_element()
        };

        vstack(0.)
            .size_full()
            .child(Self::panel_header(theme, "Rows", self.log_items.len(), trailing))
            .child(div().w_full().flex_1().min_h_0().bg(theme.bg._100).child(body))
            .into_any_element()
    }

    fn log_entry_row(
        entry: SaiLogEntry,
        theme: &AppTheme,
        on_delete: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Div {
        let stamp = LOG_ENTRY_TIME_FORMATTER.with(|formatter| formatter.string_from(&entry.timestamp));
        let title = if entry.name == "Unnamed" { "[Unnamed]".to_owned() } else { entry.name.clone() };
        let accent = Self::kind_accent(&entry.kind);
        let summary = SaiDebugBodyFormat::one_line_preview(
            &entry.message,
            SaiDebugInspectorMetrics::COLLAPSED_PREVIEW_MAX_CHARS,
        );
        let copy_line = entry.console_line();
        let id = entry.id;
        let message = entry.message.clone();

        hstack(10.)
            .align(VerticalAlignment::Top)
            .w_full()
            .child(div().w(px(4.)).flex_none().self_stretch().my(px(2.)).bg(accent).clip_shape(Shape::continuous(2.)))
            .child(
                vstack(4.)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(
                        hstack(8.)
                            .w_full()
                            .child(label(
                                stamp,
                                Font::caption2().weight(FontWeight::MEDIUM).monospaced_digit(),
                                theme.text.secondary.opacity(0.85),
                            ))
                            .child(label(title, caption_semibold(), theme.text.primary))
                            .child(
                                label(
                                    entry.kind.short_label().to_owned(),
                                    Font::caption2().weight(FontWeight::SEMIBOLD),
                                    white().opacity(0.95),
                                )
                                .px(px(6.))
                                .py(px(2.))
                                .bg(accent.opacity(0.55))
                                .rounded_full(),
                            )
                            .child(spacer_min_w(0.)),
                    )
                    .child(
                        SaiDebugExpandableMonospaceBlock::new(
                            SharedString::from(format!("sai-log-message-{id}")),
                            "MESSAGE",
                            summary,
                            theme.clone(),
                            Rc::new(move |_: &mut App| message.clone()),
                        )
                        .background(theme.bg._100.opacity(0.5)),
                    )
                    .child(label(
                        format!("{}:{}", entry.file, entry.line),
                        Font::caption2(),
                        theme.text.secondary.opacity(0.55),
                    )),
            )
            .child(
                vstack(6.)
                    .flex_none()
                    .child(
                        plain(SharedString::from(format!("sai-log-copy-{id}")))
                            .on_press(move |_, cx| UIPasteboard::general().set_string(&copy_line, cx))
                            .child(Self::log_row_action("doc.on.doc", theme.text.primary, theme)),
                    )
                    .child(
                        plain(SharedString::from(format!("sai-log-delete-{id}")))
                            .on_press(on_delete)
                            .child(Self::log_row_action("trash", hex(DESTRUCTIVE_RED), theme)),
                    ),
            )
            .p(px(10.))
            .bg(theme.bg._200.opacity(0.55))
            .clip_shape(Shape::continuous(10.))
    }

    fn log_row_action(symbol: &str, color: Hsla, theme: &AppTheme) -> Div {
        div()
            .w(px(34.))
            .h(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.bg._300.opacity(0.9))
            .clip_shape(Shape::continuous(8.))
            .children(IconUi::sized(symbol, px(ROW_ICON_SIDE), color, false))
    }

    fn kind_accent(kind: &SaiLogKind) -> Hsla {
        hex(match kind {
            SaiLogKind::Default => 0x9e9e9e,
            SaiLogKind::Ui => 0x7c4dff,
            SaiLogKind::Network => LINK_BLUE,
            SaiLogKind::Websocket => LED_GREEN,
            SaiLogKind::Cache => CACHE_AMBER,
            SaiLogKind::Util => 0x78909c,
            SaiLogKind::Event => 0xec407a,
            SaiLogKind::Debug => 0xb0bec5,
            SaiLogKind::Custom(_) => 0x90a4ae,
        })
    }
}

impl Render for SaiDebuggerHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        SaiLogger::set_has_reader(self.sheet_open && self.tab_index == 9);
        SaiHttpTraceStore::set_has_reader(self.sheet_open && matches!(self.tab_index, 0 | 11));
        let this = cx.entity();
        let weak_this = cx.weak_entity();
        let (is_orb_visible, is_performance_hud_visible) = {
            let overlay_state = self.overlay_state.read(cx);
            (overlay_state.is_orb_visible(), overlay_state.is_performance_hud_visible())
        };

        let orb = is_orb_visible.then(|| self.orb(cx));
        let hud = is_performance_hud_visible.then(|| {
            let dismiss_this = weak_this.clone();
            let share_this = weak_this.clone();
            SaiPerformanceHUD::new(
                "sai-debugger-performance-hud",
                move |_, cx| {
                    dismiss_this
                        .update(cx, |host, cx| {
                            host.overlay_state.update(cx, |state, cx| state.dismiss_performance_hud(cx));
                        })
                        .ok();
                },
                move |url, cx| {
                    share_this
                        .update(cx, |host, cx| {
                            host.performance_share_item = Some(SaiPerformanceShareItem::new(url));
                            if let Some(item) = host.performance_share_item.take() {
                                SaiPerformanceShareSheet::new(item.url).present(cx);
                            }
                        })
                        .ok();
                },
            )
        });

        let content_this = this.clone();
        let dismiss_this = weak_this;
        let sheet = BottomSheetUi::new(
            "sai-debugger-sheet",
            Binding::entity(
                this,
                |host: &Self| host.sheet_open,
                |host, is_open, cx| {
                    host.sheet_open = is_open;
                    cx.notify();
                },
            ),
            move |_, cx| content_this.update(cx, |host, cx| host.sheet_content(cx)),
            |_, _| div(),
        )
        .snap_fractions(vec![0.58, 0.93])
        .footer_shows_top_border(false)
        .content_scrollable(false)
        .on_did_dismiss(move |_, cx| {
            dismiss_this
                .update(cx, |host, cx| {
                    host.sheet_header = None;
                    host.active_panel = None;
                    let Some(entry) = host.pending_sheet_preview.take() else {
                        return;
                    };
                    host.active_sheet_preview = Some(entry);
                    cx.notify();
                })
                .ok();
        });

        let preview = self.active_sheet_preview.clone().map(SaiBottomSheetPreview::new);

        div()
            .relative()
            .size_full()
            .children(orb)
            .children(hud)
            .child(sheet)
            .children(preview)
    }
}

fn sai_debugger_orb_colors(slot: SaiDevHostSlot) -> [Hsla; 2] {
    match slot {
        SaiDevHostSlot::Config | SaiDevHostSlot::Fallback => [hex(DESTRUCTIVE_RED), hex(0xc2410c)],
        SaiDevHostSlot::Staging => [hex(0x7c3aed), hex(0x2563eb)],
        SaiDevHostSlot::Production => [hex(0xf59e0b), hex(0xb45309)],
        SaiDevHostSlot::Custom => [hex(0x0891b2), hex(0x0d9488)],
    }
}

fn sai_debugger_led_color(socket: &SaiWebSocketManager) -> Hsla {
    if matches!(socket.auto_reconnect_status(), SaiWebSocketAutoReconnectStatus::Scheduled { .. }) {
        return hex(CACHE_AMBER);
    }
    match socket.phase() {
        SaiWebSocketManagerPhase::Connected(..) => hex(LED_GREEN),
        SaiWebSocketManagerPhase::Connecting(_) => hex(CACHE_AMBER),
        SaiWebSocketManagerPhase::Failed(_) => hex(LED_RED),
        _ => hex(LED_GRAY),
    }
}

/// a row of a lazy list with 14 padding around the stack and spacing between rows
fn lazy_row(index: usize, count: usize, spacing: f32, child: impl IntoElement) -> AnyElement {
    div()
        .w_full()
        .px(px(LIST_PADDING))
        .pt(px(if index == 0 { LIST_PADDING } else { spacing }))
        .when(index + 1 == count, |row| row.pb(px(LIST_PADDING)))
        .child(child)
        .into_any_element()
}

/// list state reset scrolls back to the start but the reader should stay on their rows
fn reset_list_keeping_anchor<T, K: PartialEq>(
    list_state: &ListState,
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> K,
) {
    let top = list_state.logical_scroll_top();
    let anchor = (top.item_ix != 0 || top.offset_in_item != px(0.))
        .then(|| old.get(top.item_ix).map(&key))
        .flatten();
    list_state.reset(new.len());
    if let Some(anchor) = anchor
        && let Some(item_ix) = new.iter().position(|item| key(item) == anchor)
    {
        list_state.scroll_to(ListOffset { item_ix, offset_in_item: top.offset_in_item });
    }
}

type SaiDebuggerSelectEnvironment = Rc<dyn Fn(SaiDevHostEnvironment, &mut Window, &mut App)>;
type SaiDebuggerSheetClose = Rc<dyn Fn(&mut Window, &mut App)>;

struct SaiDebuggerSheetHeader {
    theme: AppTheme,
    revision: i64,
    on_select_environment: SaiDebuggerSelectEnvironment,
    on_close: SaiDebuggerSheetClose,
    socket: Entity<SaiWebSocketManager>,
    _timeline: RepeatingTimer,
}

impl SaiDebuggerSheetHeader {
    fn new(
        theme: AppTheme,
        revision: i64,
        on_select_environment: SaiDebuggerSelectEnvironment,
        on_close: SaiDebuggerSheetClose,
        cx: &mut Context<Self>,
    ) -> Self {
        let socket = SaiWebSocketManager::shared(cx);
        cx.observe(&socket, |_, _, cx| cx.notify()).detach();
        let timeline = RepeatingTimer::scheduled_timer(Duration::from_secs(1), true, cx, |_, cx| cx.notify());
        Self { theme, revision, on_select_environment, on_close, socket, _timeline: timeline }
    }

    fn set_props(&mut self, theme: AppTheme, revision: i64, cx: &mut Context<Self>) {
        self.theme = theme;
        self.revision = revision;
        cx.notify();
    }

    fn environment_picker(&self) -> SegmentedPicker {
        let environments = SaiDevHostEnvironment::ALL_CASES;
        let current = SaiDevHostOverride::slot().environment();
        let selected_index = environments
            .iter()
            .position(|environment| Some(*environment) == current)
            .unwrap_or(environments.len());
        let on_select_environment = self.on_select_environment.clone();
        SegmentedPicker::new(
            ElementId::NamedInteger("sai-debugger-environment".into(), self.revision as u64),
            environments.iter().map(|environment| SharedString::from(environment.title())).collect(),
            selected_index,
            move |index, window, cx| {
                let Some(environment) = SaiDevHostEnvironment::ALL_CASES.get(index).copied() else {
                    return;
                };
                if Some(environment) == SaiDevHostOverride::slot().environment() {
                    return;
                }
                on_select_environment(environment, window, cx);
            },
        )
    }

    fn connection_subtitle(socket: &SaiWebSocketManager) -> String {
        let display_url = socket.active_url_display_string();
        match socket.phase() {
            SaiWebSocketManagerPhase::Connected(_, since) => {
                let elapsed = Date::now().time_interval_since(since);
                format!("WebSocket \u{b7} {} \u{b7} {display_url}", Self::format_duration(elapsed))
            }
            SaiWebSocketManagerPhase::Connecting(url) => format!("Connecting\u{2026} {}", url.absolute_string()),
            SaiWebSocketManagerPhase::Failed(message) => format!("Error: {message}"),
            SaiWebSocketManagerPhase::Disconnected => format!("Disconnected \u{b7} {display_url}"),
            SaiWebSocketManagerPhase::Idle => format!("Idle \u{b7} {display_url}"),
        }
    }

    fn format_duration(t: f64) -> String {
        let ti = t.max(0.) as i64;
        let s = ti % 60;
        let m = (ti / 60) % 60;
        let h = ti / 3600;
        if h > 0 {
            return format!("{h}:{m:02}:{s:02}");
        }
        if m > 0 {
            return format!("{m}:{s:02}");
        }
        format!("0:{s:02}")
    }
}

impl Render for SaiDebuggerSheetHeader {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let socket = self.socket.read(cx);
        let led_color = sai_debugger_led_color(socket);
        let subtitle = Self::connection_subtitle(socket);
        let status = socket.auto_reconnect_status().clone();
        let on_close = self.on_close.clone();

        vstack(10.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .flex_none()
            .child(
                hstack(12.)
                    .w_full()
                    .child(
                        div().size(px(12.)).flex_none().rounded_full().bg(led_color).shadow(vec![BoxShadow {
                            color: led_color.opacity(0.55),
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(4.),
                            spread_radius: px(0.),
                            inset: false,
                        }]),
                    )
                    .child(
                        vstack(4.)
                            .align(HorizontalAlignment::Leading)
                            .flex_1()
                            .min_w_0()
                            .child(label(
                                "Sai debugger",
                                Font::headline().weight(FontWeight::SEMIBOLD),
                                theme.text.primary,
                            ))
                            .child(label(subtitle, Font::caption(), theme.text.secondary).line_clamp(2))
                            .child(SaiDebuggerReconnectBanner {
                                theme: theme.clone(),
                                status,
                                now: Date::now(),
                            }),
                    )
                    .child(
                        plain("sai-debugger-close")
                            .flex_none()
                            .on_press(move |window, cx| on_close(window, cx))
                            .children(IconUi::sized("xmark.circle.fill", px(22.), theme.text.secondary, true)),
                    ),
            )
            .when(SaiDevHostOverride::is_effective(), |header| header.child(self.environment_picker()))
            .px(px(16.))
            .py(px(12.))
            .bg(theme.bg._300)
    }
}

#[derive(IntoElement)]
struct SaiDebuggerReconnectBanner {
    theme: AppTheme,
    status: SaiWebSocketAutoReconnectStatus,
    now: Date,
}

impl SaiDebuggerReconnectBanner {
    fn reconnect_countdown(seconds: f64) -> String {
        if seconds < 60. {
            return format!("{seconds:.0} s");
        }
        let total = seconds as i64;
        format!("{}m {}s", total / 60, total % 60)
    }
}

impl RenderOnce for SaiDebuggerReconnectBanner {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let SaiWebSocketAutoReconnectStatus::Scheduled { attempt_index, fire_at, last_reason } = self.status else {
            return div().into_any_element();
        };
        let remaining = fire_at.time_interval_since(&self.now).max(0.);
        let amber = hex(CACHE_AMBER);
        let title = format!(
            "\u{221e} reconnect \u{b7} #{attempt_index} \u{0447}\u{0435}\u{0440}\u{0435}\u{0437} {}",
            Self::reconnect_countdown(remaining)
        );

        hstack(8.)
            .align(VerticalAlignment::Top)
            .relative()
            .w_full()
            .child(
                div()
                    .pt(px(1.))
                    .flex_none()
                    .children(IconUi::sized("arrow.triangle.2.circlepath", px(ROW_ICON_SIDE), amber, false)),
            )
            .child(
                vstack(2.)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(label(
                        title,
                        Font::caption2().weight(FontWeight::SEMIBOLD).monospaced_digit(),
                        self.theme.text.primary,
                    ))
                    .child(
                        label(last_reason, Font::caption2(), self.theme.text.secondary.opacity(0.88)).line_clamp(2),
                    ),
            )
            .px(px(10.))
            .py(px(7.))
            .bg(amber.opacity(0.12))
            .continuous_corners(9.)
            .child(centred_stroke(amber.opacity(0.35), 1., Shape::continuous(9.)))
            .into_any_element()
    }
}
