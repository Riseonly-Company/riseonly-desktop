use std::rc::Rc;

use gpui::{App, Axis, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, RenderOnce, SharedString, Window, white};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::{AppTheme, color};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::switch_ui::SwitchUi;
use rise_ui::{IconUi, MainText, TextAlignT, page_header_toolbar_button::PageHeaderMetrics};

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::input_field_accessory::InputFieldAccessory;
use crate::core::ui::shaped_text_field::ShapedTextField;
use crate::modules::chat::shared::poll_publicity::PollPublicity;
use crate::modules::chat::stores::message::message_actions::message_poll_codec::{MessagePollCodec, MessagePollDraft};
use crate::modules::media_picker::ui::media_picker_header_icon_button::MediaPickerHeaderIconButton;

const OPTION_ROW_ANIMATION_SECONDS: f64 = 0.2;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct PollOptionField {
    id: Uuid,
    text: String,
}

impl PollOptionField {
    fn new() -> Self {
        Self { id: Uuid::new_v4(), text: String::new() }
    }
}

/// one option on screen
/// a removed row stays drawn until it has faded
struct PollOptionRow {
    field: PollOptionField,
    presence: Presence<()>,
}

struct MediaPickerPollComposerState {
    question: String,
    rows: Vec<PollOptionRow>,
    is_anonymous: bool,
    allows_multiple: bool,
    is_quiz: bool,
    correct_option_id: Option<Uuid>,
}

impl MediaPickerPollComposerState {
    fn new() -> Self {
        let row = || PollOptionRow {
            field: PollOptionField::new(),
            presence: Presence::new(Some(())).with_transition(Transition::opacity()),
        };
        Self {
            question: String::new(),
            rows: vec![row(), row()],
            is_anonymous: true,
            allows_multiple: false,
            is_quiz: false,
            correct_option_id: None,
        }
    }

    /// options, a fading row has already left the array
    fn options(&self) -> impl Iterator<Item = &PollOptionField> {
        self.rows.iter().filter(|row| row.presence.is_present()).map(|row| &row.field)
    }

    fn options_count(&self) -> usize {
        self.options().count()
    }

    fn filled_options(&self) -> Vec<&PollOptionField> {
        self.options()
            .filter(|option| !trimming_whitespaces_and_newlines(&option.text).is_empty())
            .collect()
    }

    fn can_create(&self) -> bool {
        let filled = self.filled_options();
        !trimming_whitespaces_and_newlines(&self.question).is_empty()
            && filled.len() >= MessagePollCodec::MIN_OPTIONS
            && (!self.is_quiz || filled.iter().any(|option| Some(option.id) == self.correct_option_id))
    }

    fn append_option(&mut self, cx: &mut Context<Self>) {
        let mut presence = Presence::new(None).with_transition(Transition::opacity());
        presence.set_in(Some(()), ease_out(OPTION_ROW_ANIMATION_SECONDS), RiseClock::monotonic(), cx);
        self.rows.push(PollOptionRow { field: PollOptionField::new(), presence });
        cx.notify();
    }

    fn remove_option(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        for row in self.rows.iter_mut().filter(|row| row.field.id == id) {
            row.presence.set_in(None, ease_out(OPTION_ROW_ANIMATION_SECONDS), now, cx);
        }
        if self.correct_option_id == Some(id) {
            self.correct_option_id = None;
        }
        cx.notify();
    }

    fn set_option_text(&mut self, id: Uuid, value: String, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.field.id == id) {
            row.field.text = value;
            cx.notify();
        }
    }

    fn drop_finished_rows(&mut self) {
        let now = RiseClock::monotonic();
        self.rows.retain(|row| row.presence.is_present() || row.presence.is_running(now));
    }

    fn draft(&self) -> Option<MessagePollDraft> {
        if !self.can_create() {
            return None;
        }
        let kept = self.filled_options();
        Some(MessagePollDraft {
            question: self.question.clone(),
            options: kept.iter().map(|option| option.text.clone()).collect(),
            is_anonymous: self.is_anonymous,
            allows_multiple: !self.is_quiz && self.allows_multiple,
            quiz_correct_index: if self.is_quiz {
                kept.iter().position(|option| Some(option.id) == self.correct_option_id)
            } else {
                None
            },
        })
    }
}

