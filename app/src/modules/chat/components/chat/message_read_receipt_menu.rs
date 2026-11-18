use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, Hsla, SharedString, Subscription, Window, white};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{tr, tr_plural};
use rise_platform::haptics::selection_changed;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAccessory;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_page_host::{
    HoldContextMenuScrollContent, HoldContextMenuScrollViewportModifiers,
};
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::reaction_emoji_glyph::ReactionEmojiGlyph;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::gifts::components::gift_worn_cosmetics::GiftWornCosmeticsModifiers;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::RiseConversationAuxiliaryLoadState;
use crate::modules::chat::engine::messages::rise_message_read_receipt_presentation::{
    MessageReadReceiptReactor, RiseMessageReadReceiptFormatSeed, RiseMessageReadReceiptPresentation,
    RiseMessageReadReceiptPresentationSnapshot, RiseMessageReadReceiptSummaryKind,
    RiseMessageReadReceiptUserPresentation,
};
use crate::modules::chat::shared::message_permissions::ChatMessageAction;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageReaction, ReactionReactor,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

const CONTENT_FADE_DURATION: f64 = 0.16;
const SUMMARY_READERS_SHOWN: usize = 3;
const AVATAR_OVERLAP: f32 = -9.0;
const PLACEHOLDER_TITLE_WIDTHS: [f32; 4] = [132.0, 96.0, 154.0, 112.0];
const PLACEHOLDER_SUBTITLE_WIDTHS: [f32; 4] = [74.0, 90.0, 66.0, 82.0];

pub enum MessageReadReceiptMenuMetrics {}

impl MessageReadReceiptMenuMetrics {
    pub const SUMMARY_HEIGHT: f32 = 62.0;
    pub const DETAILS_ROW_HEIGHT: f32 = 62.0;
    pub const DETAILS_MAX_HEIGHT: f32 = 434.0;
    pub const SUMMARY_AVATAR_SIZE: f32 = 28.0;
    pub const DETAILS_AVATAR_SIZE: f32 = 40.0;

    pub fn details_list_height(rows: usize) -> f32 {
        (rows.max(1) as f32 * Self::DETAILS_ROW_HEIGHT).min(Self::DETAILS_MAX_HEIGHT)
    }

    /// placeholder rows must take the height the resolved list will take
    /// else the panel resizes under the pointer when readers arrive
    pub fn placeholder_row_count(total_count: i64) -> usize {
        let known = if total_count > 0 { total_count as usize } else { 4 };
        known.min((Self::DETAILS_MAX_HEIGHT / Self::DETAILS_ROW_HEIGHT) as usize)
    }
}

pub enum MessageReadReceiptMenuPolicy {}

impl MessageReadReceiptMenuPolicy {
    /// who reacted with what from the reactors the message carries
    /// newest reaction of a person wins when the server lists them under more than one emoji
    pub fn reactions_by_user(reactions: &[MessageReaction]) -> HashMap<String, String> {
        let mut by_user: HashMap<String, (String, f64)> = HashMap::new();
        for reaction in reactions {
            for reactor in reaction.reactors.iter().filter(|reactor| !reactor.user_id.is_empty()) {
                if let Some(known) = by_user.get(&reactor.user_id)
                    && known.1 >= reactor.created_at
                {
                    continue;
                }
                by_user.insert(reactor.user_id.clone(), (reaction.emoji.clone(), reactor.created_at));
            }
        }
        by_user.into_iter().map(|(user_id, (emoji, _))| (user_id, emoji)).collect()
    }

    /// everyone the message names under a reaction, earliest reaction per person
    /// sender is left out, the menu opens on ones own message and the server never lists the sender
    pub fn reactors(reactions: &[MessageReaction], excluding_sender_id: &str) -> Vec<MessageReadReceiptReactor> {
        let mut by_user: HashMap<String, ReactionReactor> = HashMap::new();
        for reaction in reactions {
            for reactor in reaction
                .reactors
                .iter()
                .filter(|reactor| !reactor.user_id.is_empty() && reactor.user_id != excluding_sender_id)
            {
                if let Some(known) = by_user.get(&reactor.user_id)
                    && known.created_at <= reactor.created_at
                {
                    continue;
                }
                by_user.insert(reactor.user_id.clone(), reactor.clone());
            }
        }
        by_user
            .into_values()
            .map(|reactor| MessageReadReceiptReactor {
                id: reactor.user_id,
                name: trimming_whitespaces_and_newlines(&reactor.name).to_owned(),
                logo: reactor.avatar_url.unwrap_or_default(),
                reacted_at_milliseconds: (reactor.created_at * 1_000.0) as i64,
            })
            .collect()
    }

