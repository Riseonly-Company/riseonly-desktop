use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Bounds, Context, ElementId, Entity, EntityId, Hsla, IntoElement, Path, Pixels, Point, Render,
    RenderOnce, SharedString, StyleRefinement, TransformationMatrix, Window, canvas, div, point, prelude::*, px, size,
};
use rise_canon::rise_wire_methods;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::ThemeStore;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::frame_request::is_painted_on_screen;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::skeleton_ui::SkeletonListUi;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::message_voice_view::{reserve_waveform_swap_keys, waveform_bar_path};
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_media_save_actions::ChatProfileMediaSaveActions;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_message_jump::ChatProfileMessageJump;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_pagination::ChatProfilePaginationPrefetchModifiers;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_section_list::ChatProfileSectionList;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileFormatter, ChatProfileTabBottomSpacer, ChatProfileTabDataRender, ChatProfileTabScrollContainer,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileMediaEntry, ChatProfileSource,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab_window::ChatProfileTabHost;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const CACHE_ID: &str = rise_wire_methods::message::GET_AUDIO_MESSAGES;
const ROW_HEIGHT: f32 = 64.5;
const CONTENT_HEIGHT: f32 = 64.0;
const WAVE_BAR_WIDTH: f32 = 2.0;
const WAVE_BAR_SPACING: f32 = 2.0;

fn row_element_id(entry: &ChatProfileMediaEntry, part: &str) -> ElementId {
    ElementId::Name(format!("chat-profile-voice-{part}-{}", entry.id).into())
}

struct ChatProfileVoiceRow {
    entry: ChatProfileMediaEntry,
    chat_id: String,
    nav_store: Option<Entity<NavigationStore>>,
}

#[derive(IntoElement)]
pub(crate) struct ChatProfileVoiceTab {
    chat_id: String,
}

