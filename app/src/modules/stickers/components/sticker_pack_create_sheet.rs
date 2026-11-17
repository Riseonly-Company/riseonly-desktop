use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, FontWeight, SharedString, Subscription, Window, transparent_black};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_text::character_count;
use rise_i18n::tr;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::error_text_ui::ErrorTextUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::input_ui::input_ui_state::{InputUiEvent, InputUiState};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;
use rise_ui::layer_keys::LayerKeysExt;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;

const INITIAL_FRACTION: f32 = 0.34;
const MINIMUM_FRACTION: f32 = 0.27;
const MAXIMUM_FRACTION: f32 = 0.58;
const CONTENT_FRACTION_CHROME: f32 = 18.0;
const FRACTION_EPSILON: f32 = 0.005;

pub type StickerPackCreateSheetOnCreate = Rc<dyn Fn(String, &mut Window, &mut App)>;

fn title_rule() -> RiseFieldRule {
    RiseFieldRule::new("stickers_create_pack", "/title")
}

#[derive(Clone)]
struct StickerPackCreateSheetSubmit {
    is_submitting: bool,
    on_create: StickerPackCreateSheetOnCreate,
}

/// state of the create sheet, no title focus twin since desktop has no on screen keyboard
struct StickerPackCreateSheetState {
    title: String,
    field: Entity<InputUiState>,
    submission_locked: bool,
    resting_fraction: f32,
    selected_fraction: f32,
    submit: Option<StickerPackCreateSheetSubmit>,
    _events: Subscription,
}

impl StickerPackCreateSheetState {
    fn new(kind: PackKind, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(InputUiState::single_line);
        field.update(cx, |field, cx| {
            field.set_chromeless(true, cx);
            field.set_placeholder(
                tr(if kind == PackKind::Emoji { "emoji_pack_name_placeholder" } else { "sticker_pack_name_placeholder" }),
                cx,
            );
        });
        let events = cx.subscribe_in(&field, window, |this: &mut Self, field, event, window, cx| match event {
            InputUiEvent::Changed => {
                let value = field.read(cx).text().to_owned();
                let clamped = title_rule().clamped(&value);
                if clamped != value {
                    field.update(cx, |field, cx| field.set_text(&clamped, cx));
                }
                this.title = clamped;
                cx.notify();
            }
            InputUiEvent::Submitted => this.submit(window, cx),
            InputUiEvent::Cancelled => ScreenFocus::release_text_field(window, cx),
        });
        Self {
            title: String::new(),
            field,
            submission_locked: false,
            resting_fraction: INITIAL_FRACTION,
            selected_fraction: INITIAL_FRACTION,
            submit: None,
            _events: events,
        }
    }

    fn trimmed_title(&self) -> String {
        title_rule().schema.as_ref().map_or_else(|| self.title.clone(), |schema| schema.normalize(&self.title).to_string())
    }

    fn can_create(&self, is_submitting: bool) -> bool {
        !self.submission_locked && !is_submitting && title_rule().is_satisfied(&self.title)
    }

    fn title_error_text(&self, error_text: Option<&str>) -> Option<String> {
        if let Some(error_text) = error_text.filter(|error_text| !error_text.is_empty()) {
            return Some(error_text.to_owned());
        }
        if self.trimmed_title().is_empty() {
            return None;
        }
        title_rule().error_text(&self.title)
    }

    fn update_resting_fraction(&mut self, content_height: f32, window: &Window, cx: &mut Context<Self>) {
        let screen_height = f32::from(window.viewport_size().height).max(1.0);
        let measured =
            MAXIMUM_FRACTION.min(MINIMUM_FRACTION.max((content_height + CONTENT_FRACTION_CHROME) / screen_height));
        if (measured - self.resting_fraction).abs() <= FRACTION_EPSILON {
            return;
        }
        self.resting_fraction = measured;
        self.selected_fraction = measured;
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(submit) = self.submit.clone() else {
            return;
        };
        if !self.can_create(submit.is_submitting) {
            return;
        }
        self.submission_locked = true;
        cx.notify();
        (submit.on_create)(self.trimmed_title(), window, cx);
    }
}

