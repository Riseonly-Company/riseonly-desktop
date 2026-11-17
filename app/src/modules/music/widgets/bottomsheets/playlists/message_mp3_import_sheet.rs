use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Axis, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, ListAlignment, ParentElement, Render,
    RenderOnce, SharedString, Styled, Subscription, Task, Window,
};
use gpui::relative;
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::paths::temporary_directory;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::keyboard_dismiss::KeyboardDismiss;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::chat::components::chat::bubble::chat_audio_track_adapter::ChatAudioTrackAdapter;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageMediaItem};
use crate::modules::music::stores::music_actions::music_message_import_types::{
    MusicMessageMP3ImportLimits, MusicMessageMP3ImportRequest, MusicMessageMP3ImportSource,
};
use crate::modules::music::stores::music_actions::music_types::MusicLocalPlaylistDestination;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_cover_picker_ui::MusicCoverPickerUi;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const KEY_ESCAPE: &str = "escape";
const VIEWER_ROLE_OWNER: &str = "owner";
const DESCRIPTION_MAX_VISIBLE_LINES: usize = 5;

pub struct MessageMusicImportPresenter {
    is_presented: bool,
    source: Option<MusicMessageMP3ImportSource>,
    source_cleanup_task: Option<Task<()>>,
}

impl MessageMusicImportPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_presented: false, source: None, source_cleanup_task: None }
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn source(&self) -> Option<&MusicMessageMP3ImportSource> {
        self.source.as_ref()
    }

    /// reported as a guide fact by level not edge
    /// a step starting with the sheet already open counts as done
    fn set_is_presented(&mut self, is_presented: bool, cx: &mut Context<Self>) {
        self.is_presented = is_presented;
        GuideFacts::set(RiseGuideSignal::ImportSheetOpened, is_presented, cx);
        cx.notify();
    }

    pub fn present(&mut self, message: &GetMessageMessage, cx: &mut Context<Self>) {
        if message.protects_content_locally() {
            return;
        }
        let Some(item) = message.first_importable_mp3_item() else {
            return;
        };
        self.present_validated(item, &message.id, &message.chat_id, &message.text, cx);
    }

    /// presents the sheet for a media item
    /// takes message id and chat id
    /// plus caption and import flag
    pub fn present_item(
        &mut self,
        item: &MessageMediaItem,
        message_id: &str,
        chat_id: &str,
        caption: &str,
        allows_import: bool,
        cx: &mut Context<Self>,
    ) {
        if !allows_import || !item.is_importable_mp3() {
            return;
        }
        self.present_validated(item, message_id, chat_id, caption, cx);
    }

    fn present_validated(
        &mut self,
        item: &MessageMediaItem,
        message_id: &str,
        chat_id: &str,
        caption: &str,
        cx: &mut Context<Self>,
    ) {
        self.source_cleanup_task = None;

        HoldContextMenuPresenter::dismiss(cx);
        let (suggested_title, suggested_artist) = ChatAudioTrackAdapter::title_and_artist(item, caption);
        self.source = Some(MusicMessageMP3ImportSource {
            message_id: message_id.to_owned(),
            chat_id: chat_id.to_owned(),
            media_id: item.id.clone(),
            remote_url: item.url.clone(),
            local_preview_url: item.local_preview_url.clone(),
            thumbnail_url: item.thumbnail_url.clone(),
            file_name: item.file_name.clone(),
            file_size: item.file_size,
            mime_type: item.mime_type.clone(),
            duration_seconds: item.duration,
            caption: caption.to_owned(),
            suggested_title,
            suggested_artist,
        });
        self.set_is_presented(true, cx);
    }

    pub fn set_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if presented {
            let has_source = self.source.is_some();
            self.set_is_presented(has_source, cx);
        } else {
            self.dismiss(cx);
        }
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.set_is_presented(false, cx);
        self.schedule_source_cleanup(cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.source_cleanup_task = None;
        self.set_is_presented(false, cx);
        self.source = None;
        cx.notify();
    }

    fn schedule_source_cleanup(&mut self, cx: &mut Context<Self>) {
        self.source_cleanup_task = None;
        let Some(closing_source_id) = self.source.as_ref().map(MusicMessageMP3ImportSource::id) else {
            return;
        };

        let delay = BottomSheetMetrics::DISMISS_DURATION + 0.12;
        self.source_cleanup_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            this.update(cx, |this, cx| {
                if this.is_presented
                    || this.source.as_ref().map(MusicMessageMP3ImportSource::id).as_ref() != Some(&closing_source_id)
                {
                    return;
                }
                this.source = None;
                this.source_cleanup_task = None;
                cx.notify();
            })
            .ok();
        }));
    }
}

