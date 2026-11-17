use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{App, ElementId, Entity, FontWeight, SharedString, Subscription, Window, transparent_black, white};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::keyboard_dismiss::KeyboardDismiss;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::section_reveal_ui::SectionRevealUi;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Edge, Presence, Transition};

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::core::ui::emoji_picker_ui::emoji_picker_ui::EmojiPickerUi;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::chat::components::chat::topic_icon_glyph::TopicIconGlyph;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::chat::pages::chats::chat::chat_topics::channel_kind_editor::{
    ChannelKindPicker, VoiceChannelConfigDraft, VoiceChannelConfigEditor,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatChannelManagementPolicy, ChatTopic, VoiceChannelConfig,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTOExt;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

static TITLE_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("chat_create_topic", "/title"));

const SAVE_ERROR_FADE_SECONDS: f64 = 0.2;
const TITLE_TILE_GLYPH: &str = "Aa";

type TopicEditorSheetOnCompleted = Rc<dyn Fn(bool, &mut App)>;
type TopicEditorSheetOnSaved = Rc<dyn Fn(ChatTopic, &mut App)>;

/// state of the sheet, titleFocused cant be written to the field directly
/// a focus request is a new field identity with auto_focus counted by focus_generation
struct TopicEditorSheetState {
    title: String,
    icon: String,
    emoji_picker_open: bool,
    title_emoji_open: bool,
    is_submitting: bool,
    save_error_visible: bool,
    save_error_presence: Presence<()>,
    completed_topic: Option<ChatTopic>,
    resting_fraction: f32,
    selected_fraction: f32,
    measured_content_height: f32,
    collapsed_content_height: f32,
    panel_was_open: bool,
    panel_closing: bool,
    sheet_host_height: f32,
    channel_kind: ChatChannelKind,
    voice_draft: VoiceChannelConfigDraft,
    title_focused: bool,
    focus_generation: u64,
    observes_services: bool,
    _services: Subscription,
}

