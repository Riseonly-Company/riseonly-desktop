use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, IntoElement, MouseDownEvent, RenderOnce, SharedString, Window, black, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_platform::haptics;
use rise_theme::{RiseThemeAppearance, ThemeStore};
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::HoldContextMenuGlass;
use rise_ui::icon_ui::IconUi;
use rise_ui::keyboard_dismiss::{KeyboardDismiss, KeyboardDismissModifiers};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::rich_text::rich_text_markup::RichEditorStyle;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::unread_count_badge_ui::UnreadCountBadgeUi;

use crate::modules::chat::components::chat::composer::chat_composer_text_view::ChatComposerTextView;
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatBottomBarMetrics, ChatComposerCaretEdit, ChatComposerCaretEditResult, ChatDraftCustomEmoji,
};
use crate::modules::chat::components::chat::input_picker::chat_input_emoji_tab::ChatInputEmojiTab;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

enum CaptionBarMetrics {}

impl CaptionBarMetrics {
    const CIRCLE: f32 = ChatBottomBarMetrics::CIRCLE;
    const H_PADDING: f32 = ChatBottomBarMetrics::H_PADDING;
    const TOP_PADDING: f32 = ChatBottomBarMetrics::TOP_PADDING;
    const INPUT_CORNER: f32 = ChatBottomBarMetrics::INPUT_CORNER;
    const INPUT_MAX_HEIGHT: f32 = ChatBottomBarMetrics::INPUT_MAX_HEIGHT;
    const ROW_GAP: f32 = ChatBottomBarMetrics::ROW_GAP;
}

const OBJECT_REPLACEMENT: &str = "\u{FFFC}";

const PICKER_PANEL_HEIGHT: f32 = 336.0;

struct MediaPickerCaptionBarState {
    composer_input_height: f32,
    picker_visible: bool,
    composer_focus_token: i64,
    composer_selection: Option<NSRange>,
    composer_selection_text: String,
    composer_requested_selection: Option<NSRange>,
    composer_selection_token: i64,
    input_width: f32,
    picker_motion: Animated<f32>,
}

impl MediaPickerCaptionBarState {
    fn new() -> Self {
        Self {
            composer_input_height: CaptionBarMetrics::CIRCLE,
            picker_visible: false,
            composer_focus_token: 0,
            composer_selection: None,
            composer_selection_text: String::new(),
            composer_requested_selection: None,
            composer_selection_token: 0,
            input_width: 0.0,
            picker_motion: Animated::new(0.0),
        }
    }

    fn set_picker_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.picker_visible = visible;
        self.picker_motion.set_in(
            if visible { 1.0 } else { 0.0 },
            ease_out(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION),
            RiseClock::monotonic(),
            cx,
        );
        cx.notify();
    }
}

#[derive(IntoElement, Clone)]
pub struct MediaPickerCaptionBar {
    id: ElementId,
    selection: Entity<MediaSelectionState>,
    theme_store: Entity<ThemeStore>,
    caption: Binding<String>,
    caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
    on_send: Rc<dyn Fn(&mut Window, &mut App)>,
    bottom_safe_inset: f32,
    on_picker_visible_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
    send_enabled: Option<bool>,
}