enum MessageMusicImportSheetLayout {}

impl MessageMusicImportSheetLayout {
    const DESTINATIONS_FRACTION: f32 = 0.67;
    const METADATA_FRACTION: f32 = 0.92;
    const HEADER_CONTROL_SIZE: f32 = 42.0;
}

pub struct MessageMusicImportSheetHost {
    presenter: Entity<MessageMusicImportPresenter>,
    navigation_path: Vec<MusicLocalPlaylistDestination>,
    navigation_strip_index: Animated<f32>,
    departing_destination: Option<MusicLocalPlaylistDestination>,
    sheet_fraction: f32,
    title: String,
    description: String,
    artist_name: String,
    local_cover_file: Option<SaiWsOptimisticLocalFile>,
    cover_preview: Option<CleverImage>,
    transferred_cover_urls: HashSet<Url>,
    focus_handle: FocusHandle,
    seen_is_presented: bool,
    seen_source_id: Option<String>,
    destination_list: Entity<MessageMP3DestinationListModel>,
    _subscriptions: Vec<Subscription>,
}

impl MessageMusicImportSheetHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let presenter = MessageMusicImportPresenter::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&presenter, |this, _, cx| this.presenter_did_change(cx)),
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.on_release(|this, cx| {
                this.clear_cover_selection();
                GuideFacts::set(RiseGuideSignal::ImportTrackFormOpened, false, cx);
            }),
        ];
        let seen_is_presented = presenter.read(cx).is_presented();
        let source = presenter.read(cx).source().cloned();
        let destination_list = cx.new(|cx| MessageMP3DestinationListModel::new(cx));
        let mut this = Self {
            presenter,
            navigation_path: Vec::new(),
            navigation_strip_index: Animated::new(0.0),
            departing_destination: None,
            sheet_fraction: MessageMusicImportSheetLayout::DESTINATIONS_FRACTION,
            title: String::new(),
            description: String::new(),
            artist_name: String::new(),
            local_cover_file: None,
            cover_preview: None,
            transferred_cover_urls: HashSet::new(),
            focus_handle: cx.focus_handle(),
            seen_is_presented,
            seen_source_id: source.as_ref().map(MusicMessageMP3ImportSource::id),
            destination_list,
            _subscriptions: subscriptions,
        };
        if seen_is_presented && let Some(source) = source {
            this.prepare(&source, cx);
        }
        this.navigation_path_did_change(cx);
        this
    }

    fn presenter_did_change(&mut self, cx: &mut Context<Self>) {
        let is_presented = self.presenter.read(cx).is_presented();
        let source = self.presenter.read(cx).source().cloned();
        let source_id = source.as_ref().map(MusicMessageMP3ImportSource::id);
        let presented_changed = is_presented != self.seen_is_presented;
        let source_changed = source_id != self.seen_source_id;
        self.seen_is_presented = is_presented;
        self.seen_source_id = source_id;
        if !is_presented {
            if presented_changed {
                self.clear_cover_selection();
                GuideFacts::set(RiseGuideSignal::ImportTrackFormOpened, false, cx);
            }
        } else if (presented_changed || source_changed)
            && let Some(source) = source
        {
            self.prepare(&source, cx);
        }
        cx.notify();
    }

    /// reacts to the navigation path changing, also runs once on start
    fn navigation_path_did_change(&mut self, cx: &mut Context<Self>) {
        let is_presented = self.presenter.read(cx).is_presented();
        GuideFacts::set(RiseGuideSignal::ImportTrackFormOpened, is_presented && !self.navigation_path.is_empty(), cx);
        if !is_presented {
            return;
        }
        self.sheet_fraction = if self.navigation_path.is_empty() {
            MessageMusicImportSheetLayout::DESTINATIONS_FRACTION
        } else {
            MessageMusicImportSheetLayout::METADATA_FRACTION
        };
    }

    /// local cover file, reacts when its url changes
    fn set_local_cover_file(&mut self, local_cover_file: Option<SaiWsOptimisticLocalFile>) {
        let previous_url = self.local_cover_file.as_ref().map(|file| file.url.clone());
        self.local_cover_file = local_cover_file;
        let Some(previous_url) = previous_url else {
            return;
        };
        if self.local_cover_file.as_ref().map(|file| &file.url) == Some(&previous_url) {
            return;
        }
        if self.transferred_cover_urls.remove(&previous_url) {
            return;
        }
        Self::remove_managed_temporary_cover(&previous_url);
    }

    fn select_destination(
        &mut self,
        destination: MusicLocalPlaylistDestination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.navigation_path.is_empty() {
            return;
        }
        self.navigation_path.push(destination);
        self.navigation_path_did_change(cx);
        self.animate_navigation(cx);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        KeyboardDismiss::perform(cx);
        self.presenter.update(cx, |presenter, cx| presenter.dismiss(cx));
    }

    fn navigate_back(&mut self, cx: &mut Context<Self>) {
        KeyboardDismiss::perform(cx);
        let Some(departing) = self.navigation_path.pop() else {
            return;
        };
        self.departing_destination = Some(departing);
        self.navigation_path_did_change(cx);
        self.animate_navigation(cx);
        cx.notify();
    }

    /// animates the path change with the sheet snap duration
    fn animate_navigation(&mut self, cx: &mut Context<Self>) {
        let target = self.navigation_path.len() as f32;
        let spec = ease_out(BottomSheetMetrics::SNAP_DURATION);
        self.navigation_strip_index.set_in(target, spec, RiseClock::monotonic(), cx);
    }

    fn prepare(&mut self, source: &MusicMessageMP3ImportSource, cx: &mut Context<Self>) {
        self.clear_cover_selection();
        let had_path = !self.navigation_path.is_empty();
        self.navigation_path.clear();
        self.navigation_strip_index.jump(0.0);
        self.departing_destination = None;
        self.sheet_fraction = MessageMusicImportSheetLayout::DESTINATIONS_FRACTION;
        self.title = MusicMessageMP3ImportLimits::title(&source.suggested_title);
        self.artist_name = MusicMessageMP3ImportLimits::artist(&source.suggested_artist);
        self.description = String::new();
        if had_path {
            self.navigation_path_did_change(cx);
        }
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_local_playlist_destinations_if_needed(cx));
    }

    fn enqueue_and_dismiss(
        &mut self,
        source: MusicMessageMP3ImportSource,
        destination: MusicLocalPlaylistDestination,
        cx: &mut Context<Self>,
    ) {
        if let Some(file) = &self.local_cover_file {
            self.transferred_cover_urls.insert(file.url.clone());
        }
        let request = MusicMessageMP3ImportRequest::new(
            source,
            destination,
            &self.title,
            Some(self.description.as_str()),
            &self.artist_name,
            self.local_cover_file.clone(),
        );

        self.clear_cover_selection();
        self.presenter.update(cx, |presenter, cx| presenter.dismiss(cx));
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.enqueue_message_mp3_import(request, cx));
        cx.notify();
    }

    fn clear_cover_selection(&mut self) {
        self.set_local_cover_file(None);
        self.cover_preview = None;
    }

    fn remove_managed_temporary_cover(url: &Url) {
        let Some(file) = url.to_file_path() else {
            return;
        };
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let (Ok(file_path), Ok(temp_path)) = (file.canonicalize(), temporary_directory().canonicalize()) else {
                return;
            };
            if file_path.starts_with(temp_path) {
                std::fs::remove_file(file_path).ok();
            }
        }));
    }

    fn navigation_content(this: &Entity<Self>, window: &Window, cx: &mut App) -> AnyElement {
        let Some(source) = this.read(cx).presenter.read(cx).source().cloned() else {
            return div().size_full().into_any_element();
        };
        let theme = rise_ui::theme(cx).clone();
        let focus_handle = this.read(cx).focus_handle.clone();
        let now = RiseClock::monotonic();
        let host = this.read(cx);
        let strip_index = host.navigation_strip_index.frame_value(now, window);
        let is_moving = host.navigation_strip_index.is_running(now);
        let is_root_on_screen = host.navigation_path.is_empty() || is_moving;
        let metadata_destination = host
            .navigation_path
            .last()
            .cloned()
            .or_else(|| if is_moving { host.departing_destination.clone() } else { None });
        let mut strip = div().id("message-music-import-strip").relative().size_full().overflow_hidden();
        if is_root_on_screen {
            let root = Self::destination_page(this, &theme, cx);
            strip = strip.child(div().absolute().top_0().left(relative(-strip_index)).size_full().child(root));
        }
        if let Some(destination) = metadata_destination {
            let form = Self::metadata_page(this, source, destination, &theme);
            strip = strip.child(div().absolute().top_0().left(relative(1.0 - strip_index)).size_full().child(form));
        }
        let key_host = this.clone();
        div()
            .id("message-music-import-navigation")
            .size_full()
            .bg(theme.bg._100)
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != KEY_ESCAPE || key_host.read(cx).navigation_path.is_empty() {
                    return;
                }
                key_host.update(cx, |host, cx| host.navigate_back(cx));
                cx.stop_propagation();
            })
            .child(strip)
            .into_any_element()
    }

    fn destination_page(this: &Entity<Self>, theme: &AppTheme, cx: &App) -> AnyElement {
        let dismiss_host = this.clone();
        let select_host = this.clone();
        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(Self::sheet_header(
                "message-music-import-cancel",
                tr("music_message_import_choose_playlist"),
                "xmark",
                tr("cancel"),
                move |_, cx| dismiss_host.update(cx, |host, cx| host.dismiss(cx)),
                theme,
            ))
            .child(div().flex_1().min_h_0().w_full().child(MessageMP3DestinationScreen {
                list_model: this.read(cx).destination_list.clone(),
                on_select: Rc::new(move |destination, window, cx| {
                    select_host.update(cx, |host, cx| host.select_destination(destination, window, cx));
                }),
            }))
            .into_any_element()
    }

    fn metadata_page(
        this: &Entity<Self>,
        source: MusicMessageMP3ImportSource,
        destination: MusicLocalPlaylistDestination,
        theme: &AppTheme,
    ) -> AnyElement {
        let back_host = this.clone();
        let add_host = this.clone();
        let add_source = source.clone();
        let add_destination = destination.clone();
        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(Self::sheet_header(
                "message-music-import-back",
                tr("music_message_import_metadata"),
                "chevron.backward",
                tr("back"),
                move |_, cx| back_host.update(cx, |host, cx| host.navigate_back(cx)),
                theme,
            ))
            .child(div().flex_1().min_h_0().w_full().child(MessageMP3MetadataScreen {
                source,
                destination,
                title: Binding::entity(this.clone(), |host: &Self| host.title.clone(), |host, value, _| {
                    host.title = value;
                }),
                description: Binding::entity(this.clone(), |host: &Self| host.description.clone(), |host, value, _| {
                    host.description = value;
                }),
                artist_name: Binding::entity(this.clone(), |host: &Self| host.artist_name.clone(), |host, value, _| {
                    host.artist_name = value;
                }),
                local_cover_file: Binding::entity(
                    this.clone(),
                    |host: &Self| host.local_cover_file.clone(),
                    |host, value, _| host.set_local_cover_file(value),
                ),
                cover_preview: Binding::entity(
                    this.clone(),
                    |host: &Self| host.cover_preview.clone(),
                    |host, value, _| host.cover_preview = value,
                ),
                on_add: Rc::new(move |_, cx| {
                    let source = add_source.clone();
                    let destination = add_destination.clone();
                    add_host.update(cx, |host, cx| host.enqueue_and_dismiss(source, destination, cx));
                }),
            }))
            .into_any_element()
    }

    fn sheet_header(
        id: &'static str,
        title: String,
        system_image: &'static str,
        accessibility_label: String,
        action: impl Fn(&mut Window, &mut App) + 'static,
        theme: &AppTheme,
    ) -> impl IntoElement {
        let control_size = MessageMusicImportSheetLayout::HEADER_CONTROL_SIZE;
        zstack(Alignment::Center)
            .w_full()
            .px(px(14.0))
            .pt(px(5.0))
            .pb(px(12.0))
            .child(
                div().w_full().px(px(control_size + 8.0)).child(
                    MainText::new(title).px(18.0).tac(TextAlignT::Center).font_weight(FontWeight::SEMIBOLD),
                ),
            )
            .child(
                hstack(0.0)
                    .w_full()
                    .child(
                        LiquidGlassButton::new(id, LiquidGlassButtonShape::Circle)
                            .size(control_size)
                            .accessibility_label(Some(accessibility_label))
                            .on_press(action)
                            .children(IconUi::sized(system_image, px(16.0), theme.text.primary, false)),
                    )
                    .child(spacer_min_w(0.0))
                    .child(div().size(px(control_size))),
            )
    }
}