impl ChatProfileVoiceTab {
    pub(crate) fn new(chat_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into() }
    }

    fn chat_title(chat_id: &str, cx: &mut App) -> String {
        let services = ChatServicesStore::shared(cx);
        let app: &App = cx;
        services.read(app).chat_info(chat_id, app).map(|chat| chat.title).unwrap_or_default()
    }

    /// same player as in the bubble by message id, a voice started here keeps playing in the chat
    fn toggle(row: &ChatProfileVoiceRow, cx: &mut App) {
        let entry = &row.entry;
        let chat_title = Self::chat_title(&row.chat_id, cx);
        ChatVoiceMessagePlayer::shared(cx).update(cx, |player, cx| {
            player.toggle(
                entry.message_id.clone(),
                row.chat_id.clone(),
                chat_title,
                Some(entry.sender_name.clone()),
                entry.item.clone(),
                cx,
            );
        });
        if !AuthServicesStore::shared(cx).read(cx).is_current_user(&entry.sender_id) {
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.mark_media_played(&row.chat_id, &entry.message_id, cx);
            });
        }
    }

    fn row(
        entry: &ChatProfileMediaEntry,
        chat_id: &str,
        item_count: usize,
        entry_index_by_id: &HashMap<String, i64>,
        nav_store: Option<&Entity<NavigationStore>>,
        cx: &mut App,
    ) -> AnyElement {
        let separator = ThemeStore::shared(cx).read(cx).current_theme().border._200.opacity(0.4);
        let row = Rc::new(ChatProfileVoiceRow {
            entry: entry.clone(),
            chat_id: chat_id.to_owned(),
            nav_store: nav_store.cloned(),
        });
        let actions = Self::voice_entry_hold_actions(entry, chat_id, cx);
        let content_row = row.clone();
        let action_row = row.clone();
        let tap_row = row;
        let hold = HoldContextMenuContainer::new(
            row_element_id(entry, "hold"),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |window: &mut Window, cx: &mut App| Self::row_content(&content_row, window, cx)),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::handle_voice_entry_action(&action_row, &id, cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| Self::toggle(&tap_row, cx)));

        let scope = chat_id.to_owned();
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(hold.chat_profile_pagination_prefetch(
                row_element_id(entry, "prefetch"),
                entry_index_by_id.get(&entry.id).copied().unwrap_or(0),
                item_count,
                CACHE_ID,
                chat_id,
                None,
                move |_, cx| {
                    MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_audio_tab(&scope, cx));
                },
                cx,
            ))
            .child(div().w_full().pl(px(76.0)).child(div().w_full().h(px(0.5)).bg(separator)))
            .into_any_element()
    }

    fn row_content(row: &Rc<ChatProfileVoiceRow>, window: &mut Window, cx: &mut App) -> AnyElement {
        let entry = &row.entry;
        let (is_playing, is_loading, progress) = {
            let player = ChatVoiceMessagePlayer::shared(cx);
            let player = player.read(cx);
            (
                player.is_playing_message_id(&entry.message_id),
                player.is_loading_message_id(&entry.message_id),
                player.progress_for(&entry.message_id),
            )
        };
        let (primary, on_primary, text_secondary) = {
            let theme = ThemeStore::shared(cx).read(cx).current_theme();
            (theme.primary._100, theme.content_on(theme.primary._100), theme.text.secondary)
        };
        let props = ChatProfileVoiceRowProps {
            entry: entry.clone(),
            is_playing,
            is_loading,
            progress: if is_playing { 0.0 } else { progress },
            primary,
            on_primary,
            text_secondary,
        };
        ChatProfileVoiceRowView::element(row_element_id(entry, "content"), props, window, cx)
    }

    fn voice_entry_hold_actions(entry: &ChatProfileMediaEntry, chat_id: &str, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let mut items: Vec<HoldContextMenuAction> = Vec::new();
        if !entry.has_protected_content {
            items.push(HoldContextMenuAction::new("forward", tr("ctx_forward")).system_image("arrowshape.turn.up.right"));
            items.push(HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"));
        }
        items.extend(ChatProfileMediaSaveActions::actions(entry));
        items.push(HoldContextMenuAction::new("goto", tr("ctx_go_to_message")).system_image("arrow.turn.up.left"));
        if let Some(delete) = ChatProfileSource::delete_action(&entry.message_id, &entry.sender_id, chat_id, cx) {
            items.push(delete);
        }
        items
    }

    fn handle_voice_entry_action(row: &ChatProfileVoiceRow, id: &str, cx: &mut App) {
        let entry = &row.entry;
        let chat_id = row.chat_id.as_str();
        if ChatProfileMediaSaveActions::handle(id, entry, chat_id, cx) {
            return;
        }
        if let Some(for_everyone) = ChatProfileSource::deletes_for_everyone(id) {
            let ids = [entry.message_id.clone()];
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.delete_messages(&ids, chat_id, None, None, for_everyone, cx);
            });
            return;
        }
        match id {
            "forward" => {
                let Some(message) = ChatProfileSource::forwardable_message(entry, chat_id, cx) else {
                    return;
                };
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_messages(vec![message], cx));
            }
            "copy" => {
                if entry.has_protected_content {
                    return;
                }
                let link = RiseonlyPublicLink::Chat { id: chat_id.to_owned() }
                    .web_url(AppEnvironment::current())
                    .map(|link| link.absolute_string().to_string());
                if let Some(link) = link {
                    UIPasteboard::general().set_string(link.as_str(), cx);
                }
            }
            "goto" => {
                if let Some(nav_store) = &row.nav_store {
                    ChatProfileMessageJump::open(chat_id, None, &entry.message_id, nav_store, cx);
                }
            }
            _ => {}
        }
    }
}

impl ChatProfileVoiceTab {
    fn body(chat_id: &str, cx: &mut App) -> AnyElement {
        let chat_id = chat_id.to_string();
        let nav_store = env::<NavigationStoreKey>(cx);
        let entries = ChatProfileSource::voice_items(&chat_id, cx);
        let services = MessageServicesStore::shared(cx);
        let sections =
            services.update(cx, |services, cx| services.message_tab_media_sections(CACHE_ID, &chat_id, &entries, cx));
        let entry_index_by_id = Rc::new(services.update(cx, |services, cx| {
            services.message_tab_entry_index_by_id(
                CACHE_ID,
                &chat_id,
                || entries.iter().map(|entry| entry.id.clone()).collect(),
                cx,
            )
        }));
        let is_empty = entries.is_empty();
        let item_count = entries.len();

        let retry_chat_id = chat_id.clone();
        let content_chat_id = chat_id.clone();
        let loading_id = ElementId::Name(format!("chat-profile-voice-loading-{chat_id}").into());
        let scroll_id = ElementId::Name(format!("chat-profile-voice-scroll-{chat_id}").into());
        let list_id = ElementId::Name(format!("chat-profile-voice-list-{chat_id}").into());
        ChatProfileTabDataRender::new(
            CACHE_ID,
            chat_id,
            is_empty,
            "mic.fill",
            tr("chat_profile_empty_voice"),
            move |_, cx| {
                MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.load_initial_audio_tab(&retry_chat_id, None, true, cx);
                });
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(scroll_id, move |_, _| {
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .child(ChatProfileSectionList::new(list_id, sections, ROW_HEIGHT, move |entry, _, cx| {
                            Self::row(entry, &content_chat_id, item_count, &entry_index_by_id, nav_store.as_ref(), cx)
                        }))
                        .child(ChatProfileTabBottomSpacer)
                })
            },
            move |_, _| {
                ChatProfileTabScrollContainer::new(loading_id.clone(), |_, _| SkeletonListUi {
                    rows: 8,
                    ..SkeletonListUi::default()
                })
                .scroll_enabled(false)
                .shows_indicators(false)
            },
        )
        .into_any_element()
    }
}

