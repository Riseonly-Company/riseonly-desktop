use gpui::prelude::*;
use gpui::{AnyElement, App, AppContext, Context, ElementId, FontWeight, SharedString, Window, div, px};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::keyboard_dismiss::KeyboardDismissModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::numeric_text::NumericText;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Edge, Transition};
use rise_ui::{IconUi, MainText};
use crate::modules::chat::components::chat_search::chat_search_overlay::ChatSearchHero;
use crate::modules::chat::components::chat::composer::chat_composer_types::{ChatBottomBarMetrics, resign_chat_keyboard};
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::motion::ease_in_out;
use crate::modules::chat::components::chat_search::chat_message_search_results_list::ChatMessageSearchResultsList;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::RiseConversationAuxiliaryLoadState;
use crate::modules::chat::engine::messages::rise_message_search_session::{ChatMessageSearchPolicy, RiseMessageSearchPresentationState};
use crate::modules::chat::shared::chat_message_search_presenter::ChatMessageSearchPresenter;
use crate::modules::chat::shared::chat_search_highlight::ChatMessageSearchHighlight;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    /// the profile search button left a request for this chat
    pub(in super::super) fn consume_pending_message_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if self.is_context_preview(cx) || chat_id.is_empty() {
            return;
        }
        let consumed = ChatMessageSearchPresenter::shared(cx).update(cx, |presenter, cx| presenter.consume(&chat_id, cx));
        if consumed {
            self.activate_message_search(window, cx);
            cx.on_next_frame(window, |_, window, cx| {
                cx.on_next_frame(window, |screen: &mut Self, _, cx| {
                    screen.message_search_focus_requests += 1;
                    cx.notify();
                });
            });
        }
    }

    pub(in super::super) fn activate_message_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.is_message_search_active {
            return;
        }
        self.cancel_message_selection(window, cx);
        self.state.message_search_query = String::new();
        self.state.message_search_index = 0;
        self.state.message_search_delivered_query = None;
        self.state.message_search_pending_step = 0;
        self.state.message_search_shows_list = false;
        self.state.is_message_search_active = true;
        cx.notify();
    }

    pub(in super::super) fn deactivate_message_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.is_message_search_active {
            return;
        }
        resign_chat_keyboard(window, cx);
        let chat_id = self.resolved_chat_id(cx);
        message_interactions(cx, |store, cx| store.end_chat_message_search(&chat_id, cx));
        self.state.is_message_search_active = false;
        self.state.message_search_shows_list = false;
        self.state.message_search_query = String::new();
        self.state.message_search_index = 0;
        self.state.message_search_delivered_query = None;
        self.state.message_search_pending_step = 0;
        cx.notify();
    }

    pub(in super::super) fn current_message_search_highlight(&self, cx: &mut App) -> Option<ChatMessageSearchHighlight> {
        if !self.state.is_message_search_active {
            return None;
        }
        let hits = self.message_search_state(cx).hits;
        let hit = hits.get(usize::try_from(self.state.message_search_index).ok()?)?;
        if hit.highlight_ranges.is_empty() {
            return None;
        }
        Some(ChatMessageSearchHighlight { message_id: hit.id.clone(), ranges: hit.highlight_ranges.clone() })
    }

    /// where the found word sits in its message as a share of the text
    pub(in super::super) fn message_search_focus(&self, message_id: &str, cx: &mut App) -> Option<f32> {
        if !self.state.is_message_search_active {
            return None;
        }
        let hits = self.message_search_state(cx).hits;
        let hit = hits.get(usize::try_from(self.state.message_search_index).ok()?)?;
        if hit.id != message_id && !hit.message.matches_message_id(message_id) {
            return None;
        }
        let first = hit.highlight_ranges.iter().map(|range| range.location).min()?;
        let total = hit.message.text.encode_utf16().count();
        if total == 0 {
            return None;
        }
        Some((first as f32 / total as f32).clamp(0.0, 1.0))
    }

    pub(in super::super) fn message_search_state(&self, cx: &mut App) -> RiseMessageSearchPresentationState {
        let chat_id = self.resolved_chat_id(cx);
        let query = self.state.message_search_query.clone();
        let mut search = message_services(cx, |store, cx| store.chat_message_search_state(&chat_id, &query, cx));
        if self.is_thread_screen() {
            return search;
        }
        if let Some(topic_id) = self.resolved_topic_id(cx) {
            search.hits.retain(|hit| hit.message.topic_id.as_deref().unwrap_or("") == topic_id);
            search.total_count = None;
        }
        search
    }

    pub(in super::super) fn handle_message_search_query_change(&mut self, query: &str, cx: &mut Context<Self>) {
        self.state.message_search_index = 0;
        self.state.message_search_pending_step = 0;
        self.state.message_search_delivered_query = None;
        let chat_id = self.resolved_chat_id(cx);
        message_interactions(cx, |store, cx| store.search_chat_messages(&chat_id, query, cx));
    }

    pub(in super::super) fn handle_message_search_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.is_message_search_active {
            return;
        }
        let search = self.message_search_state(cx);
        let query = trimming_whitespaces_and_newlines(&self.state.message_search_query).to_owned();
        if query.is_empty() || !search.did_load() {
            return;
        }
        let starved = search.hits.len() <= usize::try_from(self.state.message_search_index + self.state.message_search_pending_step).unwrap_or(0);
        if starved && search.has_more && !search.is_loading() {
            self.load_more_message_search(cx);
        }
        if self.state.message_search_delivered_query.as_deref() != Some(query.as_str()) {
            if search.hits.is_empty() && search.has_more {
                return;
            }
            self.state.message_search_delivered_query = Some(query);
            self.state.message_search_index = 0;
            self.state.message_search_pending_step = 0;
            if let Some(first) = search.hits.first() {
                self.jump_to_message_id(&first.id, window, cx);
            }
            cx.notify();
            return;
        }
        if self.state.message_search_pending_step != 0 {
            let target = self.state.message_search_index + self.state.message_search_pending_step;
            if let Some(hit) = usize::try_from(target).ok().and_then(|index| search.hits.get(index)) {
                self.state.message_search_pending_step = 0;
                self.state.message_search_index = target;
                self.jump_to_message_id(&hit.id, window, cx);
            } else if !search.has_more || matches!(search.load_state, RiseConversationAuxiliaryLoadState::Failed) {
                self.state.message_search_pending_step = 0;
            }
            cx.notify();
        }
    }

    /// plus one walks toward older hits like the up arrow
    /// minus one goes toward newer ones
    pub(in super::super) fn step_message_search(&mut self, delta: i64, window: &mut Window, cx: &mut Context<Self>) {
        let search = self.message_search_state(cx);
        if search.hits.is_empty() {
            return;
        }
        let target = self.state.message_search_index + delta;
        if let Some(hit) = usize::try_from(target).ok().and_then(|index| search.hits.get(index)) {
            self.state.message_search_index = target;
            self.state.message_search_pending_step = 0;
            self.jump_to_message_id(&hit.id, window, cx);
        } else if delta > 0 && search.has_more {
            self.state.message_search_pending_step = delta;
        } else {
            return;
        }
        if ChatMessageSearchPolicy::should_load_more(
            self.state.message_search_index,
            search.hits.len() as i64,
            search.has_more,
            search.is_loading(),
        ) {
            self.load_more_message_search(cx);
        }
        cx.notify();
    }

    pub(in super::super) fn load_more_message_search(&self, cx: &mut App) {
        let chat_id = self.resolved_chat_id(cx);
        let query = self.state.message_search_query.clone();
        message_interactions(cx, |store, cx| store.load_more_chat_message_search(&chat_id, &query, cx));
    }

    pub(in super::super) fn jump_to_message_search_hit(&mut self, index: i64, window: &mut Window, cx: &mut Context<Self>) {
        let search = self.message_search_state(cx);
        let Some(hit) = usize::try_from(index).ok().and_then(|index| search.hits.get(index)) else {
            return;
        };
        self.state.message_search_index = index;
        self.state.message_search_pending_step = 0;
        self.jump_to_message_id(&hit.id, window, cx);
        cx.notify();
    }

    pub(in super::super) fn message_search_results_layer(
        &mut self,
        top_inset: f32,
        bottom_inset: f32,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let search = self.message_search_state(cx);
        let this = cx.weak_entity();
        let did_load = search.did_load();
        let is_loading = search.is_loading();
        let list_key = trimming_whitespaces_and_newlines(&self.state.message_search_query).to_lowercase();
        let on_select_screen = this.clone();
        let list = ChatMessageSearchResultsList::new(
            ElementId::Name(format!("chat-message-search-results-{list_key}").into()),
            search.hits,
            self.state.message_search_index,
            is_loading,
            search.has_more,
            did_load,
            move |index, window, cx| {
                let _ = on_select_screen.update(cx, |screen, cx| {
                    screen.state.message_search_shows_list = false;
                    screen.jump_to_message_search_hit(index, window, cx);
                    cx.notify();
                });
            },
            Self::act(this, |screen, _, cx| screen.load_more_message_search(cx)),
        )
        .top_inset(top_inset)
        .bottom_inset(bottom_inset);
        div().size_full().child(list).into_any_element()
    }

    pub(in super::super) fn message_search_position_text(&self, cx: &mut App) -> String {
        let search = self.message_search_state(cx);
        if matches!(search.load_state, RiseConversationAuxiliaryLoadState::Loaded) && search.hits.is_empty() && !search.has_more {
            return tr("chat_search_no_results");
        }
        let shown = if search.hits.is_empty() { 0 } else { self.state.message_search_index + 1 };
        let total = search.total_count.unwrap_or(search.hits.len() as i64);
        tr("chat_search_match_position")
            .replace("{n}", &shown.to_string())
            .replace("{m}", &total.max(shown).to_string())
    }

    pub(in super::super) fn message_search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.weak_entity();
        let value = Binding::entity(
            cx.entity(),
            |screen: &Self| screen.state.message_search_query.clone(),
            |screen: &mut Self, new_value: String, cx: &mut Context<Self>| {
                if new_value == screen.state.message_search_query {
                    return;
                }
                screen.state.message_search_query = new_value.clone();
                screen.handle_message_search_query_change(&new_value, cx);
                cx.notify();
            },
        );
        let field = SearchFieldUi::new("chat-message-search-field", value)
            .placeholder(SharedString::from(tr("chat_search_in_chat_placeholder")))
            .on_submit(Self::act(this.clone(), |screen, window, cx| screen.step_message_search(1, window, cx)))
            .on_cancel(Self::act(this, |screen, window, cx| screen.deactivate_message_search(window, cx)))
            .uses_inline_text_field(true)
            .auto_focus(true)
            .focus_requests(self.message_search_focus_requests)
            .keeps_focus_on_submit(true);
        Self::appearing(
            "chat-message-search-field-presence",
            Transition::opacity().combined(Transition::scale(0.96)),
            ChatSearchHero::animation(),
            div().w_full().child(field),
            window,
            cx,
        )
    }

    pub(in super::super) fn message_search_close_button(&mut self, theme: &AppTheme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.weak_entity();
        let label = tr("cancel");
        let button = LiquidGlassButton::new("chat-message-search-close", LiquidGlassButtonShape::Circle)
            .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
            .accessibility_label(Some(label.clone()))
            .on_press(Self::act(this, |screen, window, cx| screen.deactivate_message_search(window, cx)))
            .children(IconUi::sized("xmark", px(15.0), theme.text.secondary, false));
        Self::appearing(
            "chat-message-search-close-presence",
            Transition::opacity().combined(Transition::scale(0.96)),
            ChatSearchHero::animation(),
            div().id("chat-message-search-close-host").tooltip_label(label).child(button),
            window,
            cx,
        )
    }

    pub(in super::super) fn message_search_bar(
        &mut self,
        theme: &AppTheme,
        safe_bottom_inset: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let search = self.message_search_state(cx);
        let this = cx.weak_entity();
        let has_query = !is_blank(&self.state.message_search_query);
        let primary = theme.text.primary;
        let position = has_query.then(|| {
            let text = self.message_search_position_text(cx);
            let plate = LiquidGlassButton::new("chat-message-search-position", LiquidGlassButtonShape::Capsule)
                .horizontal_padding(16.0)
                .vertical_padding(12.0)
                .accessibility_label(Some(text.clone()))
                .child(
                    NumericText::new("chat-message-search-position-text", text)
                        .animation(ease_in_out(0.18), search.hits.is_empty())
                        .text_element(move |text| {
                            MainText::new(text)
                                .px(15.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(primary)
                                .into_any_element()
                        }),
                );
            div().opacity(if search.hits.is_empty() && !search.did_load() { 0.6 } else { 1.0 }).child(plate)
        });
        let list_label = tr(if self.state.message_search_shows_list {
            "chat_search_show_as_chat"
        } else {
            "chat_search_show_as_list"
        });
        let toggle_screen = this.clone();
        let toggle = self.transitioned(
            "chat-message-search-list-toggle",
            has_query && !search.hits.is_empty(),
            Transition::opacity(),
            ease_in_out(0.18),
            window,
            cx,
            move |_, _, _| {
                div().child(
                    LiquidGlassButton::new("chat-message-search-list-button", LiquidGlassButtonShape::Capsule)
                        .horizontal_padding(16.0)
                        .vertical_padding(12.0)
                        .accessibility_label(Some(list_label.clone()))
                        .on_press(Self::act(toggle_screen, |screen, _, cx| {
                            screen.state.message_search_shows_list = !screen.state.message_search_shows_list;
                            cx.notify();
                        }))
                        .child(MainText::new(list_label).px(15.0).font_weight(FontWeight::SEMIBOLD).color(primary)),
                )
            },
        );
        Self::watch(&this, "chat-message-search-revision", search.revision, false, window, cx, |screen, _, _, window, cx| {
            screen.handle_message_search_results(window, cx);
        });
        let bottom_padding = self.composer_bottom_padding(safe_bottom_inset, cx);
        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ChatBottomBarMetrics::ROW_GAP))
            .w_full()
            .flex_none()
            .h(px(ChatBottomBarMetrics::CIRCLE))
            .children(position)
            .child(div().flex_1())
            .children(toggle);
        Self::appearing(
            "chat-message-search-bar-presence",
            Transition::move_edge(Edge::Bottom).combined(Transition::opacity()),
            ChatSearchHero::animation(),
            div()
                .w_full()
                .flex_none()
                .px(px(ChatBottomBarMetrics::H_PADDING))
                .pt(px(ChatBottomBarMetrics::TOP_PADDING))
                .pb(px(bottom_padding))
                .child(row),
            window,
            cx,
        )
    }

    pub(in super::super) fn message_search_step_buttons(&mut self, theme: &AppTheme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let search = self.message_search_state(cx);
        let this = cx.weak_entity();
        let hits_empty = search.hits.is_empty();
        let can_step_older = self.state.message_search_index + 1 < search.hits.len() as i64 || search.has_more;
        let can_step_newer = self.state.message_search_index > 0;
        let step_button = |id: &'static str, host: &'static str, symbol: &'static str, enabled: bool, delta: i64| {
            let button = LiquidGlassButton::new(id, LiquidGlassButtonShape::Circle)
                .size(ChatBottomBarMetrics::CIRCLE)
                .disabled(!enabled || hits_empty)
                .on_press(Self::act(this.clone(), move |screen, window, cx| screen.step_message_search(delta, window, cx)))
                .children(IconUi::sized(
                    symbol,
                    px(17.0),
                    theme.text.primary.opacity(if enabled { 1.0 } else { 0.35 }),
                    false,
                ));
            div().id(host).child(button)
        };
        let older = step_button("chat-message-search-older", "chat-message-search-older-host", "chevron.up", can_step_older, 1);
        let newer = step_button("chat-message-search-newer", "chat-message-search-newer-host", "chevron.down", can_step_newer, -1);
        let stack = div()
            .flex()
            .flex_col()
            .gap(px(ChatBottomBarMetrics::ROW_GAP))
            .child(older)
            .child(newer)
            .keyboard_dismiss_excluded();
        Self::appearing(
            "chat-message-search-steps-presence",
            ChatMessageNavigationButtonMotion::transition(),
            ChatSearchHero::animation(),
            div().opacity(if hits_empty { 0.0 } else { 1.0 }).child(stack),
            window,
            cx,
        )
    }
}
