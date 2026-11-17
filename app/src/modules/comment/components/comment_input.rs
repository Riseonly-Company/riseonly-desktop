use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, App, BoxShadow, Context, ElementId, Entity, FocusHandle, IntoElement, KeyDownEvent, Keystroke, MouseDownEvent,
    RenderOnce, SharedString, Subscription, Task, Window, anchored, point, relative,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::NSRange;
use rise_core::rise_app_events::RiseAppEvents;
use rise_i18n::tr;
use rise_platform::haptics;
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore};
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::keyboard_dismiss::{KeyboardDismissModifiers, RISE_INPUT_PICKER_DISMISS_REQUESTED, ScreenFocus};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::rich_text::rich_text_markup::RichEditorStyle;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::window_cover::WindowCoverFrame;
use rise_widgets::buttons::composer_send_button::ComposerSendButton;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::widgets::inputs::composer_reply_bar::{ComposerReplyBar, ComposerReplyBarKind};
use crate::modules::chat::components::chat::composer::chat_composer_input_surface::ChatComposerInputSurface;
use crate::modules::chat::components::chat::composer::chat_composer_text_view::ChatComposerTextView;
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatBottomBarMetrics, ChatComposerInputChromeModifiers, resign_chat_keyboard,
};
use crate::modules::chat::components::chat::input_picker::chat_gif_search_overlay::{ChatGifSearchOverlay, GifSearchHero};
use crate::modules::chat::components::chat::input_picker::chat_input_picker_panel::ChatInputPickerPanel;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::stickers::components::sticker_pack_add_cell::StickerAssetComposerStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{GifItem, PackKind, StickerItem};

type CommentInputVisibilityChange = Rc<dyn Fn(bool, f64, &mut App)>;
type CommentInputLiftChange = Rc<dyn Fn(f32, &mut App)>;

const PICKER_PANEL_HEIGHT: f32 = 336.0;
const PICKER_POPOVER_TOP_CORNER: f32 = 28.0;
const PICKER_POPOVER_FOOT: f32 = 16.0;
const PICKER_POPOVER_GAP: f32 = 6.0;
const PICKER_POPOVER_RISE: f32 = 12.0;

#[derive(IntoElement)]
pub struct CommentInput {
    id: ElementId,
    preview_text: Option<String>,
    on_keyboard_visibility_change: CommentInputVisibilityChange,
    on_picker_lift_change: CommentInputLiftChange,
}

struct CommentInputState {
    focus_token: i64,
    input_height: f32,
    input_width: f32,
    input_surface: ChatComposerInputSurface,
    picker_visible: bool,
    picker_motion: Animated<f32>,
    bar_motion: Animated<f32>,
    reported_picker_lift: f32,
    gif_search_visible: bool,
    selection: NSRange,
    selection_text: String,
    requested_selection: Option<NSRange>,
    selection_token: i64,
    focus_handle: FocusHandle,
    on_keyboard_visibility_change: CommentInputVisibilityChange,
    on_picker_lift_change: CommentInputLiftChange,
    lift_release: Option<Task<()>>,
    _subscriptions: [Subscription; 2],
}