impl Render for MessageMusicImportSheetHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let this = cx.entity();
        let visible = Binding::entity(
            self.presenter.clone(),
            |presenter| presenter.is_presented(),
            |presenter, value, cx| presenter.set_presented(value, cx),
        );
        let sheet_fraction = Binding::entity(this.clone(), |host: &Self| host.sheet_fraction, |host, value, _| {
            host.sheet_fraction = value;
        });
        BottomSheetUi::new(
            "message-music-import-sheet",
            visible,
            move |window: &mut Window, cx: &mut App| Self::navigation_content(&this, window, cx),
            |_: &mut Window, _: &mut App| div(),
        )
        .snap_fractions(vec![
            MessageMusicImportSheetLayout::DESTINATIONS_FRACTION,
            MessageMusicImportSheetLayout::METADATA_FRACTION,
        ])
        .selected_snap_fraction(Some(sheet_fraction))
        .experimental_bottom_sheet(true)
        .experimental_uses_liquid_glass_surface(false)
        .full_page_allows_swipe_dismiss(true)
        .full_page_shows_drag_indicator(true)
        .full_page_hides_footer(true)
        .full_page_avoids_keyboard_when_footer_hidden(true)
        .footer_shows_top_border(false)
        .footer_horizontal_padding(Some(0.0))
        .footer_top_padding(Some(0.0))
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
    }
}

