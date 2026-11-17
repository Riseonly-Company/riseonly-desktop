use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash as _, Hasher as _};
use std::rc::Rc;
use std::sync::Arc;
use gpui::{AnyView, App, AppContext as _, Context, Window, px, size};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuActionHandler;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{ChatMessageRenderEntry, ChatMessageRenderEntryKind};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{RiseManualListBottomStickyHeader, RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListItemPlaceholderViewFactory, RiseManualListTopStickyHeader, RiseManualListTransactionAnchor};
use crate::modules::chat::components::chat::async_render::rise_manual_message_hosting_view::{RiseManualMessageHostingView, RiseManualMessageHostingViewOnHeightMismatch};
use crate::modules::chat::components::chat::async_render::rise_native_call_message_cell::RiseNativeCallMessageCell;
use crate::modules::chat::components::chat::async_render::rise_native_system_row_view::{RiseNativeSystemRowOpenPerson, RiseNativeSystemRowSelectTopic, RiseNativeSystemRowView, RiseNativeSystemRowViewConfiguration};
use crate::modules::chat::components::chat::async_render::rise_native_text_message_cell::{RiseNativeTextMessageCell, RiseNativeTextMessagePlaceholderView};
use crate::modules::chat::components::chat::async_render::rise_sticky_message_avatar_view::RiseStickyMessageAvatarView;
use crate::modules::chat::components::chat::bubble::message_sticker_view::RiseMessageStickerFastScrollPlaceholderView;
use crate::modules::chat::components::chat::bubble::message_call_view::MessageCallBubbleGeometry;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::message_system_pill::{MessageSystemFormatContext, MessageSystemFormatter};
use crate::modules::chat::shared::chat_saved_message_presentation::RiseMessageSenderIdentity;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::message::message_actions::message_types::MessageContentType;
use crate::modules::chat::components::chat::native_bubbles::native_bubble_registry::NativeBubbleRegistry;
use super::super::rise_message_list_view::*;

impl RiseMessageManualListView {
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn with_latest_interaction_flags(
        &self,
        prepared: &Rc<RiseMessageCollectionConfiguration>,
    ) -> Rc<RiseMessageCollectionConfiguration> {
        let Some(latest) = self.configuration.as_ref().filter(|latest| latest.chat_id == prepared.chat_id) else {
            return prepared.clone();
        };
        if latest.interactions_enabled == prepared.interactions_enabled
            && latest.allows_message_selection == prepared.allows_message_selection
        {
            return prepared.clone();
        }
        let mut current = prepared.as_ref().clone();
        current.interactions_enabled = latest.interactions_enabled;
        current.allows_message_selection = latest.allows_message_selection;
        Rc::new(current)
    }

    pub(in super::super) fn reusable_item(
        &self,
        id: &str,
        height: f32,
        reuse_identifier: &str,
        content_version: i64,
        can_reuse_prepared_items: bool,
        sticky_header: Option<&RiseManualListBottomStickyHeader>,
    ) -> Option<RiseManualListItem> {
        if !can_reuse_prepared_items {
            return None;
        }
        let item = self.manual_items_by_id.get(id)?;
        ((item.height - height).abs() <= 0.5
            && item.reuse_identifier == reuse_identifier
            && item.content_version == content_version
            && Self::sticky_header_is_current(Some(item), sticky_header))
            .then(|| item.clone())
    }

    /// why: a series avatar belongs to its author
    /// neighbours change the series without touching this row version so a kept item must carry
    /// the current series header
    pub(in super::super) fn sticky_header_is_current(
        item: Option<&RiseManualListItem>,
        fresh: Option<&RiseManualListBottomStickyHeader>,
    ) -> bool {
        let current = item.and_then(|item| item.bottom_sticky_header.as_ref());
        current.map(|header| &header.id) == fresh.map(|header| &header.id)
            && current.map(|header| header.content_version) == fresh.map(|header| header.content_version)
    }