impl CommentInputState {
    fn new(cx: &mut Context<Self>) -> Self {
        let interactions = CommentInteractionsStore::shared(cx);
        let post = PostServicesStore::shared(cx);
        Self {
            focus_token: 0,
            input_height: ChatBottomBarMetrics::CIRCLE,
            input_width: 0.0,
            input_surface: ChatComposerInputSurface::SystemKeyboard,
            picker_visible: false,
            picker_motion: Animated::new(0.0),
            bar_motion: Animated::new(0.0),
            reported_picker_lift: 0.0,
            gif_search_visible: false,
            selection: NSRange::new(0, 0),
            selection_text: String::new(),
            requested_selection: None,
            selection_token: 0,
            focus_handle: cx.focus_handle(),
            on_keyboard_visibility_change: Rc::new(|_: bool, _: f64, _: &mut App| {}),
            on_picker_lift_change: Rc::new(|_: f32, _: &mut App| {}),
            lift_release: None,
            _subscriptions: [cx.observe(&interactions, |_, _, cx| cx.notify()), cx.observe(&post, |_, _, cx| cx.notify())],
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

    fn report_picker_lift(&mut self, lift: f32, cx: &mut Context<Self>) {
        if (self.reported_picker_lift - lift).abs() <= 0.5 {
            return;
        }
        self.reported_picker_lift = lift;
        let on_picker_lift_change = self.on_picker_lift_change.clone();
        on_picker_lift_change(lift, cx);
    }

    fn open_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.input_surface = ChatComposerInputSurface::Picker;
        self.lift_release = None;
        self.set_picker_visible(true, cx);
        let on_keyboard_visibility_change = self.on_keyboard_visibility_change.clone();
        on_keyboard_visibility_change(true, ChatBottomBarMetrics::COMPOSER_ANIM_DURATION, cx);
        self.focus_token = self.focus_token.wrapping_add(1);
    }

    /// the window click that dismisses the keyboard
    /// panel and composer are excluded so this is a click on the list or the header
    fn fold_picker_from_outside(&mut self, cx: &mut Context<Self>) {
        if !self.picker_visible && self.input_surface != ChatComposerInputSurface::Picker {
            return;
        }
        self.input_surface = ChatComposerInputSurface::SystemKeyboard;
        self.set_picker_visible(false, cx);
        self.report_picker_lift(0.0, cx);
    }

    fn switch_to_keyboard_and_focus(&mut self, cx: &mut Context<Self>) {
        self.input_surface = ChatComposerInputSurface::SystemKeyboard;
        self.set_picker_visible(false, cx);
        self.focus_token = self.focus_token.wrapping_add(1);
        let delay = ChatBottomBarMetrics::COMPOSER_ANIM_DURATION + 0.4;
        self.lift_release = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            let _ = this.update(cx, |state, cx| {
                if state.input_surface == ChatComposerInputSurface::SystemKeyboard {
                    state.report_picker_lift(0.0, cx);
                }
            });
        }));
    }

    fn field_pressed(&mut self, cx: &mut Context<Self>) {
        if self.input_surface != ChatComposerInputSurface::Picker {
            return;
        }
        self.input_surface = ChatComposerInputSurface::SystemKeyboard;
        self.set_picker_visible(false, cx);
        self.report_picker_lift(0.0, cx);
    }

    /// the panel stays up, only a press on the field folds it and focus alone doesnt
    fn place_caret(&mut self, range: Option<NSRange>, text: String, cx: &mut Context<Self>) {
        self.focus_token = self.focus_token.wrapping_add(1);
        let Some(range) = range else {
            cx.notify();
            return;
        };
        self.selection = range;
        self.selection_text = text;
        self.requested_selection = Some(range);
        self.selection_token = self.selection_token.wrapping_add(1);
        cx.notify();
    }
}