    pub fn stable_message_id(message: &GetMessageMessage, chat: &ChatInfo, is_own: bool) -> Option<String> {
        if !is_own
            || !message.is_read
            || (chat.r#type != ChatType::Direct && chat.r#type != ChatType::Group)
            || message.is_temp
            || message.is_pending_outgoing()
        {
            return None;
        }
        let server_id = message.sai_server_id();
        let message_id = trimming_whitespaces_and_newlines(&server_id);
        if message_id.is_empty() || GetSaiServerId::is_temp_id(Some(message_id)) {
            return None;
        }
        Some(message_id.to_owned())
    }
}

pub enum MessageReadReceiptMenuAccessory {}

impl MessageReadReceiptMenuAccessory {
    pub fn configuration(
        message: &GetMessageMessage,
        chat: &ChatInfo,
        is_own: bool,
        _cx: &mut App,
    ) -> Option<HoldContextMenuAccessory> {
        let message_id = MessageReadReceiptMenuPolicy::stable_message_id(message, chat, is_own)?;
        let chat_id = chat.id.clone();
        let is_direct = chat.r#type == ChatType::Direct;
        let reactions = MessageReadReceiptMenuPolicy::reactions_by_user(&message.reactions);
        let reactors = MessageReadReceiptMenuPolicy::reactors(&message.reactions, &message.sender_id);
        let summary_reactors = reactors.clone();
        let (summary_chat_id, summary_message_id) = (chat_id.clone(), message_id.clone());
        let (details_chat_id, details_message_id) = (chat_id.clone(), message_id.clone());
        Some(
            HoldContextMenuAccessory::new(
                MessageReadReceiptMenuMetrics::SUMMARY_HEIGHT,
                Rc::new(move |on_open_details: Rc<dyn Fn(&mut App)>| {
                    MessageReadReceiptSummary {
                        chat_id: summary_chat_id.clone(),
                        message_id: summary_message_id.clone(),
                        is_direct,
                        reactors: summary_reactors.clone(),
                        on_open_details,
                    }
                    .into_any_element()
                }),
                Rc::new(move |on_back: Rc<dyn Fn(&mut App)>, on_select_user: Rc<dyn Fn(String, &mut App)>| {
                    MessageReadReceiptDetails {
                        chat_id: details_chat_id.clone(),
                        message_id: details_message_id.clone(),
                        reactions_by_user: reactions.clone(),
                        reactors: reactors.clone(),
                        on_back,
                        on_select_user,
                    }
                    .into_any_element()
                }),
            )
            .on_prefetch(Rc::new(move |cx: &mut App| {
                load_read_receipts(&chat_id, &message_id, false, cx);
            })),
        )
    }
}

fn load_read_receipts(chat_id: &str, message_id: &str, force: bool, cx: &mut App) {
    MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
        store.load_message_read_receipts(chat_id, message_id, force, cx);
    });
}

fn named_id(chat_id: &str, message_id: &str, name: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("read-receipt-{chat_id}-{message_id}-{name}")))
}

struct MessageReadReceiptPresentationObserver {
    _observer: Subscription,
}

/// the shared presentation read, a change of it redraws the view holding this accessory
fn observed_snapshot(
    chat_id: &str,
    message_id: &str,
    window: &mut Window,
    cx: &mut App,
) -> RiseMessageReadReceiptPresentationSnapshot {
    let presentation = RiseMessageReadReceiptPresentation::shared(cx);
    let observed = presentation.clone();
    window.use_keyed_state(named_id(chat_id, message_id, "presentation"), cx, move |_, cx| {
        MessageReadReceiptPresentationObserver { _observer: cx.observe(&observed, |_, _, cx| cx.notify()) }
    });
    presentation.read(cx).snapshot(chat_id, message_id)
}

/// stored list with the reactors it may not hold yet folded in
fn snapshot_including_reactors(
    stored: &RiseMessageReadReceiptPresentationSnapshot,
    reactors: &[MessageReadReceiptReactor],
    cx: &mut App,
) -> RiseMessageReadReceiptPresentationSnapshot {
    if reactors.is_empty() {
        return stored.clone();
    }
    stored.including_reactors(reactors, || RiseMessageReadReceiptFormatSeed::current(cx).text_formatter())
}