const MESSAGE_MP3_DESTINATION_FOOTER_SLOT: f32 = 52.0;

const MESSAGE_MP3_RETRY_PRESS_GUARD: Duration = Duration::from_millis(600);

thread_local! {
    static MESSAGE_MP3_ROW_PRESS_GUARD: std::cell::Cell<Option<(Duration, usize)>> =
        const { std::cell::Cell::new(None) };
}

fn message_mp3_arm_row_guard(from_index: usize) {
    let now = RiseClock::monotonic();
    MESSAGE_MP3_ROW_PRESS_GUARD.with(|guard| {
        let from = match guard.get() {
            Some((at, from)) if now.saturating_sub(at) < MESSAGE_MP3_RETRY_PRESS_GUARD => from.min(from_index),
            _ => from_index,
        };
        guard.set(Some((now, from)));
    });
}

fn message_mp3_row_guard_holds(index: usize) -> bool {
    MESSAGE_MP3_ROW_PRESS_GUARD.with(|guard| guard.get()).is_some_and(|(at, from)| {
        index >= from && RiseClock::monotonic().saturating_sub(at) < MESSAGE_MP3_RETRY_PRESS_GUARD
    })
}

/// row bookkeeping of the destinations list, list keeps its scroll while pages append
struct MessageMP3DestinationListModel {
    list: ListUi,
    ids: Vec<String>,
    footer: MessageMP3DestinationFooter,
}