impl CommentInput {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            preview_text: None,
            on_keyboard_visibility_change: Rc::new(|_: bool, _: f64, _: &mut App| {}),
            on_picker_lift_change: Rc::new(|_: f32, _: &mut App| {}),
        }
    }

    pub fn preview_text(mut self, preview_text: Option<String>) -> Self {
        self.preview_text = preview_text;
        self
    }

    pub fn on_keyboard_visibility_change(mut self, on_change: impl Fn(bool, f64, &mut App) + 'static) -> Self {
        self.on_keyboard_visibility_change = Rc::new(on_change);
        self
    }

    pub fn on_picker_lift_change(mut self, on_change: impl Fn(f32, &mut App) + 'static) -> Self {
        self.on_picker_lift_change = Rc::new(on_change);
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn closed_plate(&self, theme: &AppTheme) -> AnyElement {
        div()
            .w_full()
            .px(px(ChatBottomBarMetrics::H_PADDING))
            .pt(px(ChatBottomBarMetrics::TOP_PADDING))
            .pb(px(6.0))
            .child(
                div()
                    .w_full()
                    .min_h(px(ChatBottomBarMetrics::CIRCLE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(MainText::new(tr("can_comment_false")).px(15.0).tac(TextAlignT::Center).color(theme.text.secondary)),
            )
            .keyboard_dismiss_excluded()
            .into_any_element()
    }

    fn composer_row(
        &self,
        theme: &AppTheme,
        state: &Entity<CommentInputState>,
        interactions: &Entity<CommentInteractionsStore>,
        can_comment: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let is_preview = self.preview_text.is_some();
        let failed_photo = if is_preview { None } else { interactions.update(cx, |store, cx| store.failed_photo_draft(cx)) };
        let (draft_text, reply_target_name, reply_target_snippet, reply_target_is_official, reply_target_is_premium, custom_emoji_runs) = {
            let store = interactions.read(cx);
            let draft_text = self.preview_text.clone().unwrap_or_else(|| store.comment_text().to_string());
            let runs = if is_preview { Vec::new() } else { store.composer_custom_emoji_runs(store.comment_text()) };
            (
                draft_text,
                store.reply_target_name.clone(),
                store.reply_target_snippet.clone(),
                store.reply_target_is_official,
                store.reply_target_is_premium,
                runs,
            )
        };
        let can_send =
            can_comment && (failed_photo.is_some() || !trimming_whitespaces_and_newlines(&draft_text).is_empty());
        let is_replying = !is_preview && reply_target_name.as_deref().is_some_and(|name| !name.is_empty());
        let bar_count = i64::from(is_replying) + i64::from(failed_photo.is_some());
        let bar_height = bar_count as f32 * ChatBottomBarMetrics::REPLY_EDIT_BAR_HEIGHT;

        let motion_state = state.clone();
        on_change(self.child_id("bar-height"), bar_count, true, window, cx, move |old, new, _, cx| {
            let spec = (old != new).then(|| ease_out(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION));
            motion_state.update(cx, |state, cx| {
                state.bar_motion.set_in(bar_height, spec, RiseClock::monotonic(), cx);
                cx.notify();
            });
        });

        let (input_width, input_height, focus_token, requested_selection, selection_token, surface, animated_bar_height) = {
            let state = state.read(cx);
            (
                state.input_width,
                state.input_height,
                state.focus_token,
                state.requested_selection,
                state.selection_token,
                state.input_surface,
                state.bar_motion.frame_value(RiseClock::monotonic(), window),
            )
        };

        let text_binding = match &self.preview_text {
            Some(preview) => Binding::constant(preview.clone()),
            None => Binding::entity(
                interactions.clone(),
                |store: &CommentInteractionsStore| store.comment_text().to_string(),
                |store, text: String, cx| store.set_comment_text(&text, cx),
            ),
        };

        let selection_state = state.clone();
        let selection_store = interactions.clone();
        let runs_store = interactions.clone();
        let send_store = interactions.clone();
        let mut text_view = ChatComposerTextView::new(
            self.child_id("field"),
            text_binding,
            input_width,
            ChatBottomBarMetrics::CIRCLE,
            ChatBottomBarMetrics::INPUT_MAX_HEIGHT,
            theme.text.primary,
            RichEditorStyle::make(
                MessageSizes::composer_text_font_size(),
                theme.text.primary,
                theme.text.secondary.opacity(0.7),
                theme.text.primary,
                theme.text.primary.opacity(0.12),
            ),
            Binding::entity(
                state.clone(),
                |state: &CommentInputState| state.input_height,
                |state, height, cx| {
                    state.input_height = height;
                    cx.notify();
                },
            ),
        )
        .custom_emoji_runs(custom_emoji_runs)
        .focus_token(focus_token)
        .on_cancel(|window, cx| {
            ScreenFocus::release_text_field(window, cx);
            if let Ok(escape) = Keystroke::parse("escape") {
                window.dispatch_keystroke(escape, cx);
            }
        })
        .requested_selection(requested_selection)
        .selection_token(selection_token)
        .on_selection_change(move |range, cx| {
            let text = selection_store.read(cx).comment_text().to_string();
            selection_state.update(cx, |state, cx| {
                state.selection = range;
                state.selection_text = text;
                cx.notify();
            });
        })
        .on_custom_emoji_runs_change(move |runs, cx| {
            if is_preview {
                return;
            }
            let store = runs_store.clone();
            cx.defer(move |cx| store.update(cx, |store, cx| store.sync_custom_emoji_drafts(&runs, cx)));
        })
        .on_send(move |_, cx| {
            if can_send && !is_preview {
                send_store.update(cx, |store, cx| store.send_current_comment(cx));
            }
        });
        if surface == ChatComposerInputSurface::Picker {
            let cancel_state = state.clone();
            text_view = text_view.on_cancel(move |_, cx| {
                cancel_state.update(cx, |state, cx| state.fold_picker_from_outside(cx));
            });
        }

        let press_state = state.clone();
        let field = div()
            .id(self.child_id("field-press"))
            .capture_any_mouse_down(move |_: &MouseDownEvent, _, cx| {
                press_state.update(cx, |state, cx| state.field_pressed(cx));
            })
            .child(text_view);

        let field_content = div()
            .relative()
            .w(px(input_width))
            .h(px(input_height))
            .when(draft_text.is_empty(), |content| {
                content.child(
                    pass_through()
                        .absolute()
                        .left(px(16.0))
                        .right(px(44.0))
                        .top(px(10.0))
                        .truncate()
                        .text_size(px(15.0))
                        .text_color(theme.text.secondary.opacity(0.82))
                        .child(SharedString::from(tr("create_comment_placeholder"))),
                )
            })
            .child(field)
            .when(!can_comment, |content| content.child(div().absolute().inset_0().occlude()))
            .child(self.picker_toggle(theme, state, surface, can_comment));

        let dismiss_store = interactions.clone();
        let cancel_store = interactions.clone();
        let field_stack = vstack(0.0)
            .w_full()
            .when_some(failed_photo, |stack, failed_photo| {
                let source = failed_photo.thumbnail_url.clone().unwrap_or_else(|| failed_photo.file_url.clone());
                stack.child(
                    div().w_full().pl(px(14.0)).pr(px(6.0)).h(px(ChatBottomBarMetrics::REPLY_EDIT_BAR_HEIGHT)).child(
                        ComposerReplyBar::new(
                            self.child_id("failed-photo-bar"),
                            tr("composer_photo"),
                            tr("chat_message_status_failed"),
                            move |_, cx| dismiss_store.update(cx, |store, cx| store.dismiss_failed_photo_draft(cx)),
                        )
                        .media_thumbnail_source(Some(source))
                        .theme(Some(theme.clone())),
                    ),
                )
            })
            .when(is_replying, |stack| {
                stack.child(
                    div().w_full().pl(px(14.0)).pr(px(6.0)).h(px(ChatBottomBarMetrics::REPLY_EDIT_BAR_HEIGHT)).child(
                        ComposerReplyBar::new(
                            self.child_id("reply-bar"),
                            reply_target_name.clone().unwrap_or_default(),
                            reply_target_snippet.clone().unwrap_or_default(),
                            move |_, cx| cancel_store.update(cx, |store, cx| store.cancel_reply(cx)),
                        )
                        .kind(ComposerReplyBarKind::Reply)
                        .is_official(reply_target_is_official)
                        .is_premium(reply_target_is_premium)
                        .theme(Some(theme.clone())),
                    ),
                )
            })
            .child(field_content)
            .when(theme.appearance == RiseThemeAppearance::Light, |stack| {
                stack.bg(theme.bg._300).rounded(px(ChatBottomBarMetrics::INPUT_CORNER))
            })
            .chat_composer_input_chrome(false);

        let width_state = state.clone();
        let input = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(input_height + animated_bar_height))
            .child(measure(self.child_id("input-geometry"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                width_state.update(cx, |state, cx| {
                    if (state.input_width - width).abs() > 0.5 {
                        state.input_width = width;
                        cx.notify();
                    }
                });
            }))
            .child(field_stack);

        let press_store = interactions.clone();
        hstack(ChatBottomBarMetrics::ROW_GAP)
            .align(VerticalAlignment::Bottom)
            .w_full()
            .px(px(ChatBottomBarMetrics::H_PADDING))
            .pt(px(ChatBottomBarMetrics::TOP_PADDING))
            .pb(px(6.0))
            .child(self.attach_button(theme, interactions, can_comment))
            .child(input)
            .child(
                ComposerSendButton::new(self.child_id("send"), move |_, cx| {
                    if !is_preview {
                        press_store.update(cx, |store, cx| store.send_current_comment(cx));
                    }
                })
                .size(ChatBottomBarMetrics::CIRCLE)
                .is_enabled(can_send),
            )
            .keyboard_dismiss_excluded()
            .into_any_element()
    }

    fn picker_toggle(
        &self,
        theme: &AppTheme,
        state: &Entity<CommentInputState>,
        surface: ChatComposerInputSurface,
        enabled: bool,
    ) -> AnyElement {
        let toggle_state = state.clone();
        let button = plain(self.child_id("picker-toggle"))
            .disabled(!enabled)
            .on_press(move |window, cx| {
                let _ = haptics::selection_changed();
                toggle_state.update(cx, |state, cx| {
                    if state.input_surface == ChatComposerInputSurface::Picker {
                        state.switch_to_keyboard_and_focus(cx);
                    } else {
                        state.open_picker(window, cx);
                    }
                });
            })
            .child(
                div()
                    .w(px(38.0))
                    .h(px(ChatBottomBarMetrics::CIRCLE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(surface.toggle_icon_system_name(), px(21.0), theme.text.secondary, false)),
            );
        div()
            .id(self.child_id("picker-toggle-hint"))
            .tooltip_label(tr("picker_tab_emoji"))
            .absolute()
            .right_0()
            .bottom_0()
            .pr(px(4.0))
            .block_mouse_except_scroll()
            .child(button)
            .keyboard_dismiss_excluded()
            .into_any_element()
    }

    fn attach_button(&self, theme: &AppTheme, interactions: &Entity<CommentInteractionsStore>, enabled: bool) -> AnyElement {
        let store = interactions.clone();
        let button = LiquidGlassButton::new(self.child_id("attach"), LiquidGlassButtonShape::Circle)
            .size(ChatBottomBarMetrics::CIRCLE)
            .disabled(!enabled)
            .tint((theme.appearance == RiseThemeAppearance::Light).then_some(theme.bg._300))
            .accessibility_label(Some(tr("media_picker_title")))
            .on_press(move |window, cx| store.update(cx, |store, cx| store.open_photo_picker(window, cx)))
            .children(IconUi::sized("paperclip", px(18.0), theme.text.primary, false));
        div()
            .id(self.child_id("attach-hint"))
            .flex_none()
            .tooltip_label(tr("media_picker_title"))
            .child(button)
            .keyboard_dismiss_excluded()
            .into_any_element()
    }

    fn caret(state: &Entity<CommentInputState>, interactions: &Entity<CommentInteractionsStore>, cx: &App) -> Option<NSRange> {
        let state = state.read(cx);
        (state.selection_text == interactions.read(cx).comment_text()).then_some(state.selection)
    }

    fn place_caret(
        state: &Entity<CommentInputState>,
        interactions: &Entity<CommentInteractionsStore>,
        range: Option<NSRange>,
        cx: &mut App,
    ) {
        let text = interactions.read(cx).comment_text().to_string();
        state.update(cx, |state, cx| state.place_caret(range, text, cx));
    }

    fn can_comment_on_selected_post(post: &Entity<PostServicesStore>, cx: &App) -> bool {
        post.read(cx).selected_post.as_ref().is_some_and(|post| post.can_comment)
    }

    fn open_pack_settings(
        kind: PackKind,
        nav_store: Option<&Entity<NavigationStore>>,
        state: &Entity<CommentInputState>,
        interactions: &Entity<CommentInteractionsStore>,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.update(cx, |state, cx| {
            state.set_picker_visible(false, cx);
            state.report_picker_lift(0.0, cx);
        });
        resign_chat_keyboard(window, cx);
        interactions.update(cx, |store, cx| {
            store.comments_sheet_visible = false;
            store.close_comments_sheet(cx);
        });
        let Some(nav_store) = nav_store.cloned() else {
            return;
        };
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(BottomSheetMetrics::DISMISS_DURATION + 0.05)).await;
            let _ = cx.update(|cx| {
                nav_store.update(cx, |store, cx| {
                    store.push(RootRoute::StickerSettings { kind }, NavigationStoreSettlingPush::Drop, cx);
                });
            });
        })
        .detach();
    }

    fn picker_panel(
        &self,
        state: &Entity<CommentInputState>,
        interactions: &Entity<CommentInteractionsStore>,
        post: &Entity<PostServicesStore>,
        gif_search_visible: bool,
        nav_store: Option<Entity<NavigationStore>>,
    ) -> AnyElement {
        let (emoji_state, emoji_store) = (state.clone(), interactions.clone());
        let (custom_state, custom_store) = (state.clone(), interactions.clone());
        let (backspace_state, backspace_store) = (state.clone(), interactions.clone());
        let (sticker_store, sticker_post) = (interactions.clone(), post.clone());
        let (gif_store, gif_post) = (interactions.clone(), post.clone());
        let gif_search_state = state.clone();
        let (pack_state, pack_store) = (state.clone(), interactions.clone());
        div()
            .w_full()
            .h(px(PICKER_PANEL_HEIGHT))
            .child(
                ChatInputPickerPanel::new(
                    self.child_id("picker-panel"),
                    PICKER_PANEL_HEIGHT,
                    move |emoji: String, _: &mut Window, cx: &mut App| {
                        let caret = Self::caret(&emoji_state, &emoji_store, cx);
                        let range = emoji_store.update(cx, |store, cx| store.insert_emoji(&emoji, caret, cx));
                        Self::place_caret(&emoji_state, &emoji_store, Some(range), cx);
                    },
                    move |item: StickerItem, _: &mut Window, cx: &mut App| {
                        let caret = Self::caret(&custom_state, &custom_store, cx);
                        let range = custom_store.update(cx, |store, cx| store.append_custom_emoji(&item, caret, cx));
                        Self::place_caret(&custom_state, &custom_store, range, cx);
                    },
                    move |_: &mut Window, cx: &mut App| {
                        let caret = Self::caret(&backspace_state, &backspace_store, cx);
                        let range = backspace_store.update(cx, |store, cx| store.delete_backward(caret, cx));
                        Self::place_caret(&backspace_state, &backspace_store, range, cx);
                    },
                    move |sticker: StickerItem, _: &mut Window, cx: &mut App| {
                        if !Self::can_comment_on_selected_post(&sticker_post, cx) {
                            return;
                        }
                        sticker_store.update(cx, |store, cx| store.send_sticker(&sticker, cx));
                    },
                    move |gif: GifItem, _: &mut Window, cx: &mut App| {
                        if !Self::can_comment_on_selected_post(&gif_post, cx) {
                            return;
                        }
                        gif_store.update(cx, |store, cx| store.send_gif(&gif, cx));
                    },
                    move |_: &mut Window, cx: &mut App| {
                        gif_search_state.update(cx, |state, cx| {
                            state.gif_search_visible = true;
                            cx.notify();
                        });
                    },
                    move |kind: PackKind, window: &mut Window, cx: &mut App| {
                        Self::open_pack_settings(kind, nav_store.as_ref(), &pack_state, &pack_store, window, cx);
                    },
                )
                .gif_search_namespace(Some(self.child_id("gif-search-namespace")))
                .gif_search_active(gif_search_visible)
                .uses_flat_chrome(true),
            )
            .keyboard_dismiss_excluded()
            .into_any_element()
    }

    fn picker_popover(theme: &AppTheme, panel: AnyElement, progress: f32) -> AnyElement {
        div()
            .absolute()
            .left(px(ChatBottomBarMetrics::H_PADDING))
            .right(px(ChatBottomBarMetrics::H_PADDING))
            .bottom(relative(1.0))
            .pb(px(PICKER_POPOVER_GAP))
            .child(
                div()
                    .relative()
                    .w_full()
                    .top(px(PICKER_POPOVER_RISE * (1.0 - progress)))
                    .opacity(progress)
                    .pb(px(PICKER_POPOVER_FOOT))
                    .rounded_t(px(PICKER_POPOVER_TOP_CORNER))
                    .rounded_b(px(PICKER_POPOVER_FOOT))
                    .bg(theme.bg._100)
                    .border_1()
                    .border_color(theme.text.primary.opacity(0.08))
                    .shadow(vec![BoxShadow {
                        color: gpui::black().opacity(0.28),
                        offset: point(px(0.0), px(8.0)),
                        blur_radius: px(18.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .occlude()
                    .child(panel)
                    .keyboard_dismiss_excluded(),
            )
            .into_any_element()
    }

    fn gif_search_cover(
        &self,
        state: &Entity<CommentInputState>,
        interactions: &Entity<CommentInteractionsStore>,
        post: &Entity<PostServicesStore>,
        presence: &Presence<()>,
        window: &Window,
        cx: &App,
    ) -> Option<AnyElement> {
        let now = RiseClock::monotonic();
        presence.value(now)?;
        let (gif_store, gif_post) = (interactions.clone(), post.clone());
        let cover = WindowCoverFrame::resolve(window, cx);
        let surface = div().w(cover.size.width).h(cover.size.height).occlude().child(
            ChatGifSearchOverlay::new(
                self.child_id("gif-search"),
                Binding::entity(
                    state.clone(),
                    |state: &CommentInputState| state.gif_search_visible,
                    |state, visible, cx| {
                        state.gif_search_visible = visible;
                        cx.notify();
                    },
                ),
                move |gif: GifItem, _: &mut Window, cx: &mut App| {
                    if !Self::can_comment_on_selected_post(&gif_post, cx) {
                        return;
                    }
                    gif_store.update(cx, |store, cx| store.send_gif(&gif, cx));
                },
            )
            .uses_flat_chrome(true),
        );
        let surface = presence.render_transitioned(now, window, move |_| surface)?;
        Some(anchored().position(cover.origin).child(surface).into_any_element())
    }
}

impl RenderOnce for CommentInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, cx| CommentInputState::new(cx));
        let on_keyboard_visibility_change = self.on_keyboard_visibility_change.clone();
        let on_picker_lift_change = self.on_picker_lift_change.clone();
        state.update(cx, |state, _| {
            state.on_keyboard_visibility_change = on_keyboard_visibility_change;
            state.on_picker_lift_change = on_picker_lift_change;
        });

        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let interactions = CommentInteractionsStore::shared(cx);
        let post = PostServicesStore::shared(cx);
        let nav_store = env::<NavigationStoreKey>(cx);
        let is_preview = self.preview_text.is_some();
        let selected_can_comment = post.read(cx).selected_post.as_ref().map(|post| post.can_comment);
        let comments_closed = !is_preview && selected_can_comment == Some(false);
        let can_comment = is_preview || selected_can_comment.unwrap_or(false);

        let bump_state = state.clone();
        let focus_bump_signal = interactions.read(cx).focus_bump_signal;
        on_change(self.child_id("focus-bump"), focus_bump_signal, false, window, cx, move |_, _, _, cx| {
            if is_preview {
                return;
            }
            bump_state.update(cx, |state, cx| state.switch_to_keyboard_and_focus(cx));
        });

        let dismiss_state = state.clone();
        use_task(self.child_id("picker-dismiss"), (), window, cx, async move |cx| {
            let (sender, mut receiver) = mpsc::unbounded::<()>();
            let _subscription = RiseAppEvents::default_center().observe(RISE_INPUT_PICKER_DISMISS_REQUESTED, move |_| {
                let _ = sender.unbounded_send(());
            });
            while receiver.next().await.is_some() {
                let dismiss_state = dismiss_state.clone();
                let _ = cx.update(|_, cx| {
                    if StickerAssetComposerStore::shared(cx).read(cx).preserves_input_picker_presentation() {
                        return;
                    }
                    dismiss_state.update(cx, |state, cx| state.fold_picker_from_outside(cx));
                });
            }
        });

        let (picker_visible, picker_progress, gif_search_visible, focus_handle) = {
            let state = state.read(cx);
            (
                state.picker_visible,
                state.picker_motion.frame_value(RiseClock::monotonic(), window),
                state.gif_search_visible,
                state.focus_handle.clone(),
            )
        };
        let panel_is_mounted = picker_visible || picker_progress > 0.001;

        let gif_search_presence = window.use_keyed_state(self.child_id("gif-search-presence"), cx, move |_, _| {
            Presence::new(gif_search_visible.then_some(())).with_transition(Transition::opacity())
        });
        if gif_search_presence.read(cx).is_present() != gif_search_visible {
            let now = RiseClock::monotonic();
            gif_search_presence.update(cx, |presence, cx| {
                presence.set_in(gif_search_visible.then_some(()), GifSearchHero::animation(), now, cx);
            });
        }
        let gif_search =
            self.gif_search_cover(&state, &interactions, &post, gif_search_presence.read(cx), window, cx);

        let row = if comments_closed {
            self.closed_plate(&theme)
        } else {
            self.composer_row(&theme, &state, &interactions, can_comment, window, cx)
        };

        let key_state = state.clone();
        let disappear_state = state.clone();
        vstack(0.0)
            .relative()
            .w_full()
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && key_state.read(cx).picker_visible {
                    key_state.update(cx, |state, cx| state.fold_picker_from_outside(cx));
                    cx.stop_propagation();
                }
            })
            .child(row)
            .when(panel_is_mounted, |footer| {
                let panel = self.picker_panel(&state, &interactions, &post, gif_search_visible, nav_store);
                footer.child(Self::picker_popover(&theme, panel, picker_progress))
            })
            .children(gif_search)
            .on_disappear(self.child_id("disappear"), move |_, cx| {
                disappear_state.update(cx, |state, cx| {
                    state.lift_release = None;
                    state.input_surface = ChatComposerInputSurface::SystemKeyboard;
                    state.set_picker_visible(false, cx);
                    state.report_picker_lift(0.0, cx);
                });
            })
    }
}
