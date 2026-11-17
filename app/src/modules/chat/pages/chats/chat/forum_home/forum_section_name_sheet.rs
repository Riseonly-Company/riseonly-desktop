use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyElement, App, AppContext, Context, ElementId, Entity, FontWeight, Hsla, MouseDownEvent, SharedString, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_measured_fraction::BottomSheetMeasuredFraction;
use rise_ui::button_ui::ButtonUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::main_text::MainText;
use rise_ui::rich_text::rich_text_markup::RichEditorStyle;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::formatted_text_ui::FormattedTextStyle;
use crate::core::ui::input_field_accessory::InputFieldAccessory;
use crate::modules::chat::components::chat::bubble::inline_custom_emoji_text::InlineCustomEmojiText;
use crate::modules::chat::components::chat::composer::chat_composer_text_view::ChatComposerTextView;
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatComposerCaretEdit, ChatComposerCaretEditResult, ChatComposerCustomEmojiRun, ChatDraftCustomEmoji,
};
use crate::modules::chat::components::chat::input_picker::chat_input_emoji_tab::ChatInputEmojiTab;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageCustomEmojiEntity, RemoteMediaURL};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{CustomEmojiEntityWire, MessageEntitiesCodec};
use crate::modules::comment::stores::comment_actions::comment_types::CommentEntities;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    StickerAssetType, StickerAssetTypeInfer, StickerItem,
};

const MINIMUM_FRACTION: f32 = 0.3;
const FALLBACK_PANEL_HEIGHT: f32 = 300.0;
const PLACEHOLDER: &str = "\u{FFFC}";
const INPUT_MIN_HEIGHT: f32 = 42.0;
const INPUT_MAX_HEIGHT: f32 = 92.0;

fn named_child(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

/// what the section name sheet edits, a new section when section_id is none
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForumSectionNameTarget {
    pub chat_id: String,
    pub section_id: Option<String>,
    pub name: String,
    pub name_entities: Option<String>,
}

impl ForumSectionNameTarget {
    pub fn id(&self) -> String {
        format!("{}\u{1F}{}", self.chat_id, self.section_id.as_deref().unwrap_or(""))
    }

    pub fn is_editing(&self) -> bool {
        self.section_id.is_some()
    }
}

/// section header name in small caps like uppercase, custom emoji drawn from name_entities
#[derive(IntoElement)]
pub struct ForumSectionTitleText {
    name: String,
    entities: Vec<MessageCustomEmojiEntity>,
    px: f32,
    color: Hsla,
}

impl ForumSectionTitleText {
    pub fn new(name: String, entities: Vec<MessageCustomEmojiEntity>, color: Hsla) -> Self {
        Self { name, entities, px: 12.0, color }
    }

    pub fn px(mut self, px: f32) -> Self {
        self.px = px;
        self
    }

    fn uppercased_keeping_offsets(text: &str) -> String {
        let upper = text.to_uppercase();
        if upper.encode_utf16().count() == text.encode_utf16().count() { upper } else { text.to_owned() }
    }
}

impl RenderOnce for ForumSectionTitleText {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if self.entities.is_empty() {
            return MainText::new(self.name.to_uppercase())
                .px(self.px)
                .font_weight(FontWeight::SEMIBOLD)
                .color(self.color)
                .number_of_lines(1)
                .into_any_element();
        }
        let mut style = FormattedTextStyle::new(self.color, FormattedTextStyle::standard_link_color(), self.color);
        style.font_size = (self.px + ThemeStore::text_delta_snapshot()).max(1.0);
        let id = ElementId::Name(SharedString::from(format!("forum-section-title-{}", self.name)));
        div()
            .font_weight(FontWeight::SEMIBOLD)
            .line_clamp(1)
            .text_ellipsis()
            .child(InlineCustomEmojiText::new(id, Self::uppercased_keeping_offsets(&self.name), style, self.entities))
            .into_any_element()
    }
}

/// one sheet for creating and renaming a forum section, shown while target is set
#[derive(IntoElement)]
pub struct ForumSectionNameSheetHost {
    id: ElementId,
    target: Binding<Option<ForumSectionNameTarget>>,
}

