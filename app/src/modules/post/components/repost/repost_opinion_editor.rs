use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, ElementId, Entity, FocusHandle, IntoElement, MouseDownEvent, Pixels, RenderOnce, SharedString,
    Task, Window, black,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{character_count, character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_platform::haptics::{self, NotificationKind};
use rise_theme::{RiseThemeAppearance, ThemeStore};
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::HoldContextMenuGlass;
use rise_ui::keyboard_dismiss::{KeyboardDismissModifiers, ScreenFocus};
use rise_ui::rich_text::rich_text_markup::RichEditorStyle;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, ease_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::TransitionModifiers;
use rise_ui::top_presenter::TopPresenter;
use rise_ui::layer_keys::{LayerCancel, LayerKeysExt};
use rise_widgets::buttons::composer_send_button::ComposerSendButton;

use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::chat::components::chat::composer::chat_composer_text_view::ChatComposerTextView;
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatBottomBarMetrics, ChatComposerCaretEdit, ChatComposerCaretEditResult, ChatComposerCustomEmojiRun,
    ChatComposerInputChromeModifiers, ChatDraftCustomEmoji, resign_chat_keyboard,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageCustomEmojiEntity, RemoteMediaURL};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{CustomEmojiEntityWire, MessageEntitiesCodec};
use crate::modules::post::components::repost::repost_opinion_editor_hooks::repost_opinion_editor_emoji_tab;
use crate::modules::post::components::repost::repost_overlay::{
    RepostMatchedGeometry, RepostSticker, RepostStickerStyle, RepostTransitionNamespace, RepostTransitionNamespaceKey,
};
use crate::modules::post::shared::post_field_rules::PostFieldRules;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    StickerAssetType, StickerAssetTypeInfer, StickerItem,
};

const OBJECT_REPLACEMENT: &str = "\u{FFFC}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepostOpinionTransitionPhase {
    Idle,
    Preparing,
    Opening,
    Open,
    Closing,
}

pub struct RepostOpinionEditorPresenter {
    pub is_visible: bool,
    pub text: String,
    initial_text: String,
    initial_entities: Option<String>,
    post: Option<GetPostFeedResponse>,
    session_id: String,
    source_instance_id: String,
    source_style: RepostStickerStyle,
    phase: RepostOpinionTransitionPhase,
    custom_emoji_drafts: Vec<ChatDraftCustomEmoji>,
    transition_task: Option<Task<()>>,
}