impl MessageMP3DestinationListModel {
    fn new(cx: &App) -> Self {
        Self { list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top), ids: Vec::new(), footer: MessageMP3DestinationFooter::Bottom }
    }

    /// footer is one row after the destinations
    /// a new page goes in front of it by splicing that row so the scroll stays
    fn sync(&mut self, ids: Vec<String>, footer: MessageMP3DestinationFooter) {
        if ids == self.ids {
            if footer != self.footer {
                let footer_index = ids.len();
                self.list.state().splice(footer_index..footer_index + 1, 1);
                self.footer = footer;
            }
            return;
        }
        if !self.ids.is_empty() && ids.len() > self.ids.len() && ids.starts_with(&self.ids) {
            let footer_index = self.ids.len();
            self.list.state().splice(footer_index..footer_index + 1, ids.len() - footer_index + 1);
            if self.footer != MessageMP3DestinationFooter::Bottom {
                message_mp3_arm_row_guard(footer_index);
            }
        } else {
            self.list.replaced(ids.len() + 1);
        }
        self.ids = ids;
        self.footer = footer;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageMP3DestinationFooter {
    LoadingMore,
    Retry,
    Pending,
    Bottom,
}

#[derive(IntoElement)]
struct MessageMP3DestinationScreen {
    list_model: Entity<MessageMP3DestinationListModel>,
    on_select: Rc<dyn Fn(MusicLocalPlaylistDestination, &mut Window, &mut App)>,
}

impl RenderOnce for MessageMP3DestinationScreen {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let services = MusicServicesStore::shared(cx);
        let destinations = services.update(cx, |services, cx| services.local_playlist_destinations(cx));
        let is_loading = services.update(cx, |services, cx| services.local_playlist_destinations_loading(cx));
        let error_text = services.update(cx, |services, cx| services.local_playlist_destinations_error_text(cx));

        let content = if is_loading {
            vstack(12.0)
                .size_full()
                .justify_center()
                .child(progress_view().tint(theme.primary._100))
                .child(SecondaryText::new(tr("music_message_import_loading_playlists")).px(13.0))
                .into_any_element()
        } else if destinations.is_empty() && error_text.as_deref().is_some_and(|error| !error.is_empty()) {
            MessageMP3DestinationState {
                icon: "wifi.exclamationmark",
                title: tr("music_message_import_playlists_error"),
                detail: error_text.unwrap_or_default(),
                action_title: Some(tr("music_message_import_retry")),
                action: Rc::new(|_, cx| {
                    MusicInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.refresh_local_playlist_destinations(cx));
                }),
            }
            .into_any_element()
        } else if destinations.is_empty() {
            MessageMP3DestinationState {
                icon: "music.note.list",
                title: tr("music_message_import_no_destinations"),
                detail: tr("music_message_import_no_destinations_hint"),
                action_title: None,
                action: Rc::new(|_, _| {}),
            }
            .into_any_element()
        } else {
            Self::destination_list(self.list_model.clone(), self.on_select.clone(), destinations, &theme, cx)
        };

        div().size_full().bg(theme.bg._100).child(content).on_appear("message-mp3-destination-screen", |_, cx| {
            MusicInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.load_local_playlist_destinations_if_needed(cx));
        })
    }
}