impl ForumSectionNameSheetHost {
    pub fn new(id: impl Into<ElementId>, target: Binding<Option<ForumSectionNameTarget>>) -> Self {
        Self { id: id.into(), target }
    }
}

struct ForumSectionNameSheetHostState {
    shown: Option<ForumSectionNameTarget>,
    sheet_fraction: f32,
}

impl RenderOnce for ForumSectionNameSheetHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let host = window.use_keyed_state(named_child(&self.id, "host"), cx, |_, _| ForumSectionNameSheetHostState {
            shown: None,
            sheet_fraction: MINIMUM_FRACTION,
        });
        let adopting = host.clone();
        on_change(
            named_child(&self.id, "target"),
            self.target.get(cx),
            false,
            window,
            cx,
            move |_, next: &Option<ForumSectionNameTarget>, _, cx| {
                if let Some(next) = next.clone() {
                    adopting.update(cx, |host, cx| {
                        host.shown = Some(next);
                        cx.notify();
                    });
                }
            },
        );

        let (reader, writer) = (self.target.clone(), self.target.clone());
        let visible = Binding::new(
            move |cx: &App| reader.get(cx).is_some(),
            move |value: bool, window: &mut Window, cx: &mut App| {
                if !value {
                    writer.set(None, window, cx);
                }
            },
        );
        let sheet_fraction = host.read(cx).sheet_fraction;
        let (id, target, content_host) = (self.id.clone(), self.target.clone(), host.clone());

        BottomSheetUi::new(
            named_child(&self.id, "sheet"),
            visible,
            move |_, cx| {
                let current = target.get(cx).or_else(|| content_host.read(cx).shown.clone());
                let Some(current) = current else {
                    return div().into_any_element();
                };
                let closing = target.clone();
                let measuring = content_host.clone();
                let sheet_id = named_child(&id, current.id());
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .justify_start()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .flex_none()
                            .child(ForumSectionNameSheet::new(sheet_id.clone(), current, move |window, cx| {
                                closing.set(None, window, cx);
                            }))
                            .child(measure(named_child(&sheet_id, "height"), move |bounds, window, cx| {
                                let measured = BottomSheetMeasuredFraction::value(
                                    f32::from(bounds.size.height),
                                    MINIMUM_FRACTION,
                                    window,
                                );
                                measuring.update(cx, |host, cx| {
                                    if (measured - host.sheet_fraction).abs() > 0.005 {
                                        host.sheet_fraction = measured;
                                        cx.notify();
                                    }
                                });
                            })),
                    )
                    .into_any_element()
            },
            |_, _| div(),
        )
        .snap_fractions(vec![sheet_fraction])
        .fits_content(true)
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .footer_shows_top_border(false)
        .sheet_background(Some(theme.bg._100))
    }
}

struct ForumSectionNameSheetState {
    name: String,
    drafts: Vec<ChatDraftCustomEmoji>,
    emoji_panel_visible: bool,
    did_submit: bool,
    input_height: f32,
    input_width: f32,
    selection: Option<NSRange>,
    focus_token: i64,
    requested_selection: Option<NSRange>,
    selection_token: i64,
    baseline_title: String,
    baseline_entities: Option<String>,
    emoji_panel: Entity<Presence<()>>,
}

impl ForumSectionNameSheetState {
    fn new(target: &ForumSectionNameTarget, cx: &mut Context<Self>) -> Self {
        let (baseline_title, baseline_entities) = ChatFolder::dropping_orphan_object_marks(
            &target.name,
            CommentEntities::normalized(target.name_entities.as_deref()),
        );
        let drafts = Self::drafts(&CommentEntities::normalized(baseline_entities.as_deref()).1, &baseline_title);
        Self {
            name: baseline_title.clone(),
            drafts,
            emoji_panel_visible: false,
            did_submit: false,
            input_height: INPUT_MIN_HEIGHT,
            input_width: 0.0,
            selection: None,
            focus_token: 0,
            requested_selection: None,
            selection_token: 0,
            baseline_title,
            baseline_entities,
            emoji_panel: cx.new(|_| {
                Presence::new(None).with_transition(Transition::move_edge(Edge::Bottom).combined(Transition::opacity()))
            }),
        }
    }