impl TopicEditorSheetState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ChatServicesStore::shared(cx);
        let subscription = cx.observe(&services, |sheet, _, cx| {
            if sheet.observes_services {
                cx.notify();
            }
        });
        Self {
            title: String::new(),
            icon: ChatTopic::DEFAULT_ICON.to_owned(),
            emoji_picker_open: false,
            title_emoji_open: false,
            is_submitting: false,
            save_error_visible: false,
            save_error_presence: Presence::new(None)
                .with_transition(Transition::move_edge(Edge::Top).combined(Transition::opacity())),
            completed_topic: None,
            resting_fraction: TopicEditorSheetPresentationContract::MINIMUM_FRACTION,
            selected_fraction: TopicEditorSheetPresentationContract::MINIMUM_FRACTION,
            measured_content_height: 0.0,
            collapsed_content_height: 0.0,
            panel_was_open: false,
            panel_closing: false,
            sheet_host_height: 0.0,
            channel_kind: ChatChannelKind::Text,
            voice_draft: VoiceChannelConfigDraft::new(None),
            title_focused: false,
            focus_generation: 0,
            observes_services: false,
            _services: subscription,
        }
    }

    fn normalized_title(&self) -> &str {
        trimming_whitespaces_and_newlines(&self.title)
    }

    fn normalized_icon(&self) -> Option<String> {
        ChatTopic::selectable_icon(Some(self.icon.as_str()))
    }

    fn draft_voice_config(&self) -> Option<VoiceChannelConfig> {
        self.voice_draft.normalized(self.channel_kind)
    }

    fn set_save_error_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.save_error_visible == visible {
            return;
        }
        self.save_error_visible = visible;
        self.save_error_presence.set_in(
            visible.then_some(()),
            ease_out(SAVE_ERROR_FADE_SECONDS),
            RiseClock::monotonic(),
            cx,
        );
    }

    fn set_title(&mut self, value: String, cx: &mut Context<Self>) {
        if self.title == value {
            return;
        }
        self.title = TITLE_RULE.clamped(&value);
        self.set_save_error_visible(false, cx);
    }

    fn set_title_emoji_open(&mut self, open: bool) {
        if self.title_emoji_open == open {
            return;
        }
        self.title_emoji_open = open;
    }

    fn insert_title_emoji(&mut self, emoji: &str, cx: &mut Context<Self>) {
        let next = TITLE_RULE.clamped(&format!("{}{emoji}", self.title));
        if next == self.title {
            return;
        }
        self.title = next;
        self.set_save_error_visible(false, cx);
        cx.notify();
    }

    fn request_title_focus(&mut self, cx: &mut Context<Self>) {
        if self.title_focused {
            return;
        }
        self.focus_generation = self.focus_generation.wrapping_add(1);
        cx.notify();
    }

    fn adopt_content_height(&mut self, height: f32, cx: &mut Context<Self>) {
        self.measured_content_height = height;
        let target = self.target_content_height(height);
        if self.title_focused || target <= 0.0 || self.sheet_host_height <= 0.0 {
            return;
        }
        let measured = TopicEditorSheetPresentationContract::resting_fraction(target, self.sheet_host_height);
        if (measured - self.resting_fraction).abs() <= 0.005 {
            return;
        }
        tracing::warn!(
            target: "riseonly::X1673",
            host = self.sheet_host_height,
            height,
            target,
            collapsed = self.collapsed_content_height,
            viewport = self.emoji_picker_viewport_height(),
            fraction = measured,
            icon_open = self.emoji_picker_open,
            title_open = self.title_emoji_open,
            "topic editor detent"
        );
        self.resting_fraction = measured;
        self.selected_fraction = measured;
        cx.notify();
    }

    /// moves the resting detent to the panels end height in the same update that flips them
    /// so the measurement after the flip finds it settled and asks for no second frame
    fn settle_detent(&mut self) {
        let target = self.target_content_height(self.measured_content_height);
        if self.title_focused || target <= 0.0 || self.sheet_host_height <= 0.0 {
            return;
        }
        let fraction = TopicEditorSheetPresentationContract::resting_fraction(target, self.sheet_host_height);
        self.resting_fraction = fraction;
        self.selected_fraction = fraction;
    }

    /// height the sheet rests at for this measurement
    /// emoji panel reveals by a growing clip, following it moved the detent every spring frame
    /// so it goes at once to the end height
    fn target_content_height(&mut self, height: f32) -> f32 {
        let open = self.emoji_picker_open || self.title_emoji_open;
        let was_open = std::mem::replace(&mut self.panel_was_open, open);
        if open {
            self.panel_closing = false;
            if self.collapsed_content_height <= 0.0 {
                return height;
            }
            return self.collapsed_content_height + self.emoji_picker_expansion_height();
        }
        if was_open && self.collapsed_content_height > 0.0 {
            self.panel_closing = true;
        }
        if self.panel_closing {
            if height > self.collapsed_content_height + 0.5 {
                return self.collapsed_content_height;
            }
            self.panel_closing = false;
        }
        if height > 0.0 {
            self.collapsed_content_height = height;
        }
        height
    }

    /// emoji picker height, capped at 376
    /// desktop modal is shorter than a phone so 44 percent of it cut the grid to about two rows
    /// so the panel takes the height the resting cap leaves above the footer
    fn emoji_picker_viewport_height(&self) -> f32 {
        const CEILING: f32 = 376.0;
        let screen_share = CEILING.min(self.sheet_host_height * 0.44);
        if self.sheet_host_height <= 0.0 || self.collapsed_content_height <= 0.0 {
            return screen_share;
        }
        let free = self.sheet_host_height * TopicEditorSheetPresentationContract::MAXIMUM_RESTING_FRACTION
            - TopicEditorSheetPresentationContract::handle_zone()
            - TopicEditorSheetPresentationContract::KEYBOARD_FOOTER_SPACING
            - self.collapsed_content_height
            - 8.0;
        screen_share.max(free.min(CEILING))
    }

    fn emoji_picker_expansion_height(&self) -> f32 {
        self.emoji_picker_viewport_height() + 8.0
    }
}

/// topic editor sheet, owns its presentation and state so a caller only flips the binding
#[derive(IntoElement)]
pub struct TopicEditorSheet {
    is_presented: Binding<bool>,
    chat_id: String,
    topic_id: Option<String>,
    section_id: Option<String>,
    on_completed: Option<TopicEditorSheetOnCompleted>,
    on_saved: Option<TopicEditorSheetOnSaved>,
}

impl TopicEditorSheet {
    pub fn new(is_presented: Binding<bool>, chat_id: String) -> Self {
        Self { is_presented, chat_id, topic_id: None, section_id: None, on_completed: None, on_saved: None }
    }

    pub fn topic_id(mut self, topic_id: Option<String>) -> Self {
        self.topic_id = topic_id;
        self
    }

    pub fn section_id(mut self, section_id: Option<String>) -> Self {
        self.section_id = section_id;
        self
    }