impl MessageMP3DestinationScreen {
    fn destination_list(
        list_model: Entity<MessageMP3DestinationListModel>,
        on_select: Rc<dyn Fn(MusicLocalPlaylistDestination, &mut Window, &mut App)>,
        destinations: Vec<MusicLocalPlaylistDestination>,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        let services = MusicServicesStore::shared(cx);
        let next_cursor = services.update(cx, |services, cx| services.local_playlist_destinations_next_cursor(cx));
        let has_more = services.update(cx, |services, cx| services.local_playlist_destinations_has_more(cx));
        let loading_more = services.update(cx, |services, cx| services.local_playlist_destinations_loading_more(cx));
        let page_failed = services.update(cx, |services, cx| services.local_playlist_destinations_page_failed(cx));
        let footer = if loading_more {
            MessageMP3DestinationFooter::LoadingMore
        } else if page_failed && has_more {
            MessageMP3DestinationFooter::Retry
        } else if has_more {
            MessageMP3DestinationFooter::Pending
        } else {
            MessageMP3DestinationFooter::Bottom
        };

        let ids: Vec<String> = destinations.iter().map(|destination| destination.id.clone()).collect();
        list_model.update(cx, |model, _| model.sync(ids, footer));

        let cache_key = MusicServicesStore::local_playlist_destinations_cache_key();
        let theme = theme.clone();
        let element = list_model.read(cx).list.element(move |index, _, _| {
            let Some(destination) = destinations.get(index) else {
                return Self::footer_row(footer, &theme);
            };
            let id = destination.id.clone();
            let selected = destination.clone();
            let on_select = on_select.clone();
            let options = SaiPaginationPrefetchOptions::new(
                index,
                destinations.len(),
                cache_key.clone(),
                next_cursor.clone(),
                has_more,
            )
            .threshold(1);
            let row = plain(SharedString::from(format!("message-mp3-destination-{id}")))
                .w_full()
                .on_press(move |window, cx| {
                    if message_mp3_row_guard_holds(index) {
                        return;
                    }
                    on_select(selected.clone(), window, cx)
                })
                .child(Self::destination_row(destination, &theme))
                .guide_anchor((index == 0).then_some(RiseGuideAnchor::ChatImportDestinationPlaylist))
                .sai_pagination_prefetch(
                    SharedString::from(format!("message-mp3-destination-prefetch-{id}")),
                    options,
                    |_, cx| {
                        MusicInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.load_more_local_playlist_destinations(cx));
                    },
                );
            vstack(0.0)
                .w_full()
                .child(row)
                .when(index + 1 != destinations.len(), |rows| {
                    rows.child(
                        div().w_full().pl(px(80.0)).child(div().w_full().h(px(0.5)).bg(theme.border._100)),
                    )
                })
                .into_any_element()
        });
        div().size_full().child(element).into_any_element()
    }

    fn footer_row(footer: MessageMP3DestinationFooter, theme: &AppTheme) -> AnyElement {
        let slot = || div().w_full().h(px(MESSAGE_MP3_DESTINATION_FOOTER_SLOT)).flex().items_center().justify_center();
        let content = match footer {
            MessageMP3DestinationFooter::LoadingMore => slot().child(progress_view().tint(theme.primary._100)),
            MessageMP3DestinationFooter::Pending => slot(),
            MessageMP3DestinationFooter::Retry => slot().child(
                plain("message-mp3-destination-retry")
                    .on_press(|_, cx| {
                        message_mp3_arm_row_guard(0);
                        MusicInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.retry_load_more_local_playlist_destinations(cx));
                    })
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(14.0, FontWeight::SEMIBOLD))
                            .text_color(theme.primary._100)
                            .child(tr("music_message_import_retry")),
                    ),
            ),
            MessageMP3DestinationFooter::Bottom => div().w_full(),
        };
        div().w_full().pb(px(12.0)).child(content).into_any_element()
    }

    fn destination_row(destination: &MusicLocalPlaylistDestination, theme: &AppTheme) -> impl IntoElement + use<> {
        let cover = if destination.cover_url.is_empty() {
            div()
                .size(px(52.0))
                .child(MusicCoverPlaceholder::new().icon_scale(0.34).corner_radius(10.0))
                .into_any_element()
        } else {
            MusicCoverImage::new(
                SharedString::from(format!("message-mp3-destination-cover-{}", destination.id)),
                destination.cover_url.clone(),
            )
            .size(52.0)
            .corner_radius(10.0)
            .into_any_element()
        };
        let role = if destination.viewer_role.to_lowercase() == VIEWER_ROLE_OWNER {
            tr("music_message_import_playlist_owner")
        } else {
            tr("music_message_import_playlist_member")
        };
        hstack(14.0)
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .child(cover)
            .child(
                vstack(3.0)
                    .align(HorizontalAlignment::Leading)
                    .child(
                        MainText::new(destination.title.clone())
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .child(MainText::new(role).px(12.0).color(theme.text.secondary).number_of_lines(1)),
            )
            .child(spacer_min_w(0.0))
            .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(13.0), theme.text.secondary.opacity(0.72), false))
    }
}