impl RepostOpinionEditorPresenter {
    pub const FLIGHT_SETTLE_MILLISECONDS: u64 = 640;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        AccountStateResetRegistry::register(&cx.entity(), cx);
        Self {
            is_visible: false,
            text: String::new(),
            initial_text: String::new(),
            initial_entities: None,
            post: None,
            session_id: String::new(),
            source_instance_id: String::new(),
            source_style: RepostStickerStyle::Feed,
            phase: RepostOpinionTransitionPhase::Idle,
            custom_emoji_drafts: Vec::new(),
            transition_task: None,
        }
    }

    pub fn character_limit() -> usize {
        PostFieldRules::repost_content().maximum().unwrap_or(0)
    }

    pub fn flight_animation() -> MotionSpec {
        spring(0.58, 0.86)
    }

    pub fn initial_text(&self) -> &str {
        &self.initial_text
    }

    pub fn initial_entities(&self) -> Option<&str> {
        self.initial_entities.as_deref()
    }

    pub fn post(&self) -> Option<&GetPostFeedResponse> {
        self.post.as_ref()
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn source_instance_id(&self) -> &str {
        &self.source_instance_id
    }

    pub fn source_style(&self) -> RepostStickerStyle {
        self.source_style
    }

    pub fn phase(&self) -> RepostOpinionTransitionPhase {
        self.phase
    }

    pub fn custom_emoji_drafts(&self) -> &[ChatDraftCustomEmoji] {
        &self.custom_emoji_drafts
    }

    pub fn can_start_transition(&self) -> bool {
        self.phase == RepostOpinionTransitionPhase::Idle
    }

    pub fn present(
        &mut self,
        post: GetPostFeedResponse,
        source_instance_id: String,
        source_style: RepostStickerStyle,
        cx: &mut Context<Self>,
    ) {
        if self.phase != RepostOpinionTransitionPhase::Idle {
            return;
        }
        self.transition_task = None;

        self.source_instance_id = source_instance_id;
        self.session_id = Uuid::new_v4().uuid_string();
        self.source_style = source_style;
        self.initial_text = PostFieldRules::repost_content().clamped(post.my_repost_content.as_deref().unwrap_or_default());
        self.initial_entities = post.my_repost_entities.clone();
        self.text = self.initial_text.clone();
        self.custom_emoji_drafts = Self::drafts(&post.my_repost_custom_emojis(), &self.initial_text);
        self.post = Some(post);
        self.phase = RepostOpinionTransitionPhase::Preparing;
        cx.notify();

        let opening_session_id = self.session_id.clone();
        self.transition_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(18)).await;
            let session_id = opening_session_id.clone();
            let opened = this.update(cx, move |this, cx| {
                if this.phase != RepostOpinionTransitionPhase::Preparing || this.session_id != session_id {
                    return false;
                }
                this.phase = RepostOpinionTransitionPhase::Opening;
                this.is_visible = true;
                cx.notify();
                true
            });
            if !matches!(opened, Ok(true)) {
                return;
            }

            cx.background_executor().timer(Duration::from_millis(Self::FLIGHT_SETTLE_MILLISECONDS)).await;
            let _ = this.update(cx, move |this, cx| {
                if this.phase != RepostOpinionTransitionPhase::Opening || this.session_id != opening_session_id {
                    return;
                }
                this.phase = RepostOpinionTransitionPhase::Open;
                cx.notify();
            });
        }));
    }

    pub fn matched_id(&self, source_instance_id: &str) -> Option<String> {
        (self.phase != RepostOpinionTransitionPhase::Idle && self.source_instance_id == source_instance_id)
            .then(|| self.session_id.clone())
    }

    pub fn should_hide_source(&self, source_instance_id: &str) -> bool {
        (self.is_visible || self.phase == RepostOpinionTransitionPhase::Closing) && self.source_instance_id == source_instance_id
    }

    pub fn limit_text(&mut self, cx: &mut Context<Self>) {
        let clamped = PostFieldRules::repost_content().clamped(&self.text);
        if clamped != self.text {
            self.text = clamped;
            cx.notify();
        }
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.phase == RepostOpinionTransitionPhase::Idle || self.phase == RepostOpinionTransitionPhase::Closing {
            return;
        }
        self.transition_task = None;
        let closing_session_id = self.session_id.clone();

        self.phase = RepostOpinionTransitionPhase::Closing;
        self.is_visible = false;
        cx.notify();

        self.transition_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(Self::FLIGHT_SETTLE_MILLISECONDS + 40)).await;
            let _ = this.update(cx, move |this, cx| {
                if this.phase != RepostOpinionTransitionPhase::Closing || this.session_id != closing_session_id {
                    return;
                }
                this.post = None;
                this.text = String::new();
                this.initial_text = String::new();
                this.initial_entities = None;
                this.custom_emoji_drafts = Vec::new();
                this.session_id = String::new();
                this.source_instance_id = String::new();
                this.phase = RepostOpinionTransitionPhase::Idle;
                cx.notify();
            });
        }));
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.transition_task = None;
        self.is_visible = false;
        self.text = String::new();
        self.initial_text = String::new();
        self.initial_entities = None;
        self.custom_emoji_drafts = Vec::new();
        self.post = None;
        self.session_id = String::new();
        self.source_instance_id = String::new();
        self.source_style = RepostStickerStyle::Feed;
        self.phase = RepostOpinionTransitionPhase::Idle;
        cx.notify();
    }

    pub fn can_submit(&self) -> bool {
        !trimming_whitespaces_and_newlines(&self.text).is_empty()
            || !trimming_whitespaces_and_newlines(&self.initial_text).is_empty()
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        let Some(post) = self.post.clone() else {
            return;
        };
        let value = PostFieldRules::repost_content()
            .schema
            .map_or_else(|| self.text.clone(), |schema| schema.normalized_clamped(&self.text));
        let entities = self.entities_json(&value);
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.edit_repost_opinion(&post, &value, entities, None, cx));
        let _ = haptics::notification(NotificationKind::Success);
        self.dismiss(cx);
    }

    pub fn append_custom_emoji(&mut self, item: StickerItem, selection: Option<NSRange>, cx: &mut Context<Self>) -> Option<NSRange> {
        if item.id.is_empty() || item.file_url.is_empty() {
            return None;
        }
        if character_count(&self.text) >= Self::character_limit() {
            return None;
        }
        let edit = ChatComposerCaretEdit::replace(
            &self.text,
            selection,
            OBJECT_REPLACEMENT,
            &mut self.custom_emoji_drafts,
            Some(ChatDraftCustomEmoji { placeholder: OBJECT_REPLACEMENT.to_owned(), item }),
        );
        Some(self.apply_caret_edit(edit, cx))
    }

    pub fn insert_text(&mut self, value: &str, selection: Option<NSRange>, cx: &mut Context<Self>) -> Option<NSRange> {
        let remaining = Self::character_limit().saturating_sub(character_count(&self.text));
        if remaining == 0 {
            return None;
        }
        let edit = ChatComposerCaretEdit::replace(
            &self.text,
            selection,
            character_prefix(value, remaining),
            &mut self.custom_emoji_drafts,
            None,
        );
        Some(self.apply_caret_edit(edit, cx))
    }

    pub fn delete_backward(&mut self, selection: Option<NSRange>, cx: &mut Context<Self>) -> Option<NSRange> {
        let edit = ChatComposerCaretEdit::delete_backward(&self.text, selection, &mut self.custom_emoji_drafts)?;
        Some(self.apply_caret_edit(edit, cx))
    }

    fn apply_caret_edit(&mut self, edit: ChatComposerCaretEditResult, cx: &mut Context<Self>) -> NSRange {
        self.text = edit.text;
        cx.notify();
        edit.caret
    }

    pub fn composer_custom_emoji_runs(&self, text: &str) -> Vec<ChatComposerCustomEmojiRun> {
        if self.custom_emoji_drafts.is_empty() {
            return Vec::new();
        }
        ChatComposerCaretEdit::custom_emoji_runs(text, &self.custom_emoji_drafts)
    }

    pub fn sync_custom_emoji_drafts(&mut self, runs: Vec<ChatComposerCustomEmojiRun>, cx: &mut Context<Self>) {
        let unchanged = runs.len() == self.custom_emoji_drafts.len()
            && runs.iter().zip(&self.custom_emoji_drafts).all(|(run, draft)| run.item == draft.item);
        if unchanged {
            return;
        }
        self.custom_emoji_drafts = runs
            .into_iter()
            .map(|run| ChatDraftCustomEmoji { placeholder: OBJECT_REPLACEMENT.to_owned(), item: run.item })
            .collect();
        cx.notify();
    }

    fn entities_json(&self, trimmed_text: &str) -> Option<String> {
        let wires: Vec<CustomEmojiEntityWire> = self
            .composer_custom_emoji_runs(trimmed_text)
            .into_iter()
            .map(|run| CustomEmojiEntityWire {
                offset: run.range.location as i64,
                length: run.range.length as i64,
                custom_emoji_id: run.item.id,
                file_url: RemoteMediaURL::optional_normalized(Some(run.item.file_url.as_str())).unwrap_or(run.item.file_url),
                asset_type: run.item.asset_type.wire().to_owned(),
                width: run.item.width,
                height: run.item.height,
                pack_id: run.item.pack_id,
            })
            .collect();
        if wires.is_empty() {
            return None;
        }
        MessageEntitiesCodec::encode(&wires, None)
    }

    /// stored opinion keeps one placeholder per custom emoji
    /// the entities say which emoji each placeholder is, in text order
    fn drafts(entities: &[MessageCustomEmojiEntity], text: &str) -> Vec<ChatDraftCustomEmoji> {
        if entities.is_empty() || !text.contains(OBJECT_REPLACEMENT) {
            return Vec::new();
        }
        let mut sorted: Vec<&MessageCustomEmojiEntity> = entities.iter().collect();
        sorted.sort_by_key(|entity| entity.offset);
        sorted
            .into_iter()
            .map(|entity| ChatDraftCustomEmoji {
                placeholder: OBJECT_REPLACEMENT.to_owned(),
                item: StickerItem {
                    id: entity.custom_emoji_id.clone(),
                    pack_id: entity.pack_id.clone(),
                    file_url: entity.file_url.clone(),
                    thumbnail_url: None,
                    width: entity.width,
                    height: entity.height,
                    duration_ms: None,
                    position: 0,
                    created_at_ms: 0,
                    associated_emojis: Vec::new(),
                    asset_type: StickerAssetType::infer(Some(entity.asset_type.as_str()), &entity.file_url),
                    mime_type: None,
                    codec: None,
                },
            })
            .collect()
    }
}