    pub fn on_completed(mut self, on_completed: impl Fn(bool, &mut App) + 'static) -> Self {
        self.on_completed = Some(Rc::new(on_completed));
        self
    }

    pub fn on_saved(mut self, on_saved: impl Fn(ChatTopic, &mut App) + 'static) -> Self {
        self.on_saved = Some(Rc::new(on_saved));
        self
    }
}

impl RenderOnce for TopicEditorSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let id = ElementId::Name(format!("topic-editor-sheet-{}", self.chat_id).into());
        let state = window.use_keyed_state(named_child(&id, "state"), cx, |_, cx| TopicEditorSheetState::new(cx));
        let parts = TopicEditorSheetParts {
            id: id.clone(),
            state: state.clone(),
            is_presented: self.is_presented.clone(),
            chat_id: self.chat_id,
            topic_id: self.topic_id,
            section_id: self.section_id,
            on_completed: self.on_completed,
            on_saved: self.on_saved,
        };

        let is_presented = self.is_presented.get(cx);
        state.update(cx, |sheet, _| sheet.observes_services = is_presented);
        let allowed_channel_kinds = if is_presented { parts.allowed_channel_kinds(cx) } else { Vec::new() };
        let (channel_kind, title, is_submitting, resting_fraction) = {
            let sheet = state.read(cx);
            (sheet.channel_kind, sheet.title.clone(), sheet.is_submitting, sheet.resting_fraction)
        };

        let preparing = parts.clone();
        on_change(named_child(&id, "presented"), is_presented, true, window, cx, move |_, open, _, cx| {
            if *open {
                preparing.load_effective_permissions(cx);
                preparing.prepare_draft(cx);
            }
        });
        on_change(named_child(&id, "fact-opened"), is_presented, true, window, cx, |_, open, _, cx| {
            GuideFacts::set(RiseGuideSignal::TopicEditorOpened, *open, cx);
            if !*open {
                GuideFacts::set(RiseGuideSignal::TopicEditorVoiceSelected, false, cx);
                GuideFacts::set(RiseGuideSignal::TopicEditorNameFilled, false, cx);
            }
        });
        on_change(named_child(&id, "fact-voice"), channel_kind, true, window, cx, move |_, kind, _, cx| {
            GuideFacts::set(
                RiseGuideSignal::TopicEditorVoiceSelected,
                is_presented && *kind == ChatChannelKind::Voice,
                cx,
            );
        });
        on_change(named_child(&id, "fact-name"), title, true, window, cx, move |_, value, _, cx| {
            let filled = !trimming_whitespaces_and_newlines(value).is_empty();
            GuideFacts::set(RiseGuideSignal::TopicEditorNameFilled, is_presented && filled, cx);
        });
        let adopting = parts.clone();
        on_change(named_child(&id, "allowed-kinds"), allowed_channel_kinds, false, window, cx, move |_, kinds, _, cx| {
            adopting.adopt_allowed_channel_kinds(kinds, cx);
        });

        let content_parts = parts.clone();
        let footer_parts = parts.clone();
        let dismiss_parts = parts.clone();
        let host_state = state.clone();
        let selected_fraction = Binding::entity(
            state,
            |sheet: &TopicEditorSheetState| sheet.selected_fraction,
            |sheet, value, _| sheet.selected_fraction = value,
        );

        div()
            .absolute()
            .inset_0()
            .child(measure(named_child(&id, "host-height"), move |_, window, cx| {
                let host_height = f32::from(window.viewport_size().height);
                host_state.update(cx, |sheet, cx| {
                    if (host_height - sheet.sheet_host_height).abs() <= 0.5 {
                        return;
                    }
                    sheet.sheet_host_height = host_height;
                    let height = sheet.measured_content_height;
                    sheet.adopt_content_height(height, cx);
                    cx.notify();
                });
            }))
            .child(
                BottomSheetUi::new(
                    id,
                    self.is_presented,
                    move |window, cx| content_parts.editor_content(window, cx),
                    move |window, cx| footer_parts.submit_button(window, cx),
                )
                .snap_fractions(vec![resting_fraction, TopicEditorSheetPresentationContract::FULL_HEIGHT_FRACTION])
                .selected_snap_fraction(Some(selected_fraction))
                .experimental_bottom_sheet(true)
                .modal_sized_by_detent(true)
                .experimental_uses_liquid_glass_surface(false)
                .full_page_allows_swipe_dismiss(!is_submitting)
                .dismisses_on_backdrop_tap(true)
                .dismisses_keyboard_before_backdrop_dismiss(true)
                .full_page_shows_drag_indicator(true)
                .keyboard_footer_spacing(TopicEditorSheetPresentationContract::KEYBOARD_FOOTER_SPACING)
                .footer_shows_top_border(false)
                .footer_overlays_content(Some(true))
                .footer_horizontal_padding(Some(15.0))
                .footer_top_padding(Some(10.0))
                .content_scrollable(true)
                .sheet_background(Some(theme.bg._100))
                .footer_background(Some(transparent_black()))
                .on_dismiss(move |_, cx| dismiss_parts.reset_after_dismiss(cx)),
            )
    }
}