impl MediaPickerCaptionBar {
    /// defaults are caption custom emojis none, bottom safe inset 0
    /// on_picker_visible_change and send_enabled none
    pub fn new(
        id: impl Into<ElementId>,
        selection: Entity<MediaSelectionState>,
        theme_store: Entity<ThemeStore>,
        caption: Binding<String>,
        on_send: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            selection,
            theme_store,
            caption,
            caption_custom_emojis: None,
            on_send: Rc::new(on_send),
            bottom_safe_inset: 0.0,
            on_picker_visible_change: None,
            send_enabled: None,
        }
    }

    pub fn caption_custom_emojis(mut self, caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>) -> Self {
        self.caption_custom_emojis = caption_custom_emojis;
        self
    }

    pub fn bottom_safe_inset(mut self, bottom_safe_inset: f32) -> Self {
        self.bottom_safe_inset = bottom_safe_inset;
        self
    }

    pub fn on_picker_visible_change(mut self, on_picker_visible_change: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_picker_visible_change = Some(Rc::new(on_picker_visible_change));
        self
    }

    pub fn send_enabled(mut self, send_enabled: impl Into<Option<bool>>) -> Self {
        self.send_enabled = send_enabled.into();
        self
    }

    fn send_button(&self, can_send: bool, cx: &App) -> impl IntoElement + use<> {
        let theme = self.theme_store.read(cx).current_theme();
        let accent = theme.primary._100;
        let fill = if can_send { accent } else { theme.bg._300 };
        let glyph = theme.content_on(fill);
        let count = self.selection.read(cx).count();
        let send_action = self.on_send.clone();
        div()
            .relative()
            .flex_none()
            .block_mouse_except_scroll()
            .child(
                plain(self.child_id("send"))
                    .on_press(move |window, cx| {
                        if can_send {
                            send_action(window, cx);
                        }
                    })
                    .child(
                        div()
                            .w(px(CaptionBarMetrics::CIRCLE))
                            .h(px(CaptionBarMetrics::CIRCLE))
                            .rounded_full()
                            .bg(fill)
                            .flex()
                            .items_center()
                            .justify_center()
                            .pl(px(2.0))
                            .children(IconUi::sized("paperplane.fill", px(17.0), glyph, true)),
                    ),
            )
            .child(div().absolute().top(px(-4.0)).right(px(-4.0)).child(UnreadCountBadgeUi::new(count).fill(accent)))
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn picker_toggle(&self, state: &Entity<MediaPickerCaptionBarState>, cx: &App) -> impl IntoElement + use<> {
        let picker_visible = state.read(cx).picker_visible;
        let secondary = self.theme_store.read(cx).current_theme().text.secondary;
        let bar = self.clone();
        let toggled = state.clone();
        div()
            .pr(px(4.0))
            .block_mouse_except_scroll()
            .child(
                plain(self.child_id("picker-toggle"))
                    .on_press(move |_, cx| bar.toggle_picker(&toggled, cx))
                    .child(
                        div()
                            .w(px(38.0))
                            .h(px(CaptionBarMetrics::CIRCLE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized(
                                if picker_visible { "keyboard" } else { "face.smiling" },
                                px(21.0),
                                secondary,
                                false,
                            )),
                    ),
            )
            .keyboard_dismiss_excluded()
    }

    fn emoji_panel(&self, state: &Entity<MediaPickerCaptionBarState>, cx: &App) -> impl IntoElement + use<> {
        let (flat_chrome, bg_100) = {
            let theme_store = self.theme_store.read(cx);
            (theme_store.resolved_appearance() == RiseThemeAppearance::Light, theme_store.current_theme().bg._100)
        };
        const PANEL_RADIUS: f32 = 28.0;

        let (emoji_bar, emoji_state) = (self.clone(), state.clone());
        let (custom_bar, custom_state) = (self.clone(), state.clone());
        let (backspace_bar, backspace_state) = (self.clone(), state.clone());
        let tab = ChatInputEmojiTab::new(
            self.child_id("emoji-tab"),
            move |emoji: String, window: &mut Window, cx: &mut App| {
                emoji_bar.insert_emoji(&emoji, &emoji_state, window, cx);
            },
            move |item: StickerItem, window: &mut Window, cx: &mut App| {
                custom_bar.insert_custom_emoji(item, &custom_state, window, cx);
            },
            move |window: &mut Window, cx: &mut App| {
                backspace_bar.backspace(&backspace_state, window, cx);
            },
        )
        .safe_area_bottom(0.0)
        .is_active(true)
        .reserves_picker_bottom_bar(false)
        .uses_flat_chrome(flat_chrome);

        let background = if flat_chrome {
            div().absolute().inset_0().bg(bg_100).into_any_element()
        } else {
            div()
                .absolute()
                .inset_0()
                .child(HoldContextMenuGlass::new(self.child_id("emoji-glass")).tint(black().opacity(0.30)))
                .into_any_element()
        };

        let panel = div()
            .absolute()
            .inset_0()
            .rounded_tl(px(PANEL_RADIUS))
            .rounded_tr(px(PANEL_RADIUS))
            .overflow_hidden()
            .child(background)
            .child(tab);

        let stroke = (!flat_chrome).then(|| {
            let half = 0.8 / 2.0;
            pass_through()
                .absolute()
                .top(px(-half))
                .left(px(-half))
                .right(px(-half))
                .bottom(px(-half))
                .rounded_tl(px(PANEL_RADIUS + half))
                .rounded_tr(px(PANEL_RADIUS + half))
                .border(px(0.8))
                .border_color(white().opacity(0.16))
        });

        div().relative().size_full().child(panel).children(stroke)
    }

    fn toggle_picker(&self, state: &Entity<MediaPickerCaptionBarState>, cx: &mut App) {
        haptics::selection_changed();

        let picker_visible = state.read(cx).picker_visible;
        state.update(cx, |state, cx| {
            if picker_visible {
                state.composer_focus_token += 1;
                state.set_picker_visible(false, cx);
            } else {
                state.set_picker_visible(true, cx);
            }
        });
        if !picker_visible {
            KeyboardDismiss::perform(cx);
        }
    }

    fn insert_emoji(&self, emoji: &str, state: &Entity<MediaPickerCaptionBarState>, window: &mut Window, cx: &mut App) {
        self.replace_caption(emoji, None, state, window, cx);
    }

    fn insert_custom_emoji(
        &self,
        item: StickerItem,
        state: &Entity<MediaPickerCaptionBarState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.caption_custom_emojis.is_none() {
            let emoji = item.associated_emojis.first().cloned().unwrap_or_default();
            self.insert_emoji(&emoji, state, window, cx);
            return;
        }
        self.replace_caption(
            OBJECT_REPLACEMENT,
            Some(ChatDraftCustomEmoji { placeholder: OBJECT_REPLACEMENT.to_string(), item }),
            state,
            window,
            cx,
        );
    }

    fn backspace(&self, state: &Entity<MediaPickerCaptionBarState>, window: &mut Window, cx: &mut App) {
        let mut drafts = self.drafts(cx);
        let caption = self.caption.get(cx);
        let caret = self.composer_caret(&caption, state, cx);
        let Some(edit) = ChatComposerCaretEdit::delete_backward(&caption, caret, &mut drafts) else {
            return;
        };
        if let Some(caption_custom_emojis) = &self.caption_custom_emojis {
            caption_custom_emojis.set(drafts, window, cx);
        }
        self.apply_caption_edit(edit, state, window, cx);
    }

    fn drafts(&self, cx: &App) -> Vec<ChatDraftCustomEmoji> {
        self.caption_custom_emojis.as_ref().map(|drafts| drafts.get(cx)).unwrap_or_default()
    }

    fn composer_caret(&self, caption: &str, state: &Entity<MediaPickerCaptionBarState>, cx: &App) -> Option<NSRange> {
        let state = state.read(cx);
        if state.composer_selection_text == caption { state.composer_selection } else { None }
    }

    fn replace_caption(
        &self,
        text: &str,
        inserting: Option<ChatDraftCustomEmoji>,
        state: &Entity<MediaPickerCaptionBarState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut drafts = self.drafts(cx);
        let caption = self.caption.get(cx);
        let caret = self.composer_caret(&caption, state, cx);
        let edit = ChatComposerCaretEdit::replace(&caption, caret, text, &mut drafts, inserting);
        if let Some(caption_custom_emojis) = &self.caption_custom_emojis {
            caption_custom_emojis.set(drafts, window, cx);
        }
        self.apply_caption_edit(edit, state, window, cx);
    }

    fn apply_caption_edit(
        &self,
        edit: ChatComposerCaretEditResult,
        state: &Entity<MediaPickerCaptionBarState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.caption.set(edit.text.clone(), window, cx);
        state.update(cx, |state, cx| {
            state.composer_selection = Some(edit.caret);
            state.composer_selection_text = edit.text;
            state.composer_requested_selection = Some(edit.caret);
            state.composer_selection_token = state.composer_selection_token.wrapping_add(1);
            cx.notify();
        });
    }

    fn composer_text_view(&self, state: &Entity<MediaPickerCaptionBarState>, window: &Window, cx: &App) -> impl IntoElement + use<> {
        let (text_primary, text_secondary) = {
            let theme = self.theme_store.read(cx).current_theme();
            (theme.text.primary, theme.text.secondary)
        };
        let caption = self.caption.get(cx);
        let drafts = self.drafts(cx);
        let (input_width, focus_token, requested_selection, selection_token) = {
            let state = state.read(cx);
            (state.input_width, state.composer_focus_token, state.composer_requested_selection, state.composer_selection_token)
        };

        let selection_state = state.clone();
        let selection_caption = self.caption.clone();
        let window_handle = window.window_handle();
        let emoji_drafts = self.caption_custom_emojis.clone();
        let on_send = self.on_send.clone();
        let send_selection = self.selection.clone();

        ChatComposerTextView::new(
            self.child_id("composer"),
            self.caption.clone(),
            input_width,
            CaptionBarMetrics::CIRCLE,
            CaptionBarMetrics::INPUT_MAX_HEIGHT,
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
                |state: &MediaPickerCaptionBarState| state.composer_input_height,
                |state, height, _| state.composer_input_height = height,
            ),
        )
        .custom_emoji_runs(ChatComposerCaretEdit::custom_emoji_runs(&caption, &drafts))
        .focus_token(focus_token)
        .requested_selection(requested_selection)
        .selection_token(selection_token)
        .on_selection_change(move |range, cx| {
            let text = selection_caption.get(cx);
            selection_state.update(cx, |state, cx| {
                state.composer_selection = Some(range);
                state.composer_selection_text = text;
                cx.notify();
            });
        })
        .on_custom_emoji_runs_change(move |runs, cx| {
            let Some(caption_custom_emojis) = emoji_drafts.clone() else {
                return;
            };
            let unchanged = {
                let current = caption_custom_emojis.get(cx);
                runs.len() == current.len() && runs.iter().zip(current.iter()).all(|(run, draft)| run.item == draft.item)
            };
            if unchanged {
                return;
            }
            let drafts: Vec<ChatDraftCustomEmoji> = runs
                .into_iter()
                .map(|run| ChatDraftCustomEmoji { placeholder: OBJECT_REPLACEMENT.to_string(), item: run.item })
                .collect();
            cx.defer(move |cx| {
                window_handle
                    .update(cx, |_, window, cx| caption_custom_emojis.set(drafts, window, cx))
                    .ok();
            });
        })
        .on_send(move |window, cx| {
            if !send_selection.read(cx).is_empty() {
                on_send(window, cx);
            }
        })
    }
}