    pub(in super::super) fn make_manual_items(
        &mut self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        native_ids: &HashSet<String>,
        retained_item_ids: &HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> RiseMessageManualListViewPreparedManualItemTable {
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "snapshot.make_items",
            HashMap::from([
                ("entries".to_owned(), candidate.configuration.snapshot.entries.len().to_string()),
                ("native".to_owned(), native_ids.len().to_string()),
                ("retained".to_owned(), retained_item_ids.len().to_string()),
            ]),
        );
        let table = self.make_manual_items_inner(candidate, native_ids, retained_item_ids, window, cx);
        ChatPerformanceTrace::shared().end_operation(trace_token, 2.0, HashMap::new());
        table
    }

    pub(in super::super) fn make_manual_items_inner(
        &mut self,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        native_ids: &HashSet<String>,
        retained_item_ids: &HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> RiseMessageManualListViewPreparedManualItemTable {
        let configuration = self.with_latest_interaction_flags(&candidate.configuration);
        let theme = configuration.theme_store.read(cx).current_theme().clone();
        let width = candidate.native_batch.width;
        let native_revision = self.manual_item_revision(self.item_environment_revision, self.presentation_revision);
        let snapshot = configuration.snapshot.clone();
        let entries = &snapshot.entries;
        let sticky_avatar_groups = self.sticky_avatar_groups_by_entry_id(&configuration, cx);
        let can_use_deferred_base = candidate.native_batch.purpose
            == RiseMessageManualListViewNativePreparationPurpose::DeferredHistory
            && self.rendered_configuration.as_ref().is_some_and(|rendered| {
                rendered.chat_id == configuration.chat_id
                    && rendered.snapshot.source_version == snapshot.source_version
            })
            && (self.manual_items_width - width).abs() <= 0.5
            && entries.iter().all(|entry| self.manual_items_by_id.contains_key(&entry.id));
        let mut result: HashMap<String, RiseManualListItem> = if can_use_deferred_base {
            self.manual_items_by_id.as_ref().clone()
        } else {
            let mut result = HashMap::with_capacity(entries.len());
            for id in retained_item_ids {
                if let Some(item) = self.manual_items_by_id.get(id) {
                    result.insert(id.clone(), item.clone());
                }
            }
            result
        };
        let mut updated_ids = HashSet::new();
        let requested_range = if can_use_deferred_base {
            candidate.native_batch.entry_range.clone()
        } else {
            0..entries.len()
        };
        let lower_bound = requested_range.start.min(entries.len());
        let upper_bound = lower_bound.max(requested_range.end.min(entries.len()));
        updated_ids.reserve((upper_bound - lower_bound).max(1));

        let can_reuse_prepared_items = (self.manual_items_width - width).abs() <= 0.5;

        for entry in &entries[lower_bound..upper_bound] {
            let is_retained_item = retained_item_ids.contains(&entry.id);
            let sticky_header =
                self.bottom_sticky_avatar_header(entry, &configuration, sticky_avatar_groups.get(&entry.id));
            let retained_version_is_current = is_retained_item
                && self.versions_by_id.get(&entry.id) == Some(&entry.content_version)
                && Self::sticky_header_is_current(self.manual_items_by_id.get(&entry.id), sticky_header.as_ref());
            let refreshes_retained_fallback_factory = retained_version_is_current
                && !self.rendered_native_ids.contains(&entry.id)
                && entry.message.as_ref().is_some_and(|message| {
                    !message.grid_media_items().is_empty() || message.first_audio_item().is_some()
                });
            if retained_version_is_current && !refreshes_retained_fallback_factory {
                continue;
            }

            if entry.kind != ChatMessageRenderEntryKind::Message {
                let height = RiseNativeSystemRowView::measure(entry, width, window);
                let reuse_identifier = RiseNativeSystemRowView::REUSE_IDENTIFIER;
                let content_version = entry.content_version ^ native_revision;
                if !is_retained_item
                    && let Some(item) =
                        self.reusable_item(
                            &entry.id,
                            height,
                            reuse_identifier,
                            content_version,
                            can_reuse_prepared_items,
                            sticky_header.as_ref(),
                        )
                {
                    result.insert(entry.id.clone(), item);
                    continue;
                }
                updated_ids.insert(entry.id.clone());
                let top_sticky_header = (entry.kind == ChatMessageRenderEntryKind::DateSeparator)
                    .then_some(RiseManualListTopStickyHeader);
                let view_entry = entry.clone();
                let view_configuration = configuration.clone();
                let view_theme = theme.clone();
                result.insert(
                    entry.id.clone(),
                    RiseManualListItem::new(
                        height,
                        reuse_identifier.to_owned(),
                        content_version,
                        RiseManualListItemMountPolicy::Immediate,
                        top_sticky_header,
                        sticky_header,
                        None,
                        None,
                        Rc::new(move |reusable_view: Option<AnyView>, _window: &mut Window, cx: &mut App| {
                            let view = reusable_view
                                .and_then(|view| view.downcast::<RiseNativeSystemRowView>().ok())
                                .unwrap_or_else(|| cx.new(RiseNativeSystemRowView::new));
                            let held_message = view_entry
                                .message
                                .clone()
                                .filter(|_| {
                                    view_entry.kind == ChatMessageRenderEntryKind::System
                                        && view_configuration.interactions_enabled
                                });
                            let hold_actions = match &held_message {
                                Some(message) => MessagePermissions::new(message, &view_configuration.chat, false)
                                    .context_menu_actions(view_configuration.allows_message_selection, false, cx),
                                None => Vec::new(),
                            };
                            let person_links = held_message
                                .as_ref()
                                .map(|message| {
                                    MessageSystemFormatter::person_links(
                                        message,
                                        &view_entry.label,
                                        &MessageSystemFormatContext::current_user_ids_now(cx),
                                    )
                                })
                                .unwrap_or_default();
                            let on_open_person = held_message.clone().map(|message| {
                                let on_action = view_configuration.on_action.clone();
                                Rc::new(move |user_id: String, window: &mut Window, cx: &mut App| {
                                    if let Some(on_action) = &on_action {
                                        on_action(
                                            &message.id,
                                            &ChatMessageAction::read_receipt_user_action(&user_id),
                                            window,
                                            cx,
                                        );
                                    }
                                }) as RiseNativeSystemRowOpenPerson
                            });
                            let on_hold_action = held_message.map(|message| {
                                let on_action = view_configuration.on_action.clone();
                                Rc::new(move |action: String, window: &mut Window, cx: &mut App| {
                                    if let Some(on_action) = &on_action {
                                        on_action(&message.id, &action, window, cx);
                                    }
                                }) as HoldContextMenuActionHandler
                            });
                            let on_select_topic = view_configuration.on_select_topic.clone().map(|on_select_topic| {
                                Rc::new(move |topic_id: Option<String>, window: &mut Window, cx: &mut App| {
                                    on_select_topic(topic_id.as_deref(), window, cx);
                                }) as RiseNativeSystemRowSelectTopic
                            });
                            let mut row_configuration = RiseNativeSystemRowViewConfiguration::new(view_theme.clone());
                            row_configuration.on_select_topic = on_select_topic;
                            row_configuration.on_select_date = view_configuration.on_select_date.clone();
                            row_configuration.hold_actions = hold_actions;
                            row_configuration.on_hold_action = on_hold_action;
                            row_configuration.person_links = person_links;
                            row_configuration.on_open_person = on_open_person;
                            row_configuration.row_width = Some(width);
                            view.update(cx, |view, cx| view.configure(&view_entry, row_configuration, cx));
                            view.into()
                        }),
                    ),
                );
                continue;
            }

            let (Some(message), Some(group)) = (entry.message.clone(), entry.group) else {
                self.insert_hosted_item(
                    entry,
                    &configuration,
                    candidate,
                    sticky_header,
                    width,
                    is_retained_item,
                    retained_version_is_current,
                    can_reuse_prepared_items,
                    &mut result,
                    &mut updated_ids,
                    window,
                    cx,
                );
                continue;
            };

            if message.content_type == MessageContentType::Call
                && message.call_info.is_some()
                && configuration.chat.r#type == ChatType::Direct
            {
                let is_own = MessageList::is_own_bubble(&message, &configuration.chat, cx);
                let height = MessageCallBubbleGeometry::row_height(group.is_first, group.is_last);
                let reuse_identifier = RiseNativeCallMessageCell::REUSE_IDENTIFIER;
                let content_version = entry.content_version ^ native_revision;
                if !is_retained_item
                    && let Some(item) =
                        self.reusable_item(
                            &entry.id,
                            height,
                            reuse_identifier,
                            content_version,
                            can_reuse_prepared_items,
                            sticky_header.as_ref(),
                        )
                {
                    result.insert(entry.id.clone(), item);
                    continue;
                }
                updated_ids.insert(entry.id.clone());
                let view_configuration = configuration.clone();
                let view_theme = theme.clone();
                result.insert(
                    entry.id.clone(),
                    RiseManualListItem::new(
                        height,
                        reuse_identifier.to_owned(),
                        content_version,
                        RiseManualListItemMountPolicy::Immediate,
                        None,
                        sticky_header,
                        None,
                        None,
                        Rc::new(move |reusable_view: Option<AnyView>, _window: &mut Window, cx: &mut App| {
                            let view = reusable_view
                                .and_then(|view| view.downcast::<RiseNativeCallMessageCell>().ok())
                                .unwrap_or_else(|| cx.new(RiseNativeCallMessageCell::new));
                            let on_action = view_configuration.on_action.clone();
                            let message_id = message.id.clone();
                            view.update(cx, |view, cx| {
                                view.configure(
                                    message.clone(),
                                    &view_configuration.chat,
                                    is_own,
                                    group.is_first,
                                    group.is_last,
                                    width,
                                    &view_theme,
                                    view_configuration.interactions_enabled,
                                    view_configuration.allows_message_selection,
                                    Rc::new(move |action: String, window: &mut Window, cx: &mut App| {
                                        if let Some(on_action) = &on_action {
                                            on_action(&message_id, &action, window, cx);
                                        }
                                    }),
                                    cx,
                                );
                            });
                            view.into()
                        }),
                    ),
                );
                continue;
            }

            let prepared_native = native_ids.contains(&entry.id).then(|| {
                candidate
                    .native_batch
                    .layouts
                    .get(&entry.id)
                    .cloned()
                    .zip(candidate.native_batch.ownership_by_id.get(&entry.id).copied())
            });
            if let Some(Some((prepared_layout, is_own))) = prepared_native {
                let (top, bottom) = rise_message_row_vertical_paddings(group);
                let height = (top + f32::from(prepared_layout.size.height) + bottom).ceil();
                NativeBubbleRegistry::report_row("text", entry, &configuration, width, height, cx);
                let reuse_identifier = RiseNativeTextMessageCell::REUSE_IDENTIFIER;
                let content_version = entry.content_version ^ native_revision;
                if !is_retained_item
                    && let Some(item) =
                        self.reusable_item(
                            &entry.id,
                            height,
                            reuse_identifier,
                            content_version,
                            can_reuse_prepared_items,
                            sticky_header.as_ref(),
                        )
                {
                    result.insert(entry.id.clone(), item);
                    continue;
                }
                updated_ids.insert(entry.id.clone());
                let placeholder_layout = prepared_layout.clone();
                let placeholder_chat = configuration.chat.clone();
                let placeholder_theme = theme.clone();
                let view_configuration = configuration.clone();
                let view_theme = theme.clone();
                result.insert(
                    entry.id.clone(),
                    RiseManualListItem::new(
                        height,
                        reuse_identifier.to_owned(),
                        content_version,
                        RiseManualListItemMountPolicy::Immediate,
                        None,
                        sticky_header,
                        Some(Rc::new(move |_window: &mut Window, cx: &mut App| {
                            let layout = placeholder_layout.clone();
                            let chat = placeholder_chat.clone();
                            let theme = placeholder_theme.clone();
                            cx.new(|_| {
                                RiseNativeTextMessagePlaceholderView::new(
                                    layout,
                                    &chat,
                                    is_own,
                                    group.is_first,
                                    group.is_last,
                                    &theme,
                                )
                            })
                            .into()
                        })),
                        None,
                        Rc::new(move |reusable_view: Option<AnyView>, _window: &mut Window, cx: &mut App| {
                            let view = reusable_view
                                .and_then(|view| view.downcast::<RiseNativeTextMessageCell>().ok())
                                .unwrap_or_else(|| cx.new(RiseNativeTextMessageCell::new));
                            let can_reply = view_configuration.interactions_enabled
                                && MessagePermissions::new(&message, &view_configuration.chat, is_own).can_reply(cx);
                            let reaction_configuration = view_configuration.clone();
                            let reaction_id = message.id.clone();
                            let action_configuration = view_configuration.clone();
                            let action_id = message.id.clone();
                            let can_double_tap = view_configuration.can_trigger_quick_reaction(&message, cx);
                            let tap_configuration = view_configuration.clone();
                            let tap_message = message.clone();
                            let message = message.clone();
                            let chat = view_configuration.chat.clone();
                            let layout = prepared_layout.clone();
                            view.update(cx, |view, cx| {
                                view.configure(
                                    message,
                                    chat,
                                    is_own,
                                    group.is_first,
                                    group.is_last,
                                    layout,
                                    &view_theme,
                                    can_reply,
                                    view_configuration.interactions_enabled,
                                    view_configuration.allows_message_selection,
                                    Rc::new(move |emoji: String, window: &mut Window, cx: &mut App| {
                                        if let Some(on_reaction) = &reaction_configuration.on_reaction {
                                            on_reaction(&reaction_id, &emoji, window, cx);
                                        }
                                    }),
                                    Rc::new(move |action: String, window: &mut Window, cx: &mut App| {
                                        if let Some(on_action) = &action_configuration.on_action {
                                            on_action(&action_id, &action, window, cx);
                                        }
                                    }),
                                    Rc::new(move |_cx: &App| can_double_tap),
                                    Rc::new(move |window: &mut Window, cx: &mut App| {
                                        tap_configuration.trigger_quick_reaction(&tap_message, window, cx);
                                    }),
                                    cx,
                                );
                            });
                            view.into()
                        }),
                    ),
                );
                continue;
            }

            if let Some(exact_height) = candidate.fallback_heights.get(&entry.id).copied()
                && RiseMessagePreparedGeometryPolicy::has_exact_fallback_height(Some(exact_height))
                && let Some(native) = NativeBubbleRegistry::kind_for(entry, &configuration, width, cx)
            {
                let kind = native.kind;
                let reuse_identifier = kind.reuse_identifier;
                if !is_retained_item
                    && let Some(item) =
                        self.reusable_item(
                            &entry.id,
                            exact_height,
                            reuse_identifier,
                            entry.content_version,
                            can_reuse_prepared_items,
                            sticky_header.as_ref(),
                        )
                {
                    result.insert(entry.id.clone(), item);
                    continue;
                }
                updated_ids.insert(entry.id.clone());
                let view_configuration = configuration.clone();
                let view_theme = theme.clone();
                result.insert(
                    entry.id.clone(),
                    RiseManualListItem::new(
                        exact_height,
                        reuse_identifier.to_owned(),
                        entry.content_version,
                        RiseManualListItemMountPolicy::Immediate,
                        None,
                        sticky_header,
                        None,
                        None,
                        Rc::new(move |reusable_view: Option<AnyView>, window: &mut Window, cx: &mut App| {
                            kind.make_view(
                                reusable_view,
                                &message,
                                group,
                                &view_configuration,
                                width,
                                &view_theme,
                                window,
                                cx,
                            )
                        }),
                    ),
                );
                continue;
            }

            self.insert_hosted_item(
                entry,
                &configuration,
                candidate,
                sticky_header,
                width,
                is_retained_item,
                retained_version_is_current,
                can_reuse_prepared_items,
                &mut result,
                &mut updated_ids,
                window,
                cx,
            );
        }
        RiseMessageManualListViewPreparedManualItemTable {
            items_by_id: Rc::new(result),
            updated_ids,
        }
    }

    /// hosted branch of make_manual_items, every row with no native cell of its own
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn insert_hosted_item(
        &mut self,
        entry: &Arc<ChatMessageRenderEntry>,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
        sticky_header: Option<RiseManualListBottomStickyHeader>,
        width: f32,
        is_retained_item: bool,
        retained_version_is_current: bool,
        can_reuse_prepared_items: bool,
        result: &mut HashMap<String, RiseManualListItem>,
        updated_ids: &mut HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let height_audit_list_view = self.list_view.downgrade();
        let fallback_height = match candidate.fallback_heights.get(&entry.id).copied() {
            Some(height) => height,
            None => self.estimated_row_height(entry, configuration, width, window, cx),
        };
        let height = fallback_height + self.video_note_growth(entry, configuration, width, cx);
        let hosted_revision = self.hosted_item_revision(entry, candidate);
        let reuse_identifier = RiseManualMessageHostingView::reuse_identifier(entry);
        let content_version = entry.content_version ^ hosted_revision;
        if !is_retained_item
            && let Some(item) =
                self.reusable_item(
                    &entry.id,
                    height,
                    &reuse_identifier,
                    content_version,
                    can_reuse_prepared_items,
                    sticky_header.as_ref(),
                )
        {
            result.insert(entry.id.clone(), item);
            return;
        }
        let retained_item = retained_version_is_current
            .then(|| self.manual_items_by_id.get(&entry.id))
            .flatten();
        let resolved_height = retained_item.map_or(height, |item| item.height);
        let resolved_reuse_identifier = retained_item.map_or(reuse_identifier, |item| item.reuse_identifier.clone());
        let resolved_content_version = retained_item.map_or(content_version, |item| item.content_version);
        let prepared_text = candidate.native_batch.hosted_text.get(&entry.id).cloned();
        let validates_estimated_height = !candidate.fallback_heights.contains_key(&entry.id)
            || entry.message.as_ref().is_some_and(|message| message.link_preview.is_some());
        let placeholder_view: Option<RiseManualListItemPlaceholderViewFactory> =
            match (entry.message.as_ref(), entry.group) {
                (Some(message), Some(group)) => message.first_sticker_item().cloned().map(|sticker| {
                    let is_own = MessageList::is_own_bubble(message, &configuration.chat, cx);
                    let reserves_avatar_space = MessageSizes::reserves_avatar_space(&configuration.chat, is_own);
                    let (top_padding, _) = rise_message_row_vertical_paddings(group);
                    let factory: RiseManualListItemPlaceholderViewFactory =
                        Rc::new(move |_window: &mut Window, cx: &mut App| -> AnyView {
                            cx.new(|cx| {
                                RiseMessageStickerFastScrollPlaceholderView::new(
                                    &sticker,
                                    is_own,
                                    reserves_avatar_space,
                                    top_padding,
                                    cx,
                                )
                            })
                            .into()
                        });
                    factory
                }),
                _ => None,
            };
        updated_ids.insert(entry.id.clone());
        let weak = cx.weak_entity();
        let view_entry = entry.clone();
        let view_configuration = configuration.clone();
        result.insert(
            entry.id.clone(),
            RiseManualListItem::new(
                resolved_height,
                resolved_reuse_identifier,
                resolved_content_version,
                RiseManualListItemMountPolicy::DeferredDuringFastScroll,
                None,
                sticky_header,
                placeholder_view,
                None,
                Rc::new(move |reusable_view: Option<AnyView>, _window: &mut Window, cx: &mut App| {
                    let view = reusable_view
                        .and_then(|view| view.downcast::<RiseManualMessageHostingView>().ok())
                        .unwrap_or_else(|| cx.new(RiseManualMessageHostingView::new));
                    let on_height_mismatch: Option<RiseManualMessageHostingViewOnHeightMismatch> =
                        validates_estimated_height.then(|| {
                            let weak = weak.clone();
                            let mismatch_entry = view_entry.clone();
                            let mismatch_configuration = view_configuration.clone();
                            Rc::new(move |actual_height: f32, cx: &mut App| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.record_runtime_height_mismatch(
                                        actual_height,
                                        &mismatch_entry,
                                        &mismatch_configuration,
                                        width,
                                        cx,
                                    );
                                });
                            }) as RiseManualMessageHostingViewOnHeightMismatch
                        });
                    view.update(cx, |view, cx| {
                        view.set_height_audit_scroll_view(height_audit_list_view.clone());
                        view.configure(
                            &view_entry,
                            &view_configuration,
                            prepared_text.clone(),
                            Some(width),
                            false,
                            Some(resolved_height),
                            on_height_mismatch,
                            cx,
                        );
                    });
                    view.into()
                }),
            ),
        );
    }

    pub(in super::super) fn sticky_avatar_groups_by_entry_id(
        &mut self,
        configuration: &RiseMessageCollectionConfiguration,
        cx: &mut App,
    ) -> HashMap<String, RiseMessageManualListViewStickyAvatarGroupContent> {
        let key = RiseMessageManualListViewStickyAvatarGroupCacheKey {
            chat_id: configuration.chat_id.clone(),
            source_version: configuration.snapshot.source_version,
            entry_count: configuration.snapshot.entries.len(),
            first_entry_id: configuration.snapshot.entries.first().map(|entry| entry.id.clone()),
            last_entry_id: configuration.snapshot.entries.last().map(|entry| entry.id.clone()),
            chat_type: configuration.chat.r#type,
            current_user_id: current_user_id(cx).unwrap_or_default(),
        };
        if let Some((cached_key, cached)) = &self.sticky_avatar_group_cache
            && *cached_key == key
        {
            return cached.clone();
        }
        if configuration.chat.r#type != ChatType::Group && configuration.chat.r#type != ChatType::Favourites {
            self.sticky_avatar_group_cache = Some((key, HashMap::new()));
            return HashMap::new();
        }

        let mut result = HashMap::with_capacity(configuration.snapshot.entries.len());
        let mut active_group: Option<RiseMessageManualListViewStickyAvatarGroupContent> = None;

        for entry in &configuration.snapshot.entries {
            let (Some(message), Some(group)) = (entry.message.as_ref(), entry.group) else {
                active_group = None;
                continue;
            };
            if entry.kind != ChatMessageRenderEntryKind::Message
                || MessageList::is_own_bubble(message, &configuration.chat, cx)
            {
                active_group = None;
                continue;
            }

            if group.is_first || active_group.is_none() {
                let participant = RiseMessageSenderIdentity::participant(message, &configuration.chat, cx);
                active_group = Some(RiseMessageManualListViewStickyAvatarGroupContent {
                    id: format!("avatar-{}-{}", participant.id, entry.id),
                    participant,
                });
            }
            if let Some(active_group) = &active_group {
                result.insert(entry.id.clone(), active_group.clone());
            }
            if group.is_last {
                active_group = None;
            }
        }
        self.sticky_avatar_group_cache = Some((key, result.clone()));
        result
    }

    pub(in super::super) fn bottom_sticky_avatar_header(
        &self,
        entry: &ChatMessageRenderEntry,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        group_content: Option<&RiseMessageManualListViewStickyAvatarGroupContent>,
    ) -> Option<RiseManualListBottomStickyHeader> {
        let group_content = group_content?.clone();
        let group = entry.group?;

        let mut version_hasher = DefaultHasher::new();
        self.item_environment_revision.hash(&mut version_hasher);
        group_content.id.hash(&mut version_hasher);
        group_content.participant.id.hash(&mut version_hasher);
        group_content.participant.name.hash(&mut version_hasher);
        group_content.participant.more.logo.hash(&mut version_hasher);
        group_content.participant.is_deleted.hash(&mut version_hasher);
        let content_version = version_hasher.finish() as i64;
        let avatar_size = MessageSizes::avatar_size();
        let (top_inset, bottom_inset) = rise_message_row_vertical_paddings(group);
        let header_id = group_content.id.clone();
        let view_configuration = configuration.clone();

        Some(RiseManualListBottomStickyHeader {
            id: header_id,
            size: size(px(avatar_size), px(avatar_size)),
            leading_inset: MessageSizes::avatar_leading_inset(),
            top_inset,
            bottom_inset,
            reuse_identifier: RiseStickyMessageAvatarView::REUSE_IDENTIFIER.to_owned(),
            content_version,
            view: Rc::new(move |reusable_view: Option<AnyView>, _window: &mut Window, cx: &mut App| {
                let view = reusable_view
                    .and_then(|view| view.downcast::<RiseStickyMessageAvatarView>().ok())
                    .unwrap_or_else(|| cx.new(RiseStickyMessageAvatarView::new));
                let participant = group_content.participant.clone();
                let tap_configuration = view_configuration.clone();
                let tap_participant = participant.clone();
                let color_scope_id = view_configuration.chat.id.clone();
                view.update(cx, |view, cx| {
                    view.configure(
                        Some(participant.more.logo.clone()).filter(|logo| !logo.is_empty()),
                        Some(participant.name.clone()),
                        Some(participant.id.clone()),
                        Some(color_scope_id),
                        participant.is_deleted,
                        avatar_size,
                        Rc::new(move |window: &mut Window, cx: &mut App| {
                            if let Some(on_open) = &tap_configuration.on_open_sender_profile {
                                on_open(&tap_participant, window, cx);
                            }
                        }),
                        cx,
                    );
                });
                view.into()
            }),
        })
    }

    pub(in super::super) fn hosted_item_revision(
        &self,
        entry: &ChatMessageRenderEntry,
        candidate: &RiseMessageManualListViewPreparedSnapshotCandidate,
    ) -> i64 {
        let mut hasher = DefaultHasher::new();
        self.item_environment_revision.hash(&mut hasher);
        self.presentation_revision.hash(&mut hasher);
        if let Some(message) = entry.message.as_ref()
            && message.first_audio_item().is_some()
        {
            candidate.configuration_revision.hash(&mut hasher);
        }
        hasher.finish() as i64
    }

    pub(in super::super) fn manual_item_revision(&self, configuration_revision: u64, presentation_revision: u64) -> i64 {
        let mut hasher = DefaultHasher::new();
        configuration_revision.hash(&mut hasher);
        presentation_revision.hash(&mut hasher);
        hasher.finish() as i64
    }

    pub(in super::super) fn enqueue_rendered_items(
        &mut self,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        anchor: RiseManualListTransactionAnchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.enqueue_rendered_items_animated(configuration, anchor, false, None, window, cx);
    }

    pub(in super::super) fn discard_list_transaction(&mut self, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.apply_generation != generation {
            return;
        }
        self.is_applying_snapshot = false;
        self.native_preparation_width = 0.0;
        self.rendered_preparation_width = 0.0;
        self.pending_width_repreparation = true;
        cx.defer_in(window, |this, window, cx| this.flush_deferred_work_if_possible(window, cx));
    }
}