fn named_child(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

#[derive(Clone)]
struct TopicEditorSheetParts {
    id: ElementId,
    state: Entity<TopicEditorSheetState>,
    is_presented: Binding<bool>,
    chat_id: String,
    topic_id: Option<String>,
    section_id: Option<String>,
    on_completed: Option<TopicEditorSheetOnCompleted>,
    on_saved: Option<TopicEditorSheetOnSaved>,
}

impl TopicEditorSheetParts {
    fn is_editing(&self) -> bool {
        self.topic_id.as_deref().is_some_and(|topic_id| !topic_id.is_empty())
    }

    fn current_topic(&self, cx: &mut App) -> Option<ChatTopic> {
        let topic_id = self.topic_id.as_deref().filter(|topic_id| !topic_id.is_empty())?;
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.topic_from_any_cache(&self.chat_id, topic_id, cx))
    }

    fn source_channel_kind(&self, cx: &mut App) -> Option<ChatChannelKind> {
        if !self.is_editing() {
            return None;
        }
        self.current_topic(cx).map(|topic| topic.channel_kind)
    }

    fn permission_bits(&self, cx: &mut App) -> i64 {
        let services = ChatServicesStore::shared(cx);
        let scoped = if self.is_editing() {
            services.update(cx, |store, cx| store.effective_permissions(&self.chat_id, self.topic_id.as_deref(), cx))
        } else {
            None
        };
        scoped
            .or_else(|| services.update(cx, |store, cx| store.effective_permissions(&self.chat_id, None, cx)))
            .map_or(0, |permissions| permissions.mask())
    }

    fn allowed_channel_kinds(&self, cx: &mut App) -> Vec<ChatChannelKind> {
        let source = self.source_channel_kind(cx);
        let permissions = self.permission_bits(cx);
        ChatChannelManagementPolicy::allowed_destination_kinds(source, permissions)
    }

    fn can_submit(&self, cx: &mut App) -> bool {
        let allowed = self.allowed_channel_kinds(cx);
        let sheet = self.state.read(cx);
        !sheet.normalized_title().is_empty()
            && sheet.normalized_icon().is_some()
            && (sheet.channel_kind == ChatChannelKind::Text || sheet.draft_voice_config().is_some())
            && allowed.contains(&sheet.channel_kind)
            && !sheet.is_submitting
    }

    fn title_binding(&self) -> Binding<String> {
        Binding::entity(
            self.state.clone(),
            |sheet: &TopicEditorSheetState| sheet.title.clone(),
            |sheet, value, cx| sheet.set_title(value, cx),
        )
    }

    fn editor_content(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let is_editing = self.is_editing();
        let can_submit = self.can_submit(cx);
        let channel_kind = self.state.read(cx).channel_kind;

        let closing = self.clone();
        let submitting = self.clone();
        let measuring = self.state.clone();
        let mut body = vstack(14.0)
            .relative()
            .flex_none()
            .w_full()
            .pb(px(TopicEditorSheetPresentationContract::FOOTER_ALLOWANCE))
            .bg(theme.bg._100)
            .child(
                SheetHeaderBar::new(
                    named_child(&self.id, "header"),
                    if is_editing { tr("topic_editor_edit_title") } else { tr("chat_topics_create_title") },
                    move |window, cx| closing.close(window, cx),
                )
                .right_enabled(can_submit)
                .on_right(move |window, cx| submitting.submit(window, cx)),
            )
            .child(
                GroupedBtns::new(
                    named_child(&self.id, "fields"),
                    vec![
                        GroupedBtn::new("topic_name", tr("topic_settings_name"))
                            .guide_anchor(RiseGuideAnchor::TopicEditorName)
                            .content_slot(self.name_field(&theme, cx))
                            .expanded_slot(self.title_emoji_expansion(cx)),
                        self.icon_row(cx),
                    ],
                )
                .corner_radius(24.0),
            );

        if !is_editing {
            let voice_state = self.state.clone();
            on_change(named_child(&self.id, "picked-kind"), channel_kind, false, window, cx, move |_, value, _, cx| {
                let Some(defaults) = VoiceChannelConfig::defaults(*value) else {
                    return;
                };
                voice_state.update(cx, |sheet, cx| {
                    sheet.voice_draft = VoiceChannelConfigDraft::new(Some(defaults));
                    cx.notify();
                });
            });
            let allowed = self.allowed_channel_kinds(cx);
            body = body.child(ChannelKindPicker::new(
                Binding::entity(
                    self.state.clone(),
                    |sheet: &TopicEditorSheetState| sheet.channel_kind,
                    |sheet, value, _| sheet.channel_kind = value,
                ),
                allowed,
            ));
        }

        if channel_kind.is_rtc() {
            body = body.child(VoiceChannelConfigEditor::new(
                Binding::entity(
                    self.state.clone(),
                    |sheet: &TopicEditorSheetState| sheet.voice_draft.clone(),
                    |sheet, value, _| sheet.voice_draft = value,
                ),
                channel_kind,
            ));
        }

        let error_color = theme.semantic.error_100;
        let save_error = self.state.read(cx).save_error_presence.render_transitioned(
            RiseClock::monotonic(),
            window,
            |_| {
                div()
                    .w_full()
                    .px(px(18.0))
                    .child(MainText::new(tr("topic_editor_save_error")).px(13.0).color(error_color))
            },
        );

        div()
            .w_full()
            .min_h_full()
            .child(body.children(save_error).child(measure(
                named_child(&self.id, "content-height"),
                move |bounds, _, cx| {
                    measuring.update(cx, |sheet, cx| {
                        sheet.adopt_content_height(f32::from(bounds.size.height), cx);
                    });
                },
            )))
            .into_any_element()
    }

    fn name_field(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let (title_emoji_open, focus_generation) = {
            let sheet = self.state.read(cx);
            (sheet.title_emoji_open, sheet.focus_generation)
        };
        let submitting = self.clone();
        let focus_state = self.state.clone();
        let tap_state = self.state.clone();
        let toggling = self.clone();

        let field_id = ElementId::NamedChild(
            Arc::new(self.id.clone()),
            SharedString::from(format!("title-{focus_generation}")),
        );

        hstack(12.0)
            .id(named_child(&self.id, "name-row"))
            .px(px(16.0))
            .min_h(px(56.0))
            .on_click(move |_, _, cx| tap_state.update(cx, |sheet, cx| sheet.request_title_focus(cx)))
            .child(
                div()
                    .flex_none()
                    .size(px(29.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .continuous_corners(8.0)
                    .bg(ChatTileColor::blue())
                    .swiftui_font(Font::system(14.0, FontWeight::SEMIBOLD, FontDesign::Rounded))
                    .text_color(white())
                    .child(TITLE_TILE_GLYPH),
            )
            .child(
                div().flex_1().min_w_0().child(
                    ShapedTextField::new(field_id, tr("chat_topics_create_placeholder"), self.title_binding())
                        .font_size(15.0)
                        .text_color(theme.text.primary)
                        .placeholder_color(theme.text.secondary.opacity(0.82))
                        .auto_focus(focus_generation > 0)
                        .on_focus_change(move |focused, _, cx| {
                            focus_state.update(cx, |sheet, cx| {
                                sheet.title_focused = focused;
                                if focused {
                                    sheet.set_title_emoji_open(false);
                                }
                                cx.notify();
                            });
                        })
                        .on_submit(move |window, cx| {
                            if submitting.can_submit(cx) {
                                submitting.submit(window, cx);
                            }
                        }),
                ),
            )
            .child(
                div()
                    .id(named_child(&self.id, "title-emoji-label"))
                    .tooltip_label(tr("topic_editor_emoji"))
                    .occlude()
                    .flex_none()
                    .child(
                        plain(named_child(&self.id, "title-emoji"))
                            .w(px(30.0))
                            .h(px(34.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_press(move |_, cx| toggling.toggle_title_emoji(cx))
                            .children(IconUi::sized(
                                if title_emoji_open { "keyboard" } else { "face.smiling" },
                                px(19.0),
                                theme.text.secondary,
                                false,
                            )),
                    ),
            )
            .into_any_element()
    }

    fn title_emoji_expansion(&self, cx: &App) -> SectionRevealUi {
        let sheet = self.state.read(cx);
        let is_expanded = sheet.title_emoji_open;
        let expansion_height = sheet.emoji_picker_expansion_height();
        let viewport_height = sheet.emoji_picker_viewport_height();
        let picking = self.state.clone();
        let picker = EmojiPickerUi::new(named_child(&self.id, "title-emoji-picker"), move |selected, _, cx| {
            let Some(selected) = EmojiPickerCatalog::normalized_selectable_emoji(Some(selected.as_str())) else {
                return;
            };
            picking.update(cx, |sheet, cx| sheet.insert_title_emoji(&selected, cx));
        })
        .chrome(false)
        .shows_custom_emoji(false);
        SectionRevealUi::new(
            named_child(&self.id, "title-emoji-reveal"),
            is_expanded,
            expansion_height,
            div().w_full().px(px(8.0)).pb(px(8.0)).child(div().w_full().h(px(viewport_height)).child(picker)),
        )
        .animated(false)
    }

    fn icon_row(&self, cx: &App) -> GroupedBtn {
        let icon = self.state.read(cx).icon.clone();
        let toggling = self.clone();
        GroupedBtn::new("topic_icon", tr("topic_editor_emoji"))
            .icon("face.smiling.fill")
            .icon_background(ChatTileColor::orange())
            .trailing_slot(
                div()
                    .size(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(24.0))
                    .child(TopicIconGlyph::new(icon).size(22.0)),
            )
            .show_chevron_when_trailing_slot(true)
            .expanded_slot(self.emoji_picker_expansion(cx))
            .callback(move |_, cx| toggling.toggle_emoji_picker(cx))
    }

    fn emoji_picker_expansion(&self, cx: &App) -> SectionRevealUi {
        let sheet = self.state.read(cx);
        let is_expanded = sheet.emoji_picker_open;
        let expansion_height = sheet.emoji_picker_expansion_height();
        let viewport_height = sheet.emoji_picker_viewport_height();
        let picking = self.state.clone();
        let picking_custom = self.state.clone();
        let picker = EmojiPickerUi::new(named_child(&self.id, "emoji-picker"), move |selected, _, cx| {
            let Some(selected) = EmojiPickerCatalog::normalized_selectable_emoji(Some(selected.as_str())) else {
                return;
            };
            adopt_icon(&picking, selected, cx);
        })
        .chrome(false)
        .shows_custom_emoji(true)
        .on_pick_custom(move |item: StickerItem, _, cx| {
            adopt_icon(&picking_custom, format!("{}{}", MessageReaction::CUSTOM_EMOJI_PREFIX, item.id), cx);
        });
        SectionRevealUi::new(
            named_child(&self.id, "emoji-reveal"),
            is_expanded,
            expansion_height,
            div().w_full().px(px(8.0)).pb(px(8.0)).child(div().w_full().h(px(viewport_height)).child(picker)),
        )
        .animated(false)
    }

    fn submit_button(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let can_submit = self.can_submit(cx);
        let is_submitting = self.state.read(cx).is_submitting;
        let title = if self.is_editing() { tr("common_done") } else { tr("chat_topics_create") };
        let submitting = self.clone();
        div()
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .when(!can_submit, |bar| bar.child(div().absolute().inset_0().rounded_full().bg(theme.bg._300)))
            .child(
                LiquidGlassButton::new(named_child(&self.id, "submit"), LiquidGlassButtonShape::Capsule)
                    .horizontal_padding(22.0)
                    .vertical_padding(12.0)
                    .tint(Some(theme.primary._100))
                    .haptic_on_press(can_submit)
                    .disabled(!can_submit)
                    .accessibility_label(Some(title.clone()))
                    .on_press(move |window, cx| submitting.submit(window, cx))
                    .child(
                        hstack(9.0)
                            .flex_1()
                            .justify_center()
                            .when(is_submitting, |label| {
                                label.child(progress_view().control_size(ControlSize::Small).tint(theme.content_on(theme.primary._100)))
                            })
                            .child(MainText::new(title).px(16.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100))),
                    ),
            )
            .guide_anchor(RiseGuideAnchor::TopicEditorCreate)
            .into_any_element()
    }

    fn prepare_draft(&self, cx: &mut App) {
        let topic = self.current_topic(cx);
        let allowed = self.allowed_channel_kinds(cx);
        let is_new = topic.is_none();
        self.state.update(cx, |sheet, cx| {
            sheet.title = topic.as_ref().map(|topic| topic.title.clone()).unwrap_or_default();
            sheet.icon = ChatTopic::selectable_icon(topic.as_ref().map(|topic| topic.icon.as_str()))
                .unwrap_or_else(|| ChatTopic::DEFAULT_ICON.to_owned());
            sheet.channel_kind = topic.as_ref().map_or(ChatChannelKind::Text, |topic| topic.channel_kind);
            if is_new
                && !allowed.contains(&sheet.channel_kind)
                && let Some(first) = allowed.first()
            {
                sheet.channel_kind = *first;
            }
            sheet.voice_draft = VoiceChannelConfigDraft::new(
                topic
                    .as_ref()
                    .and_then(|topic| topic.voice_config.clone())
                    .or_else(|| VoiceChannelConfig::defaults(sheet.channel_kind)),
            );
            sheet.emoji_picker_open = false;
            sheet.title_emoji_open = false;
            sheet.is_submitting = false;
            sheet.set_save_error_visible(false, cx);
            sheet.completed_topic = None;
            sheet.title_focused = false;
            sheet.focus_generation = 0;
            sheet.selected_fraction = sheet.resting_fraction;
            cx.notify();
        });
    }

    fn adopt_allowed_channel_kinds(&self, kinds: &[ChatChannelKind], cx: &mut App) {
        if self.is_editing() {
            return;
        }
        let Some(first) = kinds.first().copied() else {
            return;
        };
        self.state.update(cx, |sheet, cx| {
            if kinds.contains(&sheet.channel_kind) {
                return;
            }
            sheet.channel_kind = first;
            if let Some(defaults) = VoiceChannelConfig::defaults(first) {
                sheet.voice_draft = VoiceChannelConfigDraft::new(Some(defaults));
            }
            cx.notify();
        });
    }

    fn toggle_emoji_picker(&self, cx: &mut App) {
        let _ = selection_changed();
        KeyboardDismiss::perform(cx);
        self.state.update(cx, |sheet, cx| {
            sheet.title_focused = false;
            sheet.set_title_emoji_open(false);
            sheet.emoji_picker_open = !sheet.emoji_picker_open;
            sheet.settle_detent();
            cx.notify();
        });
    }

    fn toggle_title_emoji(&self, cx: &mut App) {
        let _ = selection_changed();
        if self.state.read(cx).title_emoji_open {
            self.state.update(cx, |sheet, cx| {
                sheet.set_title_emoji_open(false);
                sheet.request_title_focus(cx);
            });
        } else {
            KeyboardDismiss::perform(cx);
            self.state.update(cx, |sheet, cx| {
                sheet.title_focused = false;
                sheet.emoji_picker_open = false;
                sheet.set_title_emoji_open(true);
                sheet.settle_detent();
                cx.notify();
            });
        }
    }

    fn submit(&self, window: &mut Window, cx: &mut App) {
        if !self.can_submit(cx) {
            return;
        }
        let (title, icon, channel_kind, voice_config) = {
            let sheet = self.state.read(cx);
            let Some(icon) = sheet.normalized_icon() else {
                return;
            };
            (sheet.normalized_title().to_owned(), icon, sheet.channel_kind, sheet.draft_voice_config())
        };
        let window_handle = window.window_handle();
        let finishing = self.clone();
        let on_result: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |success: bool, cx: &mut App| {
            let finishing = finishing.clone();
            cx.defer(move |cx| finishing.finish(success, window_handle, cx));
        });

        if self.is_editing() {
            let Some(topic_id) = self.topic_id.clone().filter(|topic_id| !topic_id.is_empty()) else {
                return;
            };
            let Some(current) = self.current_topic(cx) else {
                self.state.update(cx, |sheet, cx| {
                    sheet.set_save_error_visible(true, cx);
                    cx.notify();
                });
                return;
            };
            if current.title == title && current.icon == icon && current.voice_config == voice_config {
                self.finish(true, window_handle, cx);
                return;
            }
            self.begin_submitting(cx);
            let chat_id = self.chat_id.clone();
            ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.update_topic(
                    &chat_id,
                    &topic_id,
                    Some(title.as_str()),
                    Some(icon.as_str()),
                    None,
                    voice_config,
                    Some(on_result),
                    cx,
                );
            });
        } else {
            self.begin_submitting(cx);
            let created_state = self.state.clone();
            let on_created: Rc<dyn Fn(ChatTopic, &mut App)> = Rc::new(move |topic: ChatTopic, cx: &mut App| {
                let created_state = created_state.clone();
                cx.defer(move |cx| {
                    created_state.update(cx, |sheet, _| sheet.completed_topic = Some(topic));
                });
            });
            let chat_id = self.chat_id.clone();
            let section_id = self.section_id.clone();
            ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.create_topic(
                    &chat_id,
                    &title,
                    &icon,
                    section_id.as_deref(),
                    channel_kind,
                    voice_config,
                    Some(on_created),
                    Some(on_result),
                    cx,
                );
            });
        }
    }

    fn load_effective_permissions(&self, cx: &mut App) {
        let topic_id = if self.is_editing() { self.topic_id.clone() } else { None };
        let chat_id = self.chat_id.clone();
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_effective_permissions(&chat_id, topic_id.as_deref(), false, cx);
        });
    }

    fn begin_submitting(&self, cx: &mut App) {
        KeyboardDismiss::perform(cx);
        self.state.update(cx, |sheet, cx| {
            sheet.title_focused = false;
            sheet.set_save_error_visible(false, cx);
            sheet.is_submitting = true;
            cx.notify();
        });
    }

    fn finish(&self, success: bool, window_handle: gpui::AnyWindowHandle, cx: &mut App) {
        self.state.update(cx, |sheet, cx| {
            sheet.is_submitting = false;
            if !success {
                sheet.set_save_error_visible(true, cx);
            }
            cx.notify();
        });

        if !success {
            if let Some(on_completed) = &self.on_completed {
                on_completed(false, cx);
            }
            return;
        }

        let saved_topic = self
            .topic_id
            .as_deref()
            .and_then(|topic_id| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.topic_from_any_cache(&self.chat_id, topic_id, cx))
            })
            .or_else(|| self.state.read(cx).completed_topic.clone());
        if let (Some(saved_topic), Some(on_saved)) = (saved_topic, &self.on_saved) {
            on_saved(saved_topic, cx);
        }
        if let Some(on_completed) = &self.on_completed {
            on_completed(true, cx);
        }
        let closing = self.is_presented.clone();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| closing.set(false, window, cx));
        });
    }

    fn close(&self, window: &mut Window, cx: &mut App) {
        if self.state.read(cx).is_submitting {
            return;
        }
        KeyboardDismiss::perform(cx);
        self.state.update(cx, |sheet, cx| {
            sheet.title_focused = false;
            cx.notify();
        });
        self.is_presented.set(false, window, cx);
    }

    fn reset_after_dismiss(&self, cx: &mut App) {
        self.state.update(cx, |sheet, cx| {
            sheet.title = String::new();
            sheet.icon = ChatTopic::DEFAULT_ICON.to_owned();
            sheet.emoji_picker_open = false;
            sheet.title_emoji_open = false;
            sheet.is_submitting = false;
            sheet.set_save_error_visible(false, cx);
            sheet.completed_topic = None;
            sheet.title_focused = false;
            sheet.channel_kind = ChatChannelKind::Text;
            sheet.voice_draft = VoiceChannelConfigDraft::new(None);
            cx.notify();
        });
    }
}