#[derive(IntoElement)]
struct MessageMP3DestinationState {
    icon: &'static str,
    title: String,
    detail: String,
    action_title: Option<String>,
    action: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl RenderOnce for MessageMP3DestinationState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let action = self.action;
        vstack(9.0)
            .size_full()
            .justify_center()
            .px(px(20.0))
            .children(IconUi::sized(self.icon, px(30.0), theme.text.secondary.opacity(0.7), false))
            .child(MainText::new(self.title).px(15.0).tac(TextAlignT::Center).font_weight(FontWeight::SEMIBOLD))
            .child(div().max_w(px(280.0)).child(SecondaryText::new(self.detail).px(12.0).tac(TextAlignT::Center)))
            .children(self.action_title.map(|action_title| {
                div().pt(px(4.0)).child(
                    plain("message-mp3-destination-state-action")
                        .on_press(move |window, cx| action(window, cx))
                        .child(
                            div()
                                .swiftui_font(Font::system_weight(14.0, FontWeight::SEMIBOLD))
                                .text_color(theme.primary._100)
                                .child(action_title),
                        ),
                )
            }))
    }
}

#[derive(IntoElement)]
struct MessageMP3MetadataScreen {
    source: MusicMessageMP3ImportSource,
    destination: MusicLocalPlaylistDestination,
    title: Binding<String>,
    description: Binding<String>,
    artist_name: Binding<String>,
    local_cover_file: Binding<Option<SaiWsOptimisticLocalFile>>,
    cover_preview: Binding<Option<CleverImage>>,
    on_add: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl MessageMP3MetadataScreen {
    fn can_add(&self, cx: &App) -> bool {
        self.destination.can_add_tracks && !trimming_whitespaces_and_newlines(&self.title.get(cx)).is_empty()
    }

    /// clamps the field on write so it never holds the tail
    fn clamped(value: Binding<String>, clamp: fn(&str) -> String) -> Binding<String> {
        let reader = value.clone();
        Binding::new(move |cx| reader.get(cx), move |raw: String, window, cx| value.set(clamp(&raw), window, cx))
    }

    fn metadata_field(
        id: &'static str,
        label: String,
        placeholder: String,
        value: Binding<String>,
        clamp: fn(&str) -> String,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        vstack(6.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(MainText::new(label).px(12.0).color(theme.text.secondary))
            .child(
                hstack(0.0)
                    .w_full()
                    .h(px(50.0))
                    .px(px(14.0))
                    .bg(theme.bg._200)
                    .rounded(px(14.0))
                    .overflow_hidden()
                    .child(
                        ShapedTextField::new(id, "", Self::clamped(value, clamp))
                            .prompt(Some(placeholder))
                            .placeholder_color(theme.text.secondary.opacity(0.72)),
                    ),
            )
    }
}

impl RenderOnce for MessageMP3MetadataScreen {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let can_add = self.can_add(cx);
        let description_is_empty = self.description.get(cx).is_empty();
        let on_add = self.on_add;
        scroll_view("message-mp3-metadata-scroll").shows_indicators(false).bg(theme.bg._100).child(
            vstack(16.0)
                .w_full()
                .pt(px(12.0))
                .child(
                    vstack(9.0)
                        .child(
                            MusicCoverPickerUi::new("message-mp3-cover", self.local_cover_file, self.cover_preview)
                                .existing_cover_url(self.source.thumbnail_url.clone())
                                .placeholder_label(Some(tr("music_message_import_cover_optional")))
                                .size(126.0)
                                .corner_radius(22.0),
                        )
                        .child(
                            SecondaryText::new(tr("music_message_import_cover_hint"))
                                .px(12.0)
                                .tac(TextAlignT::Center),
                        ),
                )
                .child(
                    vstack(10.0)
                        .w_full()
                        .px(px(16.0))
                        .align(HorizontalAlignment::Leading)
                        .child(Self::metadata_field(
                            "message-mp3-title",
                            tr("music_upload_title"),
                            tr("music_track_title_placeholder"),
                            self.title,
                            MusicMessageMP3ImportLimits::title,
                            &theme,
                        ))
                        .child(Self::metadata_field(
                            "message-mp3-artist",
                            tr("music_local_upload_artist"),
                            tr("music_unknown_artist"),
                            self.artist_name,
                            MusicMessageMP3ImportLimits::artist,
                            &theme,
                        ))
                        .child(
                            vstack(6.0)
                                .w_full()
                                .align(HorizontalAlignment::Leading)
                                .child(
                                    MainText::new(tr("music_message_import_description_optional"))
                                        .px(12.0)
                                        .color(theme.text.secondary),
                                )
                                .child(
                                    zstack(Alignment::TopLeading)
                                        .w_full()
                                        .bg(theme.bg._200)
                                        .rounded(px(14.0))
                                        .overflow_hidden()
                                        .children(description_is_empty.then(|| {
                                            div()
                                                .px(px(14.0))
                                                .py(px(13.0))
                                                .swiftui_font(Font::system_weight(15.0, FontWeight::NORMAL))
                                                .text_color(theme.text.secondary.opacity(0.72))
                                                .child(tr("music_message_import_description_placeholder"))
                                        }))
                                        .child(
                                            hstack(0.0)
                                                .w_full()
                                                .min_h(px(92.0))
                                                .max_h(px(120.0))
                                                .px(px(14.0))
                                                .py(px(13.0))
                                                .items_start()
                                                .child(
                                                    ShapedTextField::new(
                                                        "message-mp3-description",
                                                        "",
                                                        Self::clamped(
                                                            self.description,
                                                            MusicMessageMP3ImportLimits::description,
                                                        ),
                                                    )
                                                    .axis(Axis::Vertical)
                                                    .max_visible_lines(DESCRIPTION_MAX_VISIBLE_LINES)
                                                    .font_size(15.0),
                                                ),
                                        ),
                                ),
                        ),
                )
                .child(
                    div().w_full().px(px(16.0)).child(
                        hstack(10.0)
                            .w_full()
                            .p(px(13.0))
                            .bg(theme.bg._200)
                            .rounded(px(14.0))
                            .children(IconUi::sized("music.note.list", px(17.0), theme.primary._100, false))
                            .child(
                                vstack(2.0)
                                    .align(HorizontalAlignment::Leading)
                                    .child(
                                        MainText::new(self.destination.title.clone())
                                            .px(14.0)
                                            .font_weight(FontWeight::SEMIBOLD),
                                    )
                                    .child(SecondaryText::new(tr("music_message_import_background_hint")).px(11.0)),
                            )
                            .child(spacer_min_w(0.0)),
                    ),
                )
                .child(
                    div().w_full().px(px(16.0)).pb(px(28.0)).child(
                        ButtonUi::new("message-mp3-add")
                            .disabled(!can_add)
                            .on_press(move |window, cx| on_add(window, cx))
                            .child(MainText::new(tr("music_message_import_add")).font_weight(FontWeight::SEMIBOLD))
                            .guide_anchor(RiseGuideAnchor::ChatImportAddTrack),
                    ),
                ),
        )
    }
}