impl RenderOnce for ChatProfileVoiceTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chat_id = self.chat_id;
        ChatProfileTabHost::hosted(
            SharedString::from(format!("chat-profile-voice-host-{chat_id}")),
            window,
            cx,
            |cx| {
                let player = ChatVoiceMessagePlayer::shared(cx);
                vec![cx.observe(&player, |_, _, cx| cx.notify())]
            },
            move |_, cx| Self::body(&chat_id, cx),
        )
    }
}

/// what one voice row draws, equal props keep the last frame
#[derive(Clone, PartialEq)]
struct ChatProfileVoiceRowProps {
    entry: ChatProfileMediaEntry,
    is_playing: bool,
    is_loading: bool,
    progress: f64,
    primary: Hsla,
    on_primary: Hsla,
    text_secondary: Hsla,
}

/// one voice row as its own cached view, player ticks dont rebuild the whole list
struct ChatProfileVoiceRowView {
    props: Option<ChatProfileVoiceRowProps>,
    wave_swap_key: Rc<Cell<u64>>,
}

struct ChatProfileVoiceRowHolder {
    row: Entity<ChatProfileVoiceRowView>,
}

impl ChatProfileVoiceRowView {
    fn element(id: ElementId, props: ChatProfileVoiceRowProps, window: &mut Window, cx: &mut App) -> AnyElement {
        let style = StyleRefinement::default().w_full().h(px(CONTENT_HEIGHT));
        let view = window
            .use_keyed_state(id, cx, |_, cx| ChatProfileVoiceRowHolder {
                row: cx.new(|_| Self { props: None, wave_swap_key: Rc::new(Cell::new(0)) }),
            })
            .read(cx)
            .row
            .clone();
        let stale = view.update(cx, |view, _| view.set_props(props));
        if stale {
            AnyView::from(view).into_any_element()
        } else {
            view.cached(style).replay_when_moved().into_any_element()
        }
    }

    fn set_props(&mut self, props: ChatProfileVoiceRowProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        let drawn_before = self.props.is_some();
        self.props = Some(props);
        drawn_before
    }
}