fn adopt_icon(state: &Entity<TopicEditorSheetState>, icon: String, cx: &mut App) {
    state.update(cx, |sheet, cx| {
        sheet.icon = icon;
        sheet.set_save_error_visible(false, cx);
        sheet.emoji_picker_open = false;
        cx.notify();
    });
}

pub enum TopicEditorSheetPresentationContract {}

impl TopicEditorSheetPresentationContract {
    pub const FULL_HEIGHT_FRACTION: f32 = 1.0;
    pub const KEYBOARD_FOOTER_SPACING: f32 = 12.0;

    pub const MINIMUM_FRACTION: f32 = 0.42;

    pub const MAXIMUM_RESTING_FRACTION: f32 = 0.9;

    pub const FOOTER_ALLOWANCE: f32 = 92.0;

    /// the standard sheet stacks the handle strip above the content
    /// so the resting height must include it or the last row ends under the footer
    pub fn handle_zone() -> f32 {
        BottomSheetMetrics::HANDLE_TOP_PADDING
            + BottomSheetMetrics::HANDLE_HEIGHT
            + BottomSheetMetrics::HANDLE_BOTTOM_PADDING
    }

    /// host fraction for the measured content height
    /// kept separate since it is the single place deciding sheet size and a contract test checks it
    pub fn resting_fraction(content_height: f32, host_height: f32) -> f32 {
        let available = host_height.max(1.0);
        let fraction = (content_height.max(0.0) + Self::handle_zone() + Self::KEYBOARD_FOOTER_SPACING) / available;
        Self::MAXIMUM_RESTING_FRACTION.min(Self::MINIMUM_FRACTION.max(fraction))
    }
}