#[derive(IntoElement)]
pub struct StickerPackCreateSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    kind: PackKind,
    is_submitting: bool,
    error_text: Option<String>,
    on_create: StickerPackCreateSheetOnCreate,
}

impl StickerPackCreateSheet {
    pub fn new(
        is_presented: Binding<bool>,
        kind: PackKind,
        is_submitting: bool,
        error_text: Option<String>,
        on_create: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: ElementId::Name(SharedString::from(format!("sticker-pack-create-sheet-{}", kind.wire()))),
            is_presented,
            kind,
            is_submitting,
            error_text,
            on_create: Rc::new(on_create),
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

impl RenderOnce for StickerPackCreateSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let kind = self.kind;
        let state = window.use_keyed_state(self.child_id("state"), cx, move |window, cx| {
            StickerPackCreateSheetState::new(kind, window, cx)
        });
        let submit =
            StickerPackCreateSheetSubmit { is_submitting: self.is_submitting, on_create: self.on_create.clone() };
        state.update(cx, |state, _| state.submit = Some(submit));

        let field = state.read(cx).field.clone();
        field.update(cx, |field, cx| {
            field.set_text_color(Some(theme.text.primary), cx);
            field.set_placeholder_color(Some(theme.text.secondary.opacity(0.82)), cx);
        });

        let opening = state.clone();
        on_change(self.child_id("presented"), self.is_presented.get(cx), false, window, cx, move |_, open, _, cx| {
            if *open {
                opening.update(cx, |state, cx| {
                    state.title = String::new();
                    state.submission_locked = false;
                    state.selected_fraction = state.resting_fraction;
                    state.field.update(cx, |field, cx| field.clear(cx));
                    cx.notify();
                });
            }
        });
        let finishing = state.clone();
        let finishing_presented = self.is_presented.clone();
        on_change(
            self.child_id("submitting"),
            self.is_submitting,
            false,
            window,
            cx,
            move |was_submitting, submitting, _, cx| {
                if *was_submitting && !*submitting && finishing_presented.get(cx) {
                    finishing.update(cx, |state, cx| {
                        state.submission_locked = false;
                        cx.notify();
                    });
                }
            },
        );
        let failing = state.clone();
        on_change(self.child_id("error"), self.error_text.clone(), false, window, cx, move |_, error, _, cx| {
            if error.as_deref().is_some_and(|error| !error.is_empty()) {
                failing.update(cx, |state, cx| {
                    state.submission_locked = false;
                    cx.notify();
                });
            }
        });

        let resting_fraction = state.read(cx).resting_fraction;
        let selected_fraction = Binding::entity(
            state.clone(),
            |state: &StickerPackCreateSheetState| state.selected_fraction,
            |state, value, cx| {
                state.selected_fraction = value;
                cx.notify();
            },
        );
        let parts = StickerPackCreateSheetParts {
            id: self.id.clone(),
            state,
            is_presented: self.is_presented.clone(),
            kind: self.kind,
            is_submitting: self.is_submitting,
            error_text: self.error_text.clone(),
        };
        let content_parts = parts.clone();
        let footer_parts = parts;

        BottomSheetUi::new(
            self.id.clone(),
            self.is_presented,
            move |_, cx| content_parts.content(cx),
            move |_, cx| footer_parts.create_button(cx),
        )
        .snap_fractions(vec![resting_fraction, 1.0])
        .selected_snap_fraction(Some(selected_fraction))
        .experimental_bottom_sheet(true)
        .experimental_uses_liquid_glass_surface(false)
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(true))
        .footer_horizontal_padding(Some(14.))
        .footer_top_padding(Some(8.))
        .content_scrollable(false)
        .modal_sized_by_detent(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(transparent_black()))
    }
}