/// true when a reaction is newer than the cached list
/// so the list is read again instead of waiting out the cache
fn cached_list_misses_reactor(
    stored: &RiseMessageReadReceiptPresentationSnapshot,
    reactors: &[MessageReadReceiptReactor],
) -> bool {
    if reactors.is_empty()
        || stored.summary_kind != Some(RiseMessageReadReceiptSummaryKind::Readers)
        || !stored.has_resolved_initial()
        || stored.is_local_hint
        || stored.total_count > stored.readers.len() as i64
    {
        return false;
    }
    let listed: std::collections::HashSet<&str> = stored.readers.iter().map(|reader| reader.id.as_str()).collect();
    reactors.iter().any(|reactor| !listed.contains(reactor.id.as_str()))
}

fn load_on_identity(chat_id: &str, message_id: &str, name: &str, force: bool, window: &mut Window, cx: &mut App) {
    let (load_chat_id, load_message_id) = (chat_id.to_owned(), message_id.to_owned());
    on_change(
        named_id(chat_id, message_id, name),
        format!("{chat_id}:{message_id}"),
        true,
        window,
        cx,
        move |_, _, _, cx| load_read_receipts(&load_chat_id, &load_message_id, force, cx),
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageReadReceiptSummaryContent {
    Loading,
    Readers,
    Timestamp,
    Retry,
}

struct MessageReadReceiptSummaryFade {
    opacity: Animated<f32>,
}

#[derive(IntoElement)]
struct MessageReadReceiptSummary {
    chat_id: String,
    message_id: String,
    is_direct: bool,
    reactors: Vec<MessageReadReceiptReactor>,
    on_open_details: Rc<dyn Fn(&mut App)>,
}

impl MessageReadReceiptSummary {
    fn content(&self, snapshot: &RiseMessageReadReceiptPresentationSnapshot) -> MessageReadReceiptSummaryContent {
        let retry_or_loading = if snapshot.load_state == RiseConversationAuxiliaryLoadState::Failed {
            MessageReadReceiptSummaryContent::Retry
        } else {
            MessageReadReceiptSummaryContent::Loading
        };
        if !snapshot.has_resolved_summary(self.is_direct) {
            return retry_or_loading;
        }
        if snapshot.summary_kind == Some(RiseMessageReadReceiptSummaryKind::Readers) && snapshot.total_count > 0 {
            return MessageReadReceiptSummaryContent::Readers;
        }
        if snapshot.representative_read_at_text.is_some() {
            return MessageReadReceiptSummaryContent::Timestamp;
        }
        retry_or_loading
    }

    fn id(&self, name: &str) -> ElementId {
        named_id(&self.chat_id, &self.message_id, name)
    }

    fn readers_content(&self, snapshot: &RiseMessageReadReceiptPresentationSnapshot, cx: &App) -> AnyElement {
        let backdrop = rise_ui::theme(cx).bg._200;
        let avatars = snapshot.readers.iter().take(SUMMARY_READERS_SHOWN).rev().enumerate().map(|(index, reader)| {
            div().when(index > 0, |this| this.ml(px(AVATAR_OVERLAP))).child(
                UserLogoUi::new(self.id(&format!("summary-avatar-{}", reader.id)))
                    .source((!reader.logo.is_empty()).then(|| SharedString::from(reader.logo.clone())))
                    .name(Some(SharedString::from(reader.name.clone())))
                    .user_id(Some(SharedString::from(reader.id.clone())))
                    .size(MessageReadReceiptMenuMetrics::SUMMARY_AVATAR_SIZE)
                    .show_border(true)
                    .border_color(Some(backdrop))
                    .show_story_ring(false)
                    .open_story_on_press(false),
            )
        });
        let trailing = div().flex().flex_row().children(avatars).into_any_element();
        let on_open_details = self.on_open_details.clone();
        plain(self.id("readers"))
            .w_full()
            .on_press(move |_, cx| {
                selection_changed();
                on_open_details(cx);
            })
            .child(self.row(
                tr_plural("message_read_receipts_views", snapshot.total_count),
                snapshot.representative_read_at_text.clone(),
                Some(trailing),
                cx,
            ))
            .into_any_element()
    }

    fn loading_content(&self, cx: &App) -> AnyElement {
        div()
            .w_full()
            .min_h(px(MessageReadReceiptMenuMetrics::SUMMARY_HEIGHT))
            .px(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(div().opacity(0.35).child(Self::double_check(cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(5.0))
                    .child(SkeletonLine::new(96.0, 12.0))
                    .child(SkeletonLine::new(62.0, 9.0)),
            )
            .child(spacer_min_w(4.0))
            .when(!self.is_direct, |this| {
                this.child(div().flex().flex_row().children((0..SUMMARY_READERS_SHOWN).map(|index| {
                    div()
                        .when(index > 0, |this| this.ml(px(AVATAR_OVERLAP)))
                        .child(SkeletonCircle::new(MessageReadReceiptMenuMetrics::SUMMARY_AVATAR_SIZE))
                })))
            })
            .skeleton_shimmer(None)
            .into_any_element()
    }

    fn retry_content(&self, cx: &App) -> AnyElement {
        let primary = rise_ui::theme(cx).primary._100;
        let (chat_id, message_id) = (self.chat_id.clone(), self.message_id.clone());
        plain(self.id("retry"))
            .w_full()
            .min_h(px(MessageReadReceiptMenuMetrics::SUMMARY_HEIGHT))
            .px(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .on_press(move |_, cx| load_read_receipts(&chat_id, &message_id, true, cx))
            .child(
                div()
                    .size(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("arrow.clockwise", px(16.0), primary, false)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(2.0))
                    .child(
                        MainText::new(tr("message_read_receipts_unavailable"))
                            .px(13.0)
                            .secondary(true)
                            .number_of_lines(1)
                            .minimum_scale_factor(0.8),
                    )
                    .child(
                        MainText::new(tr("message_read_receipts_retry"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(primary)
                            .number_of_lines(1),
                    ),
            )
            .child(spacer_min_w(0.0))
            .into_any_element()
    }

    fn row(&self, title: String, subtitle: Option<String>, trailing: Option<AnyElement>, cx: &App) -> AnyElement {
        div()
            .w_full()
            .min_h(px(MessageReadReceiptMenuMetrics::SUMMARY_HEIGHT))
            .px(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .child(div().mr(px(10.0)).child(Self::double_check(cx)))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(2.0))
                    .child(MainText::new(title).px(16.0).number_of_lines(1).minimum_scale_factor(0.8))
                    .children(
                        subtitle
                            .filter(|subtitle| !subtitle.is_empty())
                            .map(|subtitle| MainText::new(subtitle).px(13.0).secondary(true).number_of_lines(1)),
                    ),
            )
            .child(spacer_min_w(8.0))
            .children(trailing)
            .into_any_element()
    }

    fn double_check(cx: &App) -> MessageReadReceiptDoubleCheck {
        MessageReadReceiptDoubleCheck {
            size: 15.0,
            color: rise_ui::theme(cx).text.primary,
            frame_width: 24.0,
            frame_height: 24.0,
        }
    }
}

impl RenderOnce for MessageReadReceiptSummary {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let stored = observed_snapshot(&self.chat_id, &self.message_id, window, cx);
        let force = cached_list_misses_reactor(&stored, &self.reactors);
        let snapshot = snapshot_including_reactors(&stored, &self.reactors, cx);
        load_on_identity(&self.chat_id, &self.message_id, "summary-load", force, window, cx);
        let content = self.content(&snapshot);
        let fade = window.use_keyed_state(self.id("summary-fade"), cx, |_, _| MessageReadReceiptSummaryFade {
            opacity: Animated::new(1.0),
        });
        let fade_state = fade.clone();
        on_change(self.id("summary-content"), content, false, window, cx, move |_, _, _, cx| {
            fade_state.update(cx, |fade, cx| {
                fade.opacity.jump(0.0);
                fade.opacity.set_in(1.0, ease_out(CONTENT_FADE_DURATION), RiseClock::monotonic(), cx);
                cx.notify();
            });
        });
        let opacity = fade.read(cx).opacity.frame_value(RiseClock::monotonic(), window);
        let body = match content {
            MessageReadReceiptSummaryContent::Loading => self.loading_content(cx),
            MessageReadReceiptSummaryContent::Readers => self.readers_content(&snapshot, cx),
            MessageReadReceiptSummaryContent::Timestamp => self.row(
                tr("chat_message_status_read"),
                snapshot.representative_read_at_text.clone(),
                None,
                cx,
            ),
            MessageReadReceiptSummaryContent::Retry => self.retry_content(cx),
        };
        div()
            .min_w(px(HoldContextMenuMetrics::MENU_MIN_WIDTH))
            .max_w(px(HoldContextMenuMetrics::MENU_MAX_WIDTH))
            .min_h(px(MessageReadReceiptMenuMetrics::SUMMARY_HEIGHT))
            .opacity(opacity)
            .child(body)
    }
}

#[derive(IntoElement)]
struct MessageReadReceiptDetails {
    chat_id: String,
    message_id: String,
    reactions_by_user: HashMap<String, String>,
    reactors: Vec<MessageReadReceiptReactor>,
    on_back: Rc<dyn Fn(&mut App)>,
    on_select_user: Rc<dyn Fn(String, &mut App)>,
}

impl MessageReadReceiptDetails {
    fn id(&self, name: &str) -> ElementId {
        named_id(&self.chat_id, &self.message_id, name)
    }

    fn details_content(&self, snapshot: &RiseMessageReadReceiptPresentationSnapshot, cx: &App) -> AnyElement {
        let is_failed = snapshot.load_state == RiseConversationAuxiliaryLoadState::Failed;
        if snapshot.readers.is_empty() && (!snapshot.has_resolved_initial() || (snapshot.is_local_hint && !is_failed)) {
            return self.placeholder_rows(MessageReadReceiptMenuMetrics::placeholder_row_count(snapshot.total_count));
        }
        if snapshot.readers.is_empty() {
            return self.empty_content(is_failed, cx);
        }
        let on_select_user = self.on_select_user.clone();
        let rows = snapshot.readers.iter().map(|reader| {
            let on_select_user = on_select_user.clone();
            let reader_id = reader.id.clone();
            plain(self.id(&format!("reader-{}", reader.id)))
                .w_full()
                .on_press(move |_, cx| on_select_user(ChatMessageAction::read_receipt_user_action(&reader_id), cx))
                .child(self.reader_row(reader, cx))
        });
        div()
            .id(self.id("readers-scroll"))
            .w_full()
            .h(px(MessageReadReceiptMenuMetrics::details_list_height(snapshot.readers.len())))
            .overflow_y_scroll()
            .hold_context_menu_scroll_viewport(cx)
            .child(div().w_full().flex().flex_col().child(HoldContextMenuScrollContent::new()).children(rows))
            .into_any_element()
    }

    fn empty_content(&self, is_failed: bool, cx: &App) -> AnyElement {
        let primary = rise_ui::theme(cx).primary._100;
        let (chat_id, message_id) = (self.chat_id.clone(), self.message_id.clone());
        div()
            .w_full()
            .min_h(px(150.0))
            .p(px(24.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .child(
                MainText::new(if is_failed {
                    tr("message_read_receipts_unavailable")
                } else {
                    tr("message_read_receipts_empty")
                })
                .px(15.0)
                .tac(TextAlignT::Center)
                .secondary(true),
            )
            .child(
                plain(self.id("empty-retry"))
                    .on_press(move |_, cx| load_read_receipts(&chat_id, &message_id, true, cx))
                    .child(
                        MainText::new(tr("message_read_receipts_retry"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(primary),
                    ),
            )
            .into_any_element()
    }

    fn reader_row(&self, reader: &RiseMessageReadReceiptUserPresentation, cx: &App) -> AnyElement {
        let secondary = rise_ui::theme(cx).text.secondary;
        let name_color = if reader.is_premium { PremiumNameColor::parse(reader.name_color.as_deref()) } else { None };
        div()
            .w_full()
            .h(px(MessageReadReceiptMenuMetrics::DETAILS_ROW_HEIGHT))
            .px(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .child(
                UserLogoUi::new(self.id(&format!("reader-avatar-{}", reader.id)))
                    .source((!reader.logo.is_empty()).then(|| SharedString::from(reader.logo.clone())))
                    .name(Some(SharedString::from(reader.name.clone())))
                    .user_id(Some(SharedString::from(reader.id.clone())))
                    .size(MessageReadReceiptMenuMetrics::DETAILS_AVATAR_SIZE)
                    .show_border(true)
                    .open_story_on_press(false)
                    .gift_worn_frame(
                        self.id(&format!("reader-frame-{}", reader.id)),
                        Some(reader.id.clone()),
                        MessageReadReceiptMenuMetrics::DETAILS_AVATAR_SIZE,
                    ),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(3.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(5.0))
                            .child(MainText::new(reader.name.clone()).px(16.0).color(name_color).number_of_lines(1))
                            .when(reader.is_premium, |this| {
                                this.child(
                                    PremiumBadge::new()
                                        .user_id(Some(SharedString::from(reader.id.clone())))
                                        .emoji(reader.premium_badge_emoji.clone().map(SharedString::from))
                                        .size(13.0),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(7.0))
                            .child(MessageReadReceiptDoubleCheck {
                                size: 11.0,
                                color: secondary,
                                frame_width: 18.0,
                                frame_height: 16.0,
                            })
                            .child(
                                MainText::new(reader.read_at_text.clone()).px(13.0).secondary(true).number_of_lines(1),
                            ),
                    ),
            )
            .child(spacer_min_w(0.0))
            .children(self.reactions_by_user.get(&reader.id).map(|reaction| {
                div()
                    .size(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(ReactionEmojiGlyph::new(reaction.clone(), 22.0))
            }))
            .into_any_element()
    }

    fn placeholder_rows(&self, count: usize) -> AnyElement {
        div()
            .w_full()
            .h(px(MessageReadReceiptMenuMetrics::details_list_height(count)))
            .flex()
            .flex_col()
            .children((0..count).map(|index| {
                div()
                    .w_full()
                    .h(px(MessageReadReceiptMenuMetrics::DETAILS_ROW_HEIGHT))
                    .px(px(16.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(SkeletonCircle::new(MessageReadReceiptMenuMetrics::DETAILS_AVATAR_SIZE))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap(px(6.0))
                            .child(SkeletonLine::new(PLACEHOLDER_TITLE_WIDTHS[index % 4], 12.0))
                            .child(SkeletonLine::new(PLACEHOLDER_SUBTITLE_WIDTHS[index % 4], 9.0)),
                    )
                    .child(spacer_min_w(0.0))
            }))
            .skeleton_shimmer(None)
            .into_any_element()
    }
}

impl RenderOnce for MessageReadReceiptDetails {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let stored = observed_snapshot(&self.chat_id, &self.message_id, window, cx);
        let force = cached_list_misses_reactor(&stored, &self.reactors);
        let snapshot = snapshot_including_reactors(&stored, &self.reactors, cx);
        load_on_identity(&self.chat_id, &self.message_id, "details-load", force, window, cx);
        let text_primary = rise_ui::theme(cx).text.primary;
        let on_back = self.on_back.clone();
        div()
            .w(px(HoldContextMenuMetrics::MENU_MAX_WIDTH))
            .flex()
            .flex_col()
            .child(
                plain(self.id("back"))
                    .w_full()
                    .h(px(56.0))
                    .px(px(16.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .on_press(move |_, cx| on_back(cx))
                    .children(IconUi::sized("arrow.left", px(18.0), text_primary, false))
                    .child(MainText::new(tr("back")).px(17.0))
                    .child(spacer_min_w(0.0)),
            )
            .child(
                div().w_full().px(px(HoldContextMenuMetrics::MENU_SECTION_SEPARATOR_INSET)).child(
                    div()
                        .w_full()
                        .h(px(0.66))
                        .bg(white().opacity(HoldContextMenuMetrics::MENU_SEPARATOR_OPACITY as f32)),
                ),
            )
            .child(self.details_content(&snapshot, cx))
    }
}

#[derive(IntoElement)]
struct MessageReadReceiptDoubleCheck {
    size: f32,
    color: Hsla,
    frame_width: f32,
    frame_height: f32,
}

impl RenderOnce for MessageReadReceiptDoubleCheck {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let centred_left = (self.frame_width - self.size) / 2.0;
        let top = (self.frame_height - self.size) / 2.0;
        let offset = self.size / 5.0;
        let check = |left: f32| {
            div().absolute().left(px(left)).top(px(top)).children(IconUi::sized(
                "checkmark",
                px(self.size),
                self.color,
                false,
            ))
        };
        div()
            .relative()
            .flex_shrink_0()
            .w(px(self.frame_width))
            .h(px(self.frame_height))
            .child(check(centred_left - offset))
            .child(check(centred_left + offset))
    }
}