    fn trimmed_name(&self) -> String {
        trimming_whitespaces_and_newlines(&self.name).to_owned()
    }

    fn can_submit(&self, rule: &RiseFieldRule, is_editing: bool) -> bool {
        let trimmed = self.trimmed_name();
        if self.did_submit || trimmed.is_empty() || !rule.is_satisfied(&trimmed) {
            return false;
        }
        if !is_editing {
            return true;
        }
        trimmed != self.baseline_title
            || self.entities_json(&trimmed).unwrap_or_default()
                != CommentEntities::normalized(self.baseline_entities.as_deref()).0.unwrap_or_default()
    }

    fn close_emoji_panel(&mut self, focus_field: bool, cx: &mut Context<Self>) {
        if !self.emoji_panel_visible {
            return;
        }
        self.emoji_panel_visible = false;
        if focus_field {
            self.focus_token = self.focus_token.wrapping_add(1);
        }
        cx.notify();
    }

    fn insert_emoji(&mut self, emoji: &str, rule: &RiseFieldRule, cx: &mut Context<Self>) -> Option<NSRange> {
        if emoji.is_empty() || rule.clamped(&format!("{}{}", self.name, emoji)) == self.name {
            return None;
        }
        let edit = ChatComposerCaretEdit::replace(&self.name, self.selection, emoji, &mut self.drafts, None);
        Some(self.apply_edit(edit, cx))
    }

    fn insert_custom_emoji(&mut self, item: StickerItem, rule: &RiseFieldRule, cx: &mut Context<Self>) -> Option<NSRange> {
        if item.id.is_empty()
            || item.file_url.is_empty()
            || rule.clamped(&format!("{}{}", self.name, PLACEHOLDER)) == self.name
        {
            return None;
        }
        let edit = ChatComposerCaretEdit::replace(
            &self.name,
            self.selection,
            PLACEHOLDER,
            &mut self.drafts,
            Some(ChatDraftCustomEmoji { placeholder: PLACEHOLDER.to_owned(), item }),
        );
        Some(self.apply_edit(edit, cx))
    }

    fn delete_backward(&mut self, cx: &mut Context<Self>) -> Option<NSRange> {
        let edit = ChatComposerCaretEdit::delete_backward(&self.name, self.selection, &mut self.drafts)?;
        Some(self.apply_edit(edit, cx))
    }

    fn apply_edit(&mut self, edit: ChatComposerCaretEditResult, cx: &mut Context<Self>) -> NSRange {
        self.name = edit.text;
        cx.notify();
        edit.caret
    }

    fn apply_caret(&mut self, caret: Option<NSRange>, cx: &mut Context<Self>) {
        let Some(caret) = caret else {
            return;
        };
        self.selection = Some(caret);
        self.requested_selection = Some(caret);
        self.selection_token = self.selection_token.wrapping_add(1);
        cx.notify();
    }

    fn sync_drafts(&mut self, runs: Vec<ChatComposerCustomEmojiRun>, cx: &mut Context<Self>) {
        let unchanged = runs.len() == self.drafts.len()
            && runs.iter().zip(&self.drafts).all(|(run, draft)| run.item == draft.item);
        if unchanged {
            return;
        }
        self.drafts = runs
            .into_iter()
            .map(|run| ChatDraftCustomEmoji { placeholder: PLACEHOLDER.to_owned(), item: run.item })
            .collect();
        cx.notify();
    }