#[derive(Clone)]
struct StickerPackCreateSheetParts {
    id: ElementId,
    state: Entity<StickerPackCreateSheetState>,
    is_presented: Binding<bool>,
    kind: PackKind,
    is_submitting: bool,
    error_text: Option<String>,
}

impl StickerPackCreateSheetParts {
    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn guide_anchor_for(&self, anchor: RiseGuideAnchor) -> Option<RiseGuideAnchor> {
        (self.kind == PackKind::Sticker).then_some(anchor)
    }

    fn content(&self, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let closing = self.is_presented.clone();
        let (field, count, title_error_text) = {
            let state = self.state.read(cx);
            (
                state.field.clone(),
                character_count(&state.title),
                state.title_error_text(self.error_text.as_deref()),
            )
        };
        let maximum = title_rule().maximum().unwrap_or(0);
        let measuring = self.state.clone();
        let can_create = self.state.read(cx).can_create(self.is_submitting);
        let submitting = self.state.clone();

        vstack(18.)
            .relative()
            .w_full()
            .form_primary(can_create, move |window, cx| {
                submitting.update(cx, |state, cx| state.submit(window, cx));
            })
            .px(px(16.))
            .pt(px(4.))
            .pb(px(92.))
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(
                        PageHeaderIconButton::new(self.child_id("close"), "xmark", move |window, cx| {
                            closing.set(false, window, cx);
                        })
                        .font_size(14.),
                    )
                    .child(
                        div().absolute().inset_0().flex().items_center().justify_center().child(
                            MainText::new(tr(if self.kind == PackKind::Emoji {
                                "emoji_pack_create_title"
                            } else {
                                "sticker_pack_create_title"
                            }))
                            .px(17.)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD),
                        ),
                    ),
            )
            .child(
                vstack(7.)
                    .w_full()
                    .child(
                        div()
                            .w_full()
                            .h(theme.input.height_300)
                            .px(px(15.))
                            .flex()
                            .items_center()
                            .bg(theme.bg._200)
                            .rounded_full()
                            .child(field)
                            .guide_anchor(self.guide_anchor_for(RiseGuideAnchor::StickerPackName)),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_start()
                            .when_some(title_error_text, |row, error| row.child(ErrorTextUi::new(error).px(12.)))
                            .child(spacer_min_w(8.))
                            .child(SecondaryText::new(format!("{count}/{maximum}")).px(11.)),
                    ),
            )
            .child(measure(self.child_id("height"), move |bounds, window, cx| {
                let height = f32::from(bounds.size.height);
                measuring.update(cx, |state, cx| state.update_resting_fraction(height, window, cx));
            }))
            .into_any_element()
    }

    fn create_button(&self, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let can_create = self.state.read(cx).can_create(self.is_submitting);
        let title = tr("create_action");
        let submitting = self.state.clone();
        let tint = if can_create { theme.primary._100 } else { theme.bg._300 };
        let ink = theme.content_on(tint);

        div()
            .w_full()
            .flex()
            .flex_col()
            .pb(px(12.))
            .child(
                LiquidGlassButton::new(self.child_id("create"), LiquidGlassButtonShape::Capsule)
                    .horizontal_padding(22.)
                    .vertical_padding(13.)
                    .tint(Some(tint))
                    .haptic_on_press(can_create)
                    .disabled(!can_create)
                    .accessibility_label(Some(title.clone()))
                    .on_press(move |window, cx| {
                        submitting.update(cx, |state, cx| state.submit(window, cx));
                    })
                    .child(
                        hstack(8.)
                            .w_full()
                            .justify_center()
                            .when(self.is_submitting, |row| {
                                row.child(progress_view().control_size(ControlSize::Small).tint(ink))
                            })
                            .child(
                                MainText::new(title)
                                    .px(16.)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(if can_create || self.is_submitting { ink } else { theme.text.secondary }),
                            ),
                    )
                    .guide_anchor(self.guide_anchor_for(RiseGuideAnchor::StickerPackCreateButton)),
            )
            .into_any_element()
    }
}