#[derive(IntoElement)]
pub struct MediaPickerPollComposer {
    id: ElementId,
    allows_public: bool,
    pub on_cancel: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_create: Rc<dyn Fn(MessagePollDraft, &mut Window, &mut App)>,
}

impl MediaPickerPollComposer {
    pub fn new(
        id: impl Into<ElementId>,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
        on_create: impl Fn(MessagePollDraft, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            allows_public: true,
            on_cancel: Rc::new(on_cancel),
            on_create: Rc::new(on_create),
        }
    }

    pub fn allows_public(mut self, allows_public: bool) -> Self {
        self.allows_public = allows_public;
        self
    }

    fn header(&self, state: &Entity<MediaPickerPollComposerState>, theme: &AppTheme, cx: &App) -> Div {
        let can_create = state.read(cx).can_create();
        let on_cancel = self.on_cancel.clone();
        let on_create = self.on_create.clone();
        let creating = state.clone();
        let allows_public = self.allows_public;
        let create_label = div()
            .flex()
            .items_center()
            .px(px(6.0))
            .h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .child(
                MainText::new(tr("chat_poll_create"))
                    .px(16.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(if can_create { theme.primary._100 } else { theme.text.secondary }),
            );
        div().w_full().px(px(14.0)).py(px(10.0)).bg(theme.bg._100).child(
            zstack(Alignment::Center)
                .w_full()
                .h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
                .child(
                    MainText::new(tr("chat_poll_new"))
                        .px(17.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(theme.text.primary),
                )
                .child(
                    hstack(0.0)
                        .w_full()
                        .child(
                            MediaPickerHeaderIconButton::new("poll-composer-cancel", "xmark", move |window, cx| {
                                on_cancel(window, cx)
                            })
                            .font_size(15.0),
                        )
                        .child(spacer())
                        .child(
                            plain("poll-composer-create")
                                .disabled(!can_create)
                                .on_press(move |window, cx| Self::create(&creating, allows_public, &on_create, window, cx))
                                .child(create_label),
                        ),
                ),
        )
    }

    fn question_item(state: &Entity<MediaPickerPollComposerState>, theme: &AppTheme) -> GroupedBtn {
        let text = Binding::entity(
            state.clone(),
            |state: &MediaPickerPollComposerState| state.question.clone(),
            |state, value, _| state.question = value,
        );
        let rule = MessagePollCodec::question_rule();
        GroupedBtn::new("question", "").content_slot(Self::field_row(
            "poll-question",
            tr("chat_poll_question_placeholder"),
            rule.binding(text),
            rule,
            Axis::Vertical,
            theme,
            Some(("questionmark", color("#008DE5"))),
        ))
    }

    fn option_items(state: &Entity<MediaPickerPollComposerState>, theme: &AppTheme, window: &Window, cx: &App) -> Vec<GroupedBtn> {
        let now = RiseClock::monotonic();
        let composer = state.read(cx);
        let mut items: Vec<GroupedBtn> = composer
            .rows
            .iter()
            .map(|row| {
                let id = row.field.id;
                let content = row
                    .presence
                    .render_transitioned(now, window, |_| Self::option_row(state, id, theme, cx));
                GroupedBtn::new(SharedString::from(id.uuid_string()), "").content_slot(div().w_full().children(content))
            })
            .collect();
        if composer.options_count() < MessagePollCodec::max_options() {
            let adding = state.clone();
            items.push(
                GroupedBtn::new("add_option", tr("chat_poll_add_answer"))
                    .icon("plus")
                    .icon_background(theme.primary._100)
                    .show_chevron(false)
                    .callback(move |_, cx| adding.update(cx, |state, cx| state.append_option(cx))),
            );
        }
        items
    }

    fn option_row(state: &Entity<MediaPickerPollComposerState>, id: Uuid, theme: &AppTheme, cx: &App) -> Div {
        let composer = state.read(cx);
        let is_quiz = composer.is_quiz;
        let is_correct = composer.correct_option_id == Some(id);
        let reader = state.clone();
        let writer = state.clone();
        let text = Binding::new(
            move |cx| {
                reader
                    .read(cx)
                    .rows
                    .iter()
                    .find(|row| row.field.id == id)
                    .map(|row| row.field.text.clone())
                    .unwrap_or_default()
            },
            move |value, _, cx| writer.update(cx, |state, cx| state.set_option_text(id, value, cx)),
        );
        let rule = MessagePollCodec::option_rule();
        let key = id.uuid_string();
        let choosing = state.clone();
        let removing = state.clone();
        hstack(10.0)
            .w_full()
            .when(is_quiz, |row| row.pl(px(10.0)))
            .pr(px(10.0))
            .when(is_quiz, |row| {
                row.child(
                    plain(ElementId::Name(format!("poll-correct-{key}").into()))
                        .on_press(move |_, cx| {
                            choosing.update(cx, |state, cx| {
                                state.correct_option_id = Some(id);
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(30.0))
                                .children(IconUi::sized(
                                    if is_correct { "checkmark.circle.fill" } else { "circle" },
                                    px(20.0),
                                    if is_correct { theme.semantic.success_100 } else { theme.text.secondary },
                                    is_correct,
                                )),
                        ),
                )
            })
            .child(Self::field_row(
                format!("poll-answer-{key}"),
                tr("chat_poll_answer_placeholder"),
                rule.binding(text),
                rule,
                Axis::Horizontal,
                theme,
                if is_quiz { None } else { Some(("list.bullet", color("#7E57F7"))) },
            ))
            .when(composer.options_count() > MessagePollCodec::MIN_OPTIONS, |row| {
                row.child(
                    plain(ElementId::Name(format!("poll-remove-{key}").into()))
                        .on_press(move |_, cx| removing.update(cx, |state, cx| state.remove_option(id, cx)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(30.0))
                                .children(IconUi::sized("minus.circle.fill", px(20.0), theme.semantic.error_100, true)),
                        ),
                )
            })
    }

    fn field_icon(system_name: &str, color: Hsla) -> Div {
        div()
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .size(px(29.0))
            .continuous_corners(8.0)
            .bg(color)
            .children(IconUi::sized(system_name, px(14.0), white(), true))
    }

    #[allow(clippy::too_many_arguments)]
    fn field_row(
        id: impl Into<SharedString>,
        placeholder: String,
        text: Binding<String>,
        rule: RiseFieldRule,
        axis: Axis,
        theme: &AppTheme,
        icon: Option<(&str, Hsla)>,
    ) -> MediaPickerPollComposerFieldRow {
        MediaPickerPollComposerFieldRow {
            id: id.into(),
            placeholder,
            text,
            rule,
            axis,
            text_primary: theme.text.primary,
            icon: icon.map(|(symbol, color)| Self::field_icon(symbol, color)),
        }
    }

    fn mode_items(state: &Entity<MediaPickerPollComposerState>, allows_public: bool, cx: &App) -> Vec<GroupedBtn> {
        let mut items = Vec::new();
        if allows_public {
            let anonymous = Binding::entity(
                state.clone(),
                |state: &MediaPickerPollComposerState| state.is_anonymous,
                |state, value, _| state.is_anonymous = value,
            );
            items.push(
                GroupedBtn::new("anonymous", tr("chat_poll_anonymous_voting"))
                    .icon("eye.slash.fill")
                    .icon_background(color("#5856D6"))
                    .show_chevron(false)
                    .trailing_slot(SwitchUi::new("poll-anonymous", anonymous)),
            );
        }
        if !state.read(cx).is_quiz {
            let multiple = Binding::entity(
                state.clone(),
                |state: &MediaPickerPollComposerState| state.allows_multiple,
                |state, value, _| state.allows_multiple = value,
            );
            items.push(
                GroupedBtn::new("multiple", tr("chat_poll_multiple_answers"))
                    .icon("checklist")
                    .icon_background(color("#34C759"))
                    .show_chevron(false)
                    .trailing_slot(SwitchUi::new("poll-multiple", multiple)),
            );
        }
        let quiz = Binding::entity(
            state.clone(),
            |state: &MediaPickerPollComposerState| state.is_quiz,
            |state, value, _| {
                state.is_quiz = value;
                if value {
                    state.allows_multiple = false;
                } else {
                    state.correct_option_id = None;
                }
            },
        );
        items.push(
            GroupedBtn::new("quiz", tr("chat_poll_quiz_mode"))
                .icon("checkmark.seal.fill")
                .icon_background(color("#FF9500"))
                .show_chevron(false)
                .trailing_slot(SwitchUi::new("poll-quiz", quiz)),
        );
        items
    }

    fn create(
        state: &Entity<MediaPickerPollComposerState>,
        allows_public: bool,
        on_create: &Rc<dyn Fn(MessagePollDraft, &mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(mut draft) = state.read(cx).draft() {
            draft.is_anonymous = draft.is_anonymous || !allows_public;
            on_create(draft, window, cx);
        }
    }
}

impl RenderOnce for MediaPickerPollComposer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| MediaPickerPollComposerState::new());
        state.update(cx, |state, _| state.drop_finished_rows());
        let is_quiz = state.read(cx).is_quiz;

        let answers = GroupedBtns::new("poll-answers", Self::option_items(&state, &theme, window, cx))
            .title(tr("chat_poll_answers"))
            .uses_lazy_rows(false);
        let answers = if is_quiz { answers.end_group_title(tr("chat_poll_quiz_hint")) } else { answers };
        let modes = GroupedBtns::new("poll-modes", Self::mode_items(&state, self.allows_public, cx)).uses_lazy_rows(false);
        let modes = if self.allows_public { modes } else { modes.end_group_title(PollPublicity::anonymous_only_hint()) };

        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(self.header(&state, &theme, cx))
            .child(
                div().flex_1().min_h_0().w_full().child(
                    scroll_view("poll-composer-scroll").child(
                        vstack(18.0)
                            .w_full()
                            .pt(px(10.0))
                            .pb(px(40.0))
                            .child(
                                GroupedBtns::new("poll-question-group", vec![Self::question_item(&state, &theme)])
                                    .title(tr("chat_poll_question"))
                                    .uses_lazy_rows(false),
                            )
                            .child(answers)
                            .child(modes),
                    ),
                ),
            )
    }
}

#[derive(IntoElement)]
struct MediaPickerPollComposerFieldRow {
    id: SharedString,
    placeholder: String,
    text: Binding<String>,
    rule: RiseFieldRule,
    axis: Axis,
    text_primary: Hsla,
    icon: Option<Div>,
}

impl RenderOnce for MediaPickerPollComposerFieldRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_vertical = self.axis == Axis::Vertical;
        let remaining = self.rule.remaining(&self.text.get(cx));
        let field = ShapedTextField::new(ElementId::Name(self.id.clone()), self.placeholder, self.text)
            .axis(self.axis)
            .font_size(15.0);
        let field = if is_vertical { field.max_visible_lines(5) } else { field.max_visible_lines(1) };
        let row = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_w_0()
            .gap(px(12.0))
            .px(px(16.0))
            .py(px(12.0))
            .min_h(px(50.0))
            .text_color(self.text_primary);
        let row = if is_vertical { row.items_start() } else { row.items_center() };
        row.when_some(self.icon, |row, icon| {
            row.child(if is_vertical { icon.mt(px(2.0)) } else { icon })
        })
        .child(field)
        .child(InputFieldAccessory::new(
            ElementId::NamedChild(std::sync::Arc::new(ElementId::Name(self.id)), "accessory".into()),
            remaining,
            None,
        ))
    }
}