    /// canon name_entities for the saved name, custom emoji only with offsets from the trimmed name
    fn entities_json(&self, trimmed: &str) -> Option<String> {
        let wires: Vec<CustomEmojiEntityWire> = ChatComposerCaretEdit::custom_emoji_runs(trimmed, &self.drafts)
            .into_iter()
            .map(|run| CustomEmojiEntityWire {
                offset: run.range.location as i64,
                length: run.range.length as i64,
                custom_emoji_id: run.item.id,
                file_url: RemoteMediaURL::optional_normalized(Some(run.item.file_url.as_str()))
                    .unwrap_or(run.item.file_url),
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

    fn drafts(entities: &[MessageCustomEmojiEntity], text: &str) -> Vec<ChatDraftCustomEmoji> {
        if entities.is_empty() || !text.contains(PLACEHOLDER) {
            return Vec::new();
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut owned: Vec<&MessageCustomEmojiEntity> = entities
            .iter()
            .filter(|entity| {
                entity.length == 1
                    && usize::try_from(entity.offset).ok().and_then(|offset| units.get(offset)) == Some(&0xFFFC)
            })
            .collect();
        owned.sort_by_key(|entity| entity.offset);
        owned
            .into_iter()
            .map(|entity| ChatDraftCustomEmoji {
                placeholder: PLACEHOLDER.to_owned(),
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

#[derive(IntoElement)]
struct ForumSectionNameSheet {
    id: ElementId,
    target: ForumSectionNameTarget,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ForumSectionNameSheet {
    fn new(id: ElementId, target: ForumSectionNameTarget, on_close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id, target, on_close: Rc::new(on_close) }
    }
}

impl RenderOnce for ForumSectionNameSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let target = self.target.clone();
        let state = window.use_keyed_state(named_child(&self.id, "state"), cx, move |_, cx| {
            ForumSectionNameSheetState::new(&target, cx)
        });
        let parts = ForumSectionNameSheetParts {
            id: self.id.clone(),
            target: self.target,
            state: state.clone(),
            on_close: self.on_close,
        };
        let appearing = state.clone();
        on_change(named_child(&self.id, "appear"), (), true, window, cx, move |_, _, _, cx| {
            appearing.update(cx, |state, cx| {
                state.focus_token = state.focus_token.wrapping_add(1);
                cx.notify();
            });
        });
        parts.body(window, cx)
    }
}

#[derive(Clone)]
struct ForumSectionNameSheetParts {
    id: ElementId,
    target: ForumSectionNameTarget,
    state: Entity<ForumSectionNameSheetState>,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ForumSectionNameSheetParts {
    fn child_id(&self, name: &'static str) -> ElementId {
        named_child(&self.id, name)
    }

    fn rule(&self) -> RiseFieldRule {
        if self.target.is_editing() {
            RiseFieldRule::new("chat_update_section", "/name")
        } else {
            RiseFieldRule::new("chat_create_section", "/name")
        }
    }

    fn can_submit(&self, cx: &App) -> bool {
        self.state.read(cx).can_submit(&self.rule(), self.target.is_editing())
    }

    fn body(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let is_editing = self.target.is_editing();
        let (can_submit, did_submit) = (self.can_submit(cx), self.state.read(cx).did_submit);

        let title_state = self.state.clone();
        let title = div()
            .id(self.child_id("title"))
            .w_full()
            .pt(px(4.0))
            .flex()
            .justify_center()
            .on_click(move |_, _, cx| title_state.update(cx, |state, cx| state.close_emoji_panel(false, cx)))
            .child(
                MainText::new(if is_editing { tr("forum_section_rename") } else { tr("forum_section_create_title") })
                    .px(18.0)
                    .font_weight(FontWeight::BOLD)
                    .color(theme.text.primary),
            );

        let card = GroupedBtns::new(
            self.child_id("card"),
            vec![
                GroupedBtn::new("name", tr("forum_section_name"))
                    .show_chevron(false)
                    .content_slot(self.name_row(&theme, cx)),
            ],
        );

        let now = RiseClock::monotonic();
        let (visible, presence) = {
            let state = self.state.read(cx);
            (state.emoji_panel_visible, state.emoji_panel.clone())
        };
        let spec = if visible { ease_out(0.22) } else { ease_out(0.2) };
        presence.update(cx, |presence, cx| presence.set_in(visible.then_some(()), spec, now, cx));
        let panel = presence.read(cx).value(now).is_some().then(|| self.emoji_panel());
        let panel = presence.read(cx).render_transitioned(now, window, move |_| {
            div().w_full().px(px(GroupedCardMetrics::HORIZONTAL_PADDING)).children(panel)
        });

        let submitting = self.clone();
        let button = div().w_full().px(px(12.0)).pb(px(6.0)).child(
            ButtonUi::new(self.child_id("submit"))
                .disabled(!can_submit)
                .is_pending(did_submit)
                .on_press(move |window, cx| submitting.submit(window, cx))
                .child(MainText::new(if is_editing { tr("common_save") } else { tr("forum_section_create") })),
        );

        vstack(16.0)
            .w_full()
            .bg(theme.bg._100)
            .child(title)
            .child(card)
            .children(panel)
            .child(button)
            .into_any_element()
    }

    fn emoji_panel(&self) -> AnyElement {
        let (emoji_state, emoji_rule) = (self.state.clone(), self.rule());
        let (custom_state, custom_rule) = (self.state.clone(), self.rule());
        let backspace_state = self.state.clone();
        div()
            .w_full()
            .h(px(FALLBACK_PANEL_HEIGHT))
            .rounded(px(GroupedCardMetrics::CORNER_RADIUS))
            .overflow_hidden()
            .child(
                ChatInputEmojiTab::new(
                    self.child_id("emoji-tab"),
                    move |emoji, _, cx| {
                        emoji_state.update(cx, |state, cx| {
                            let caret = state.insert_emoji(&emoji, &emoji_rule, cx);
                            state.apply_caret(caret, cx);
                        });
                    },
                    move |sticker, _, cx| {
                        custom_state.update(cx, |state, cx| {
                            let caret = state.insert_custom_emoji(sticker, &custom_rule, cx);
                            state.apply_caret(caret, cx);
                        });
                    },
                    move |_, cx| {
                        backspace_state.update(cx, |state, cx| {
                            let caret = state.delete_backward(cx);
                            state.apply_caret(caret, cx);
                        });
                    },
                )
                .safe_area_bottom(0.0)
                .uses_flat_chrome(true),
            )
            .into_any_element()
    }

    fn name_row(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let rule = self.rule();
        let (name, visible, input_height, input_width, focus_token, requested_selection, selection_token, runs) = {
            let state = self.state.read(cx);
            (
                state.name.clone(),
                state.emoji_panel_visible,
                state.input_height,
                state.input_width,
                state.focus_token,
                state.requested_selection,
                state.selection_token,
                ChatComposerCaretEdit::custom_emoji_runs(&state.name, &state.drafts),
            )
        };
        let font_size = MessageSizes::composer_text_font_size();

        let ruled = rule.binding(Binding::entity(
            self.state.clone(),
            |state: &ForumSectionNameSheetState| state.name.clone(),
            |state, name, cx| {
                state.name = name;
                cx.notify();
            },
        ));
        let (reading, writing) = (ruled.clone(), ruled);
        let submitting = self.clone();
        let name_binding = Binding::new(
            move |cx: &App| reading.get(cx),
            move |next: String, window: &mut Window, cx: &mut App| {
                if !next.contains('\n') {
                    writing.set(next, window, cx);
                    return;
                }
                writing.set(next.replace('\n', ""), window, cx);
                let parts = submitting.clone();
                window.defer(cx, move |window, cx| parts.submit(window, cx));
            },
        );

        let selection_state = self.state.clone();
        let runs_state = self.state.clone();
        let send_parts = self.clone();
        let field = ChatComposerTextView::new(
            self.child_id("name-field"),
            name_binding,
            input_width,
            INPUT_MIN_HEIGHT,
            INPUT_MAX_HEIGHT,
            theme.text.primary,
            RichEditorStyle::make(
                font_size,
                theme.text.primary,
                theme.text.secondary.opacity(0.7),
                theme.text.primary,
                theme.text.primary.opacity(0.12),
            ),
            Binding::entity(
                self.state.clone(),
                |state: &ForumSectionNameSheetState| state.input_height,
                |state, height, cx| {
                    state.input_height = height;
                    cx.notify();
                },
            ),
        )
        .custom_emoji_runs(runs)
        .focus_token(focus_token)
        .requested_selection(requested_selection)
        .selection_token(selection_token)
        .on_selection_change(move |range, cx| {
            selection_state.update(cx, |state, _| state.selection = Some(range));
        })
        .on_custom_emoji_runs_change(move |runs, cx| {
            let state = runs_state.clone();
            cx.defer(move |cx| state.update(cx, |state, cx| state.sync_drafts(runs, cx)));
        })
        .holds_custom_emoji_line_height(true)
        .on_send(move |window, cx| send_parts.submit(window, cx));

        let press_state = self.state.clone();
        let field = div()
            .id(self.child_id("name-field-press"))
            .capture_any_mouse_down(move |_: &MouseDownEvent, _, cx| {
                press_state.update(cx, |state, cx| state.close_emoji_panel(true, cx));
            })
            .child(field);

        let placeholder_top = 10.0 + ChatComposerTextView::held_line_extra(font_size, cx);
        let placeholder = name.is_empty().then(|| {
            pass_through()
                .absolute()
                .top(px(placeholder_top))
                .left(px(5.0))
                .text_size(px(font_size))
                .text_color(theme.text.secondary.opacity(0.82))
                .line_clamp(1)
                .child(SharedString::from(tr("forum_section_name")))
        });

        let width_state = self.state.clone();
        let input = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(input_height))
            .child(measure(self.child_id("name-geometry"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                width_state.update(cx, |state, cx| {
                    if (state.input_width - width).abs() > 0.5 {
                        state.input_width = width;
                        cx.notify();
                    }
                });
            }))
            .children(placeholder)
            .child(div().absolute().top_0().left_0().w(px(input_width)).h(px(input_height)).child(field));

        let toggling = self.clone();
        let emoji_button = plain(self.child_id("emoji-toggle"))
            .w(px(44.0))
            .h(px(44.0))
            .flex()
            .items_center()
            .justify_center()
            .on_press(move |window, cx| toggling.toggle_emoji_panel(window, cx))
            .children(IconUi::sized(
                if visible { "keyboard" } else { "face.smiling" },
                px(19.0),
                theme.text.secondary,
                false,
            ));

        hstack(4.0)
            .w_full()
            .items_center()
            .pl(px(11.0))
            .pr(px(6.0))
            .py(px(6.0))
            .min_h(px(54.0))
            .rounded(px(GroupedCardMetrics::CORNER_RADIUS))
            .bg(theme.bg._200)
            .child(input)
            .child(InputFieldAccessory::new(self.child_id("accessory"), rule.remaining(&name), None))
            .child(emoji_button)
            .into_any_element()
    }

    fn toggle_emoji_panel(&self, window: &mut Window, cx: &mut App) {
        let _ = selection_changed();
        let visible = self.state.read(cx).emoji_panel_visible;
        if !visible {
            ScreenFocus::release_text_field(window, cx);
        }
        self.state.update(cx, |state, cx| {
            state.emoji_panel_visible = !visible;
            if visible {
                state.focus_token = state.focus_token.wrapping_add(1);
            }
            cx.notify();
        });
    }

    fn submit(&self, window: &mut Window, cx: &mut App) {
        if !self.can_submit(cx) {
            return;
        }
        let (trimmed, entities) = self.state.update(cx, |state, cx| {
            state.did_submit = true;
            cx.notify();
            let trimmed = state.trimmed_name();
            let entities = state.entities_json(&trimmed);
            (trimmed, entities)
        });
        let window_handle = window.window_handle();
        let (finishing, on_close) = (self.state.downgrade(), self.on_close.clone());
        let finish: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |saved: bool, cx: &mut App| {
            let _ = finishing.update(cx, |state, cx| {
                state.did_submit = false;
                cx.notify();
            });
            if saved {
                let on_close = on_close.clone();
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, window, cx| on_close(window, cx));
                });
            }
        });
        let chat_id = self.target.chat_id.clone();
        let section_id = self.target.section_id.clone();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| match section_id {
            Some(section_id) => {
                store.rename_section(&chat_id, &section_id, &trimmed, entities.as_deref(), Some(finish), cx)
            }
            None => store.create_section(&chat_id, &trimmed, entities.as_deref(), Some(finish), cx),
        });
    }
}