impl Render for ChatProfileVoiceRowView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(props) = self.props.clone() else {
            return div().into_any_element();
        };
        let ChatProfileVoiceRowProps { entry, is_playing, is_loading, progress, primary, on_primary, text_secondary } = props;
        let entry = &entry;
        let glyph = if is_loading {
            progress_view().tint(on_primary).control_size(ControlSize::Small).into_any_element()
        } else {
            div()
                .relative()
                .left(px(if is_playing { 0.0 } else { 1.0 }))
                .flex()
                .children(IconUi::sized(if is_playing { "pause.fill" } else { "play.fill" }, px(16.0), on_primary, true))
                .into_any_element()
        };
        let button = div().flex_none().child(
            plain(row_element_id(entry, "play"))
                .child(zstack(Alignment::Center).size(px(44.0)).rounded_full().bg(primary).child(glyph)),
        );

        let peaks = entry.item.waveform.clone();
        let dim = text_secondary.opacity(0.45);
        let live_message_id = is_playing.then(|| entry.message_id.clone());
        let swap_key = self.wave_swap_key.clone();
        let fixed_progress = progress;
        let wave = canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                let progress = match live_message_id.as_deref() {
                    Some(message_id) => {
                        let player = ChatVoiceMessagePlayer::shared(cx);
                        player.read(cx).progress_for(message_id)
                    }
                    None => fixed_progress,
                };
                paint_voice_wave(&swap_key, &peaks, progress, primary, dim, live_message_id, bounds, window);
            },
        )
        .w_full()
        .max_w(px(220.0))
        .h(px(18.0));

        hstack(12.0_f32)
            .w_full()
            .padding_edges(EdgeSet::HORIZONTAL, 16.0)
            .padding_edges(EdgeSet::VERTICAL, 10.0)
            .child(button)
            .child(
                vstack(4.0_f32)
                    .items_start()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(wave)
                    .child(
                        hstack(6.0_f32)
                            .w_full()
                            .child(
                                MainText::new(ChatProfileFormatter::duration(entry.item.duration.unwrap_or(0.0)))
                                    .px(12.0)
                                    .color(text_secondary),
                            )
                            .child(div().flex_none().size(px(2.0)).rounded_full().bg(text_secondary.opacity(0.4)))
                            .child(MainText::new(entry.sender_name.clone()).px(12.0).color(text_secondary))
                            .child(spacer())
                            .child(
                                MainText::new(ChatProfileFormatter::relative_date(entry.created_at))
                                    .px(11.0)
                                    .color(text_secondary),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn voice_wave_rects(peaks: &[f32], width: f32, height: f32) -> Vec<Bounds<Pixels>> {
    let total = peaks.len() as f32 * (WAVE_BAR_WIDTH + WAVE_BAR_SPACING);
    let scale = (width / total.max(1.0)).clamp(0.3, 1.0);
    let middle = height / 2.0;
    let bar_width = WAVE_BAR_WIDTH * scale;
    let mut x = 0.0_f32;
    let mut rects = Vec::with_capacity(peaks.len());
    for peak in peaks {
        if x >= width {
            break;
        }
        let bar_height = (peak * height).max(2.0);
        rects.push(Bounds::new(point(px(x), px(middle - bar_height / 2.0)), size(px(bar_width), px(bar_height))));
        x += (WAVE_BAR_WIDTH + WAVE_BAR_SPACING) * scale;
    }
    rects
}

/// number of bars whose middle is inside the played part of the width
fn voice_wave_played_count(rects: &[Bounds<Pixels>], width: f32, progress: f64) -> usize {
    let played = progress.clamp(0.0, 1.0) as f32 * width;
    rects.iter().take_while(|rect| f32::from(rect.origin.x + rect.size.width / 2.0) <= played).count()
}

#[allow(clippy::too_many_arguments)]
fn paint_voice_wave(
    swap_key: &Rc<Cell<u64>>,
    peaks: &[f32],
    progress: f64,
    accent: Hsla,
    dim: Hsla,
    live_message_id: Option<String>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let width = f32::from(bounds.size.width);
    let rects = Rc::new(voice_wave_rects(peaks, width, f32::from(bounds.size.height)));
    let played = voice_wave_played_count(&rects, width, progress);
    let base_key = reserve_waveform_swap_keys(rects.len().max(1));
    swap_key.set(base_key);
    for (index, rect) in rects.iter().enumerate() {
        if let Some(path) = waveform_bar_path(bounds.origin, *rect) {
            let color = if index < played { accent } else { dim };
            window.paint_swappable_path(path, color, base_key + index as u64, bounds.origin);
        }
    }
    let Some(message_id) = live_message_id else {
        return;
    };
    if !is_painted_on_screen(bounds, window) || window.transformation() != TransformationMatrix::unit() {
        return;
    }
    VoiceWaveSwaps {
        swap_key: swap_key.clone(),
        base_key,
        rects,
        width,
        accent,
        dim,
        message_id,
        played,
        view: window.current_view(),
    }
    .schedule(window);
}

struct VoiceWaveSwaps {
    swap_key: Rc<Cell<u64>>,
    base_key: u64,
    rects: Rc<Vec<Bounds<Pixels>>>,
    width: f32,
    accent: Hsla,
    dim: Hsla,
    message_id: String,
    played: usize,
    view: EntityId,
}

impl VoiceWaveSwaps {
    fn schedule(self, window: &Window) {
        window.on_next_frame(move |window, cx| self.tick(window, cx));
    }

    fn tick(mut self, window: &mut Window, cx: &mut App) {
        if self.swap_key.get() != self.base_key {
            return;
        }
        let (is_playing, progress) = {
            let player = ChatVoiceMessagePlayer::shared(cx);
            let player = player.read(cx);
            (player.is_playing_message_id(&self.message_id), player.progress_for(&self.message_id))
        };
        if !is_playing {
            return;
        }
        let played = voice_wave_played_count(&self.rects, self.width, progress);
        if played == self.played {
            self.schedule(window);
            return;
        }
        let origin = point(px(0.), px(0.));
        let swaps: Vec<(u64, Path<Pixels>, gpui::Background)> = self
            .rects
            .iter()
            .enumerate()
            .filter_map(|(index, rect)| {
                let color = if index < played { self.accent } else { self.dim };
                waveform_bar_path(origin, *rect).map(|path| (self.base_key + index as u64, path, color.into()))
            })
            .collect();
        if window.swap_paths(swaps) {
            self.played = played;
            self.schedule(window);
            return;
        }
        cx.notify(self.view);
    }
}