impl RenderOnce for MediaPickerCaptionBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| MediaPickerCaptionBarState::new());
        let text_secondary = self.theme_store.read(cx).current_theme().text.secondary;
        let can_send = self.send_enabled.unwrap_or_else(|| !self.selection.read(cx).is_empty());
        let caption_is_empty = self.caption.get(cx).is_empty();
        let (composer_input_height, input_width, picker_visible) = {
            let state = state.read(cx);
            (state.composer_input_height, state.input_width, state.picker_visible)
        };
        let picker_reveal = state.read(cx).picker_motion.frame_value(RiseClock::monotonic(), window);

        let on_visible_change = self.on_picker_visible_change.clone();
        on_change(
            self.child_id("picker-visible"),
            picker_visible,
            false,
            window,
            cx,
            move |_, visible, window, cx| {
                if let Some(on_picker_visible_change) = &on_visible_change {
                    on_picker_visible_change(*visible, window, cx);
                }
            },
        );

        let width_state = state.clone();
        let geometry_reader = measure(self.child_id("input-geometry"), move |bounds, _, cx| {
            width_state.update(cx, |state, cx| {
                let width = f32::from(bounds.size.width);
                if (state.input_width - width).abs() > 0.5 {
                    state.input_width = width;
                    cx.notify();
                }
            });
        });

        let press_state = state.clone();
        let field = div()
            .id(self.child_id("field"))
            .capture_any_mouse_down(move |_: &MouseDownEvent, _, cx| {
                if press_state.read(cx).picker_visible {
                    press_state.update(cx, |state, cx| state.set_picker_visible(false, cx));
                }
            })
            .child(self.composer_text_view(&state, window, cx));

        let field_content = div()
            .relative()
            .w(px(input_width))
            .h(px(composer_input_height))
            .when(caption_is_empty, |content| {
                content.child(
                    pass_through()
                        .absolute()
                        .left(px(16.0))
                        .top(px(10.0))
                        .text_size(px(15.0))
                        .text_color(text_secondary.opacity(0.82))
                        .child(SharedString::from(tr("media_picker_caption_placeholder"))),
                )
            })
            .child(field)
            .child(div().absolute().right_0().bottom_0().child(self.picker_toggle(&state, cx)));

        let input = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(composer_input_height))
            .block_mouse_except_scroll()
            .child(geometry_reader)
            .child(field_content.liquid_chrome_rounded_rectangle_with(CaptionBarMetrics::INPUT_CORNER, 0.12, None, true));

        let send_button = self.send_button(can_send, cx);
        let row = hstack(CaptionBarMetrics::ROW_GAP)
            .align(VerticalAlignment::Bottom)
            .w_full()
            .child(input)
            .child(send_button)
            .padding_edges(EdgeSet::HORIZONTAL, CaptionBarMetrics::H_PADDING)
            .padding_edges(EdgeSet::TOP, CaptionBarMetrics::TOP_PADDING)
            .padding_edges(EdgeSet::BOTTOM, 5.0)
            .keyboard_dismiss_excluded();

        let panel = (picker_reveal > 0.0).then(|| {
            div().w_full().child(
                div()
                    .w_full()
                    .h(px(PICKER_PANEL_HEIGHT * picker_reveal))
                    .overflow_hidden()
                    .occlude()
                    .child(div().w_full().h(px(PICKER_PANEL_HEIGHT)).child(self.emoji_panel(&state, cx)))
                    .keyboard_dismiss_excluded(),
            )
        });

        let disappear_state = state.clone();
        let disappear_action = self.on_picker_visible_change.clone();
        vstack(0.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(div().w_full().child(row))
            .children(panel)
            .pb(px(self.bottom_safe_inset * (1.0 - picker_reveal)))
            .on_disappear(self.child_id("disappear"), move |window, cx| {
                if let Some(window) = window
                    && disappear_state.read(cx).picker_visible
                    && let Some(on_picker_visible_change) = &disappear_action
                {
                    on_picker_visible_change(false, window, cx);
                }
            })
    }
}