impl AccountResettable for RepostOpinionEditorPresenter {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        RepostOpinionEditorPresenter::reset_account_state(self, cx);
    }
}

struct RepostOpinionEditorState {
    chrome_visible: bool,
    picker_visible: bool,
    container_bounds: Option<Bounds<Pixels>>,
    input_height: f32,
    input_width: f32,
    focus_token: i64,
    selection: NSRange,
    selection_text: String,
    requested_selection: Option<NSRange>,
    selection_token: i64,
    flight: Animated<f32>,
    chrome_motion: Animated<f32>,
    picker_motion: Animated<f32>,
}

impl RepostOpinionEditorState {
    fn new() -> Self {
        Self {
            chrome_visible: false,
            picker_visible: false,
            container_bounds: None,
            input_height: ChatBottomBarMetrics::CIRCLE,
            input_width: 0.0,
            focus_token: 0,
            selection: NSRange::new(0, 0),
            selection_text: String::new(),
            requested_selection: None,
            selection_token: 0,
            flight: Animated::new(0.0),
            chrome_motion: Animated::new(0.0),
            picker_motion: Animated::new(0.0),
        }
    }

    fn set_chrome_visible(&mut self, visible: bool, spec: Option<MotionSpec>, cx: &mut Context<Self>) {
        self.chrome_visible = visible;
        self.chrome_motion.set_in(if visible { 1.0 } else { 0.0 }, spec, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn set_picker_visible(&mut self, visible: bool, spec: MotionSpec, cx: &mut Context<Self>) {
        self.picker_visible = visible;
        self.picker_motion.set_in(if visible { 1.0 } else { 0.0 }, spec, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn presenter_visibility_changed(&mut self, visible: bool, cx: &mut Context<Self>) {
        let flight = RepostOpinionEditorPresenter::flight_animation();
        self.flight.set_in(if visible { 1.0 } else { 0.0 }, flight, RiseClock::monotonic(), cx);
        if !visible {
            self.set_chrome_visible(false, Some(flight), cx);
            self.set_picker_visible(false, flight, cx);
        }
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct RepostOpinionEditor {
    id: ElementId,
}

impl RepostOpinionEditor {
    const CARD_MAX_WIDTH: f32 = 460.0;

    const STICKER_AREA: f32 = 150.0;

    const CARD_RADIUS: f32 = 20.0;

    const CARD_BOTTOM_PADDING: f32 = 12.0;

    const POPOVER_GAP: f32 = 8.0;

    const POPOVER_MAX_WIDTH: f32 = 352.0;

    const POPOVER_RADIUS: f32 = 16.0;

    const POPOVER_MIN_HEIGHT: f32 = 220.0;

    const EDGE_INSET: f32 = 16.0;

    const PICKER_HEIGHT: f32 = 336.0;

    const TOGGLE_TRAILING: f32 = 4.0;
    const TOGGLE_WIDTH: f32 = 38.0;

    const SCRIM_OPACITY: f32 = 0.62;


    const STICKER_SLOT_HALF: f32 = 160.0;

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn caret(state: &Entity<RepostOpinionEditorState>, presenter: &Entity<RepostOpinionEditorPresenter>, cx: &App) -> Option<NSRange> {
        let state = state.read(cx);
        (state.selection_text == presenter.read(cx).text).then_some(state.selection)
    }

    /// places the caret without focusing the field
    /// focus would swap the emoji panel back for the keyboard
    fn place_caret(
        state: &Entity<RepostOpinionEditorState>,
        presenter: &Entity<RepostOpinionEditorPresenter>,
        range: Option<NSRange>,
        cx: &mut App,
    ) {
        let Some(range) = range else {
            return;
        };
        let text = presenter.read(cx).text.clone();
        state.update(cx, |state, cx| {
            state.selection = range;
            state.selection_text = text;
            state.requested_selection = Some(range);
            state.selection_token = state.selection_token.wrapping_add(1);
            cx.notify();
        });
    }

    fn close(cx: &mut App) {
        RepostOpinionEditorPresenter::shared(cx).update(cx, |presenter, cx| presenter.dismiss(cx));
    }

    /// escape takes the top layer only
    /// emoji popover first then the editor, never the panel or page under the scrim
    fn handle_escape(state: &Entity<RepostOpinionEditorState>, cx: &mut App) {
        if state.read(cx).picker_visible {
            state.update(cx, |state, cx| {
                state.focus_token = state.focus_token.wrapping_add(1);
                state.set_picker_visible(false, ease_out(0.24), cx);
            });
        } else {
            Self::close(cx);
        }
    }

    fn toggle_picker(state: &Entity<RepostOpinionEditorState>, cx: &mut App) {
        let _ = haptics::selection_changed();
        state.update(cx, |state, cx| {
            if state.picker_visible {
                state.focus_token = state.focus_token.wrapping_add(1);
                state.set_picker_visible(false, ease_out(0.24), cx);
            } else {
                state.set_picker_visible(true, ease_out(0.24), cx);
            }
        });
    }

    /// dialog card in container coordinates, left top width height
    /// compact card centred over the scrim, emoji panel is a popover and never moves it
    fn card_frame(container: Bounds<Pixels>, composer_height: f32) -> (f32, f32, f32, f32) {
        let width = f32::from(container.size.width);
        let height = f32::from(container.size.height);
        let card_width = Self::CARD_MAX_WIDTH.min((width - Self::EDGE_INSET * 2.0).max(0.0));
        let card_height = Self::STICKER_AREA + composer_height + Self::CARD_BOTTOM_PADDING;
        let top = ((height - card_height) / 2.0).max(Self::EDGE_INSET);
        ((width - card_width) / 2.0, top, card_width, card_height)
    }

    fn popover_frame(card: (f32, f32, f32, f32), container_height: f32) -> (f32, f32, f32, f32) {
        let (left, top, width, height) = card;
        let toggle_right = left + width
            - ChatBottomBarMetrics::H_PADDING
            - ChatBottomBarMetrics::CIRCLE
            - ChatBottomBarMetrics::ROW_GAP
            - Self::TOGGLE_TRAILING;
        let popover_width = Self::POPOVER_MAX_WIDTH.min(width);
        let popover_left = (toggle_right - popover_width).max(left);
        let room_below = container_height - (top + height + Self::POPOVER_GAP) - Self::EDGE_INSET;
        let room_above = top - Self::POPOVER_GAP - Self::EDGE_INSET;
        if room_below >= Self::POPOVER_MIN_HEIGHT.min(Self::PICKER_HEIGHT) || room_below >= room_above {
            let popover_height = Self::PICKER_HEIGHT.min(room_below).max(Self::POPOVER_MIN_HEIGHT.min(Self::PICKER_HEIGHT));
            (popover_left, top + height + Self::POPOVER_GAP, popover_width, popover_height)
        } else {
            let popover_height = Self::PICKER_HEIGHT.min(room_above);
            (popover_left, top - Self::POPOVER_GAP - popover_height, popover_width, popover_height)
        }
    }

    fn sticker(
        &self,
        presenter: &Entity<RepostOpinionEditorPresenter>,
        state: &Entity<RepostOpinionEditorState>,
        flight: f32,
        namespace: Option<RepostTransitionNamespace>,
        card: Option<(f32, f32, f32, f32)>,
        cx: &App,
    ) -> Option<AnyElement> {
        let editor = presenter.read(cx);
        let post = editor.post()?;
        if editor.session_id().is_empty() || !(editor.is_visible || editor.phase() == RepostOpinionTransitionPhase::Closing) {
            return None;
        }
        let container = state.read(cx).container_bounds?;
        let (card_left, card_top, card_width, _) = card?;
        let target_x = card_left + card_width / 2.0;
        let target_y = card_top + Self::STICKER_AREA / 2.0;

        let source_frame =
            RepostMatchedGeometry { id: editor.session_id().to_owned(), namespace, is_source: false }.source_frame();
        let (center_x, center_y, opacity) = match source_frame {
            Some(frame) => {
                let start_x = f32::from(frame.origin.x - container.origin.x) + f32::from(frame.size.width) / 2.0;
                let start_y = f32::from(frame.origin.y - container.origin.y) + f32::from(frame.size.height) / 2.0;
                (start_x + (target_x - start_x) * flight, start_y + (target_y - start_y) * flight, 1.0)
            }
            None => (target_x, target_y, flight.clamp(0.0, 1.0)),
        };

        let sticker = RepostSticker::new(self.child_id("sticker"), editor.text.clone(), editor.source_style())
            .custom_emojis(post.my_repost_custom_emojis())
            .live_custom_emoji_runs(editor.composer_custom_emoji_runs(&editor.text));
        let scale = 1.0 + (RepostSticker::EXPANDED_SCALE - 1.0) * flight;
        Some(
            pass_through()
                .absolute()
                .left(px(center_x - Self::STICKER_SLOT_HALF))
                .top(px(center_y - Self::STICKER_SLOT_HALF))
                .w(px(Self::STICKER_SLOT_HALF * 2.0))
                .h(px(Self::STICKER_SLOT_HALF * 2.0))
                .flex()
                .items_center()
                .justify_center()
                .opacity(opacity)
                .child(div().flex_none().occlude().child(sticker).scale_effect(scale))
                .into_any_element(),
        )
    }

    fn composer(
        &self,
        presenter: &Entity<RepostOpinionEditorPresenter>,
        state: &Entity<RepostOpinionEditorState>,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let (text_primary, text_secondary, bg_100) = {
            let theme = ThemeStore::shared(cx).read(cx).current_theme();
            (theme.text.primary, theme.text.secondary, theme.bg._100)
        };
        let (input_width, input_height, focus_token, requested_selection, selection_token, picker_visible) = {
            let state = state.read(cx);
            (
                state.input_width,
                state.input_height,
                state.focus_token,
                state.requested_selection,
                state.selection_token,
                state.picker_visible,
            )
        };
        let (text_is_empty, can_submit, custom_emoji_runs) = {
            let editor = presenter.read(cx);
            (editor.text.is_empty(), editor.can_submit(), editor.composer_custom_emoji_runs(&editor.text))
        };

        let selection_state = state.clone();
        let selection_presenter = presenter.clone();
        let runs_presenter = presenter.clone();
        let send_presenter = presenter.clone();
        let text_view = ChatComposerTextView::new(
            self.child_id("field"),
            Binding::entity(
                presenter.clone(),
                |presenter: &RepostOpinionEditorPresenter| presenter.text.clone(),
                |presenter, text, cx| {
                    presenter.text = text;
                    cx.notify();
                },
            ),
            input_width,
            ChatBottomBarMetrics::CIRCLE,
            ChatBottomBarMetrics::INPUT_MAX_HEIGHT,
            text_primary,
            RichEditorStyle::make(
                MessageSizes::composer_text_font_size(),
                text_primary,
                text_secondary.opacity(0.7),
                text_primary,
                text_primary.opacity(0.12),
            ),
            Binding::entity(
                state.clone(),
                |state: &RepostOpinionEditorState| state.input_height,
                |state, height, cx| {
                    state.input_height = height;
                    cx.notify();
                },
            ),
        )
        .custom_emoji_runs(custom_emoji_runs)
        .focus_token(focus_token)
        .requested_selection(requested_selection)
        .selection_token(selection_token)
        .on_selection_change(move |range, cx| {
            let text = selection_presenter.read(cx).text.clone();
            selection_state.update(cx, |state, cx| {
                state.selection = range;
                state.selection_text = text;
                cx.notify();
            });
        })
        .on_custom_emoji_runs_change(move |runs, cx| {
            let presenter = runs_presenter.clone();
            cx.defer(move |cx| presenter.update(cx, |presenter, cx| presenter.sync_custom_emoji_drafts(runs, cx)));
        })
        .on_send(move |_, cx| {
            send_presenter.update(cx, |presenter, cx| {
                if presenter.can_submit() {
                    presenter.save(cx);
                }
            });
        })
        .on_cancel({
            let cancel_state = state.clone();
            move |_, cx| Self::handle_escape(&cancel_state, cx)
        });

        let press_state = state.clone();
        let field = div()
            .id(self.child_id("field-press"))
            .capture_any_mouse_down(move |_: &MouseDownEvent, _, cx| {
                if press_state.read(cx).picker_visible {
                    press_state.update(cx, |state, cx| state.set_picker_visible(false, ease_out(0.24), cx));
                }
            })
            .child(text_view);

        let toggle_state = state.clone();
        let picker_toggle = div()
            .absolute()
            .right_0()
            .bottom_0()
            .pr(px(Self::TOGGLE_TRAILING))
            .block_mouse_except_scroll()
            .child(
                plain(self.child_id("picker-toggle")).on_press(move |_, cx| Self::toggle_picker(&toggle_state, cx)).child(
                    div()
                        .w(px(Self::TOGGLE_WIDTH))
                        .h(px(ChatBottomBarMetrics::CIRCLE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized(
                            "face.smiling",
                            px(21.0),
                            if picker_visible { text_primary } else { text_secondary },
                            false,
                        )),
                ),
            )
            .keyboard_dismiss_excluded();

        let field_content = div()
            .relative()
            .w(px(input_width))
            .h(px(input_height))
            .when(text_is_empty, |content| {
                content.child(
                    pass_through()
                        .absolute()
                        .left(px(16.0))
                        .top(px(10.0))
                        .text_size(px(15.0))
                        .text_color(text_secondary.opacity(0.82))
                        .child(SharedString::from(tr("repost_add_opinion"))),
                )
            })
            .child(field)
            .child(picker_toggle)
            .chat_composer_input_chrome(false);
        let field_content = div()
            .relative()
            .child(div().absolute().inset_0().rounded(px(ChatBottomBarMetrics::INPUT_CORNER)).bg(bg_100))
            .child(field_content);

        let width_state = state.clone();
        let input = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(input_height))
            .child(measure(self.child_id("input-geometry"), move |bounds, _, cx| {
                width_state.update(cx, |state, cx| {
                    state.input_width = f32::from(bounds.size.width);
                    cx.notify();
                });
            }))
            .child(field_content);

        let save_presenter = presenter.clone();
        hstack(ChatBottomBarMetrics::ROW_GAP)
            .align(VerticalAlignment::Bottom)
            .w_full()
            .child(input)
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(div().absolute().inset_0().rounded_full().bg(bg_100))
                    .child(
                        ComposerSendButton::new(self.child_id("send"), move |_, cx| {
                            save_presenter.update(cx, |presenter, cx| presenter.save(cx));
                        })
                        .size(ChatBottomBarMetrics::CIRCLE)
                        .is_enabled(can_submit),
                    ),
            )
            .padding_edges(EdgeSet::HORIZONTAL, ChatBottomBarMetrics::H_PADDING)
            .padding_edges(EdgeSet::TOP, ChatBottomBarMetrics::TOP_PADDING)
            .occlude()
            .keyboard_dismiss_excluded()
    }

    fn emoji_panel(
        &self,
        presenter: &Entity<RepostOpinionEditorPresenter>,
        state: &Entity<RepostOpinionEditorState>,
        is_active: bool,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let (flat_chrome, bg_100, stroke) = {
            let theme_store = ThemeStore::shared(cx);
            let theme_store = theme_store.read(cx);
            let theme = theme_store.current_theme();
            (theme_store.resolved_appearance() == RiseThemeAppearance::Light, theme.bg._100, theme.text.primary.opacity(0.12))
        };

        let (emoji_presenter, emoji_state) = (presenter.clone(), state.clone());
        let (custom_presenter, custom_state) = (presenter.clone(), state.clone());
        let (backspace_presenter, backspace_state) = (presenter.clone(), state.clone());
        let tab = repost_opinion_editor_emoji_tab(
            self.child_id("emoji-tab"),
            move |emoji: String, _: &mut Window, cx: &mut App| {
                let caret = Self::caret(&emoji_state, &emoji_presenter, cx);
                let placed = emoji_presenter.update(cx, |presenter, cx| presenter.insert_text(&emoji, caret, cx));
                Self::place_caret(&emoji_state, &emoji_presenter, placed, cx);
            },
            move |item: StickerItem, _: &mut Window, cx: &mut App| {
                let caret = Self::caret(&custom_state, &custom_presenter, cx);
                let placed = custom_presenter.update(cx, |presenter, cx| presenter.append_custom_emoji(item, caret, cx));
                Self::place_caret(&custom_state, &custom_presenter, placed, cx);
            },
            move |_: &mut Window, cx: &mut App| {
                let caret = Self::caret(&backspace_state, &backspace_presenter, cx);
                let placed = backspace_presenter.update(cx, |presenter, cx| presenter.delete_backward(caret, cx));
                Self::place_caret(&backspace_state, &backspace_presenter, placed, cx);
            },
            is_active,
            flat_chrome,
        );

        let background = if flat_chrome {
            div().absolute().inset_0().bg(bg_100).into_any_element()
        } else {
            div()
                .absolute()
                .inset_0()
                .bg(black())
                .child(HoldContextMenuGlass::new(self.child_id("emoji-glass")).tint(black().opacity(0.34)).opaque(true))
                .into_any_element()
        };

        div()
            .relative()
            .size_full()
            .occlude()
            .rounded(px(Self::POPOVER_RADIUS))
            .overflow_hidden()
            .shadow_lg()
            .border_1()
            .border_color(stroke)
            .child(background)
            .children(tab)
            .keyboard_dismiss_excluded()
    }

    fn run_editor_lifecycle(
        &self,
        session_id: String,
        presenter: Entity<RepostOpinionEditorPresenter>,
        state: Entity<RepostOpinionEditorState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        use_task(self.child_id("lifecycle"), session_id.clone(), window, cx, async move |cx| {
            if session_id.is_empty() {
                return;
            }
            let hidden_state = state.clone();
            let _ = cx.update(|_, cx| hidden_state.update(cx, |state, cx| state.set_chrome_visible(false, None, cx)));

            cx.background_executor().timer(Duration::from_millis(420)).await;
            let shown_state = state.clone();
            let shown_presenter = presenter.clone();
            let shown_session = session_id.clone();
            let shown = cx.update(|_, cx| {
                let editor = shown_presenter.read(cx);
                if !editor.is_visible || editor.session_id() != shown_session {
                    return false;
                }
                shown_state.update(cx, |state, cx| state.set_chrome_visible(true, Some(ease_out(0.2)), cx));
                true
            });
            if !matches!(shown, Ok(true)) {
                return;
            }

            cx.background_executor().timer(Duration::from_millis(160)).await;
            let _ = cx.update(|_, cx| {
                let editor = presenter.read(cx);
                if !editor.is_visible || editor.session_id() != session_id || state.read(cx).picker_visible {
                    return;
                }
                state.update(cx, |state, cx| {
                    state.focus_token = state.focus_token.wrapping_add(1);
                    cx.notify();
                });
            });
        });
    }
}

impl RenderOnce for RepostOpinionEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let presenter = RepostOpinionEditorPresenter::shared(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| RepostOpinionEditorState::new());
        let namespace = env::<RepostTransitionNamespaceKey>(cx);
        let (is_visible, session_id, text) = {
            let editor = presenter.read(cx);
            (editor.is_visible, editor.session_id().to_owned(), editor.text.clone())
        };

        self.run_editor_lifecycle(session_id.clone(), presenter.clone(), state.clone(), window, cx);

        let focus = window.use_keyed_state::<FocusHandle>(self.child_id("focus"), cx, |_, cx| cx.focus_handle());
        let focus = focus.read(cx).clone();

        let visibility_state = state.clone();
        let sheet_focus = focus.clone();
        on_change(self.child_id("is-visible"), is_visible, false, window, cx, move |_, visible, window, cx| {
            if *visible {
                ScreenFocus::register_sheet(&sheet_focus, cx);
            } else {
                resign_chat_keyboard(window, cx);
            }
            visibility_state.update(cx, |state, cx| state.presenter_visibility_changed(*visible, cx));
        });
        let limit_presenter = presenter.clone();
        on_change(self.child_id("text"), text, false, window, cx, move |_, _, _, cx| {
            limit_presenter.update(cx, |presenter, cx| presenter.limit_text(cx));
        });
        let landed_namespace = namespace.clone();
        on_change(self.child_id("session"), session_id, false, window, cx, move |_, session_id, _, _| {
            if session_id.is_empty()
                && let Some(namespace) = &landed_namespace
            {
                namespace.forget_frames();
            }
        });

        if is_visible {
            TopPresenter::note_presented(self.child_id("top-presenter"), window, cx);
        }

        let now = RiseClock::monotonic();
        let (flight, chrome_visible, chrome_reveal, picker_reveal, picker_visible, input_height) = {
            let state = state.read(cx);
            (
                state.flight.frame_value(now, window),
                state.chrome_visible,
                state.chrome_motion.frame_value(now, window),
                state.picker_motion.frame_value(now, window),
                state.picker_visible,
                state.input_height,
            )
        };

        let bounds_state = state.clone();
        let escape_state = state.clone();
        let mut root = div()
            .absolute()
            .inset_0()
            .when(is_visible, |root| {
                root.track_focus(&focus)
                    .layer_keys(LayerCancel::dismiss(move |_, cx| Self::handle_escape(&escape_state, cx)))
            })
            .child(measure(self.child_id("container"), move |bounds, _, cx| {
            bounds_state.update(cx, |state, cx| {
                state.container_bounds = Some(bounds);
                cx.notify();
            });
        }));

        if is_visible || flight > 0.0 {
            let scrim_color = black().opacity(Self::SCRIM_OPACITY * flight.clamp(0.0, 1.0));
            root = root.child(if is_visible {
                div()
                    .id(self.child_id("scrim"))
                    .absolute()
                    .inset_0()
                    .bg(scrim_color)
                    .occlude()
                    .on_click(|_, _, cx| Self::close(cx))
                    .into_any_element()
            } else {
                pass_through().absolute().inset_0().bg(scrim_color).into_any_element()
            });
        }

        let composer_height = input_height + ChatBottomBarMetrics::TOP_PADDING;
        let card = state.read(cx).container_bounds.map(|container| Self::card_frame(container, composer_height));
        let container_height = state.read(cx).container_bounds.map_or(0.0, |container| f32::from(container.size.height));

        if let Some((left, top, width, height)) = card
            && (chrome_visible || chrome_reveal > 0.0)
        {
            let card_color = ThemeStore::shared(cx).read(cx).current_theme().bg._200;
            root = root.child(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(width))
                    .h(px(height))
                    .rounded(px(Self::CARD_RADIUS))
                    .bg(card_color)
                    .opacity(chrome_reveal.clamp(0.0, 1.0))
                    .occlude(),
            );
        }

        root = root.children(self.sticker(&presenter, &state, flight, namespace, card, cx));

        if let Some((left, top, width, height)) = card {
            if chrome_visible || chrome_reveal > 0.0 {
                root = root.child(
                    div()
                        .absolute()
                        .left(px(left))
                        .top(px(top + Self::STICKER_AREA))
                        .w(px(width))
                        .opacity(chrome_reveal.clamp(0.0, 1.0))
                        .child(self.composer(&presenter, &state, cx)),
                );
            }
            if picker_reveal > 0.0 {
                let (panel_left, panel_top, panel_width, panel_height) =
                    Self::popover_frame((left, top, width, height), container_height);
                root = root.child(
                    div()
                        .absolute()
                        .left(px(panel_left))
                        .top(px(panel_top))
                        .w(px(panel_width))
                        .h(px(panel_height))
                        .opacity(picker_reveal.clamp(0.0, 1.0))
                        .child(self.emoji_panel(&presenter, &state, picker_visible, cx)),
                );
            }
        }

        root
    }
}
