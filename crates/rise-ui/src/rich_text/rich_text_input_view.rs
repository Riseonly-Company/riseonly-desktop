use std::rc::Rc;

use gpui::{
    App, AppContext, Context, Edges, ElementId, Entity, Focusable, Hsla, IntoElement, ParentElement, Pixels, RenderOnce,
    SharedString, Styled, Subscription, Window, div, px,
};
use rise_core::foundation::ns_range::{NSRange, byte_to_utf16_range, length_utf16, utf16_to_byte_range};
use rise_i18n::i18n::tr;

use crate::context_menu_ui::ContextMenuItem;
use crate::input_ui::{
    InputEditSubmenu, InputMode, InputUiEvent, InputUiState, RunAttributes, Selection,
};
use crate::keyboard_dismiss::EditMenuPresentation;
use crate::rich_text::rich_text_markup::{RichEditorStyle, RichTextMarkup, RichTextMarkupFormat};
use crate::swiftui::binding::Binding;
use crate::swiftui::font::FontDesign;
use crate::swiftui::rich_text::{AttributeContainer, AttributedString, RichFont};
use crate::text_input_caret::TextInputCaret;

type HeightChange = Rc<dyn Fn(f32, &mut Window, &mut App)>;
type FocusChange = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// multiline editor that keeps markdown markers in the text and dims them
/// editing engine is the kit InputUiState, this view owns markup styling and the placeholder
#[derive(IntoElement)]
pub struct RichTextInputView {
    id: ElementId,
    text: Binding<String>,
    placeholder: String,
    font_size: f32,
    text_color: Hsla,
    placeholder_color: Hsla,
    tint_color: Hsla,
    marker_color: Hsla,
    code_color: Hsla,
    code_background: Hsla,
    text_container_inset: Edges<Pixels>,
    is_scroll_enabled: bool,
    focus_token: i64,
    on_height_change: Option<HeightChange>,
    on_focus_change: Option<FocusChange>,
}

impl RichTextInputView {
    pub fn new(id: impl Into<ElementId>, text: Binding<String>) -> Self {
        let primary = Hsla::default();
        Self {
            id: id.into(),
            text,
            placeholder: String::new(),
            font_size: 15.0,
            text_color: primary,
            placeholder_color: primary,
            tint_color: primary,
            marker_color: primary,
            code_color: primary,
            code_background: primary,
            text_container_inset: Edges {
                top: px(8.0),
                left: px(4.0),
                bottom: px(8.0),
                right: px(4.0),
            },
            is_scroll_enabled: false,
            focus_token: 0,
            on_height_change: None,
            on_focus_change: None,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    pub fn text_color(mut self, text_color: Hsla) -> Self {
        self.text_color = text_color;
        self
    }

    pub fn placeholder_color(mut self, placeholder_color: Hsla) -> Self {
        self.placeholder_color = placeholder_color;
        self
    }

    pub fn tint_color(mut self, tint_color: Hsla) -> Self {
        self.tint_color = tint_color;
        self
    }

    pub fn marker_color(mut self, marker_color: Hsla) -> Self {
        self.marker_color = marker_color;
        self
    }

    pub fn code_color(mut self, code_color: Hsla) -> Self {
        self.code_color = code_color;
        self
    }

    pub fn code_background(mut self, code_background: Hsla) -> Self {
        self.code_background = code_background;
        self
    }

    pub fn autocorrection(self, _autocorrection: bool) -> Self {
        self
    }

    pub fn text_container_inset(mut self, text_container_inset: Edges<Pixels>) -> Self {
        self.text_container_inset = text_container_inset;
        self
    }

    pub fn is_scroll_enabled(mut self, is_scroll_enabled: bool) -> Self {
        self.is_scroll_enabled = is_scroll_enabled;
        self
    }

    pub fn focus_token(mut self, focus_token: i64) -> Self {
        self.focus_token = focus_token;
        self
    }

    pub fn on_height_change(mut self, on_height_change: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_height_change = Some(Rc::new(on_height_change));
        self
    }

    pub fn on_focus_change(mut self, on_focus_change: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_focus_change = Some(Rc::new(on_focus_change));
        self
    }
}

/// what the coordinator reads from the latest render
#[derive(Clone)]
struct RichTextInputViewParent {
    text: Binding<String>,
    font_size: f32,
    text_color: Hsla,
    marker_color: Hsla,
    code_color: Hsla,
    code_background: Hsla,
    text_container_inset: Edges<Pixels>,
    on_height_change: Option<HeightChange>,
    on_focus_change: Option<FocusChange>,
}

pub struct RichTextInputViewCoordinator {
    parent: RichTextInputViewParent,
    field: Entity<InputUiState>,
    was_styled: bool,
    last_applied: Option<(String, bool)>,
    last_focus_token: i64,
    last_text_color: Option<Hsla>,
    last_published_height: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

impl RichTextInputViewCoordinator {
    fn new(parent: RichTextInputViewParent, seed: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            let mut field = InputUiState::new(InputMode::MultiLine, cx);
            field.set_chromeless(true, cx);
            field.reset(seed, cx);
            field
        });
        let handle = field.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, _, event: &InputUiEvent, window, cx| {
                if *event == InputUiEvent::Changed {
                    this.text_view_did_change(window, cx);
                }
            }),
            cx.on_focus(&handle, window, |this, window, cx| this.text_view_did_begin_editing(window, cx)),
            cx.on_blur(&handle, window, |this, window, cx| this.text_view_did_end_editing(window, cx)),
        ];
        field.update(cx, |field, _| {
            field.set_on_edit_menu_presented_change(Some(Rc::new(|is_visible: bool, _: &mut Window, _: &mut App| {
                EditMenuPresentation::set_is_visible(is_visible);
            })));
        });
        let mut coordinator = Self {
            parent,
            field,
            was_styled: false,
            last_applied: None,
            last_focus_token: 0,
            last_text_color: None,
            last_published_height: None,
            _subscriptions: subscriptions,
        };
        coordinator.install_format_menu(cx);
        let color = coordinator.parent.text_color;
        coordinator.apply_text_color_if_changed(color, cx);
        coordinator.restyle(true, cx);
        coordinator.note_applied_text(cx);
        coordinator
    }

    fn apply_text_color_if_changed(&mut self, color: Hsla, cx: &mut Context<Self>) -> bool {
        if self.last_text_color == Some(color) {
            return false;
        }
        self.last_text_color = Some(color);
        self.field.update(cx, |field, cx| field.set_text_color(Some(color), cx));
        true
    }

    fn base_font(&self) -> RichFont {
        RichFont::system(self.parent.font_size)
    }

    fn editor_style(&self) -> RichEditorStyle {
        RichEditorStyle::make(
            self.parent.font_size,
            self.parent.text_color,
            self.parent.marker_color,
            self.parent.code_color,
            self.parent.code_background,
        )
    }

    fn base_typing_attributes(&self) -> AttributeContainer {
        AttributeContainer {
            font: Some(self.base_font()),
            foreground_color: Some(self.parent.text_color),
            ..AttributeContainer::default()
        }
    }

    fn text_view_did_begin_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_focus_change) = self.parent.on_focus_change.clone() {
            on_focus_change(true, window, cx);
        }
    }

    fn text_view_did_end_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_focus_change) = self.parent.on_focus_change.clone() {
            on_focus_change(false, window, cx);
        }
        EditMenuPresentation::set_is_visible(false);
    }

    /// records what the field holds now
    /// the field reports every install as Changed later in the same flush
    /// so a flag around the install would be gone by then
    fn note_applied_text(&mut self, cx: &mut Context<Self>) {
        let field = self.field.read(cx);
        self.last_applied = Some((field.text().to_owned(), field.marked_range().is_some()));
    }

    fn text_view_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (text, composing) = {
            let field = self.field.read(cx);
            (field.text().to_owned(), field.marked_range().is_some())
        };
        if self.last_applied.as_ref().is_some_and(|(applied, was_composing)| *applied == text && *was_composing == composing) {
            self.publish_height(window, cx);
            return;
        }
        self.last_applied = Some((text.clone(), composing));
        self.parent.text.set(text, window, cx);
        self.restyle(false, cx);
        self.publish_height(window, cx);
    }

    fn install_format_menu(&mut self, cx: &mut Context<Self>) {
        let coordinator = cx.weak_entity();
        let items: Vec<ContextMenuItem> = RichTextMarkupFormat::ALL_CASES
            .iter()
            .enumerate()
            .map(|(index, format)| {
                let format = *format;
                let coordinator = coordinator.clone();
                ContextMenuItem::new(
                    index as i64,
                    tr(format.title_key()),
                    Some(format.sf_symbol().to_owned()),
                    format.title_key(),
                    move |window, cx| {
                        if let Some(coordinator) = coordinator.upgrade() {
                            coordinator.update(cx, |coordinator, cx| coordinator.apply_format(format, window, cx));
                        }
                    },
                )
            })
            .collect();
        let submenu = InputEditSubmenu {
            title: tr("format_title"),
            icon: Some("textformat".to_owned()),
            items,
        };
        self.field
            .update(cx, |field, cx| field.set_edit_submenu(Some(submenu), cx));
    }

    fn apply_format(&mut self, format: RichTextMarkupFormat, window: &mut Window, cx: &mut Context<Self>) {
        let (raw, selection) = {
            let field = self.field.read(cx);
            let raw = field.text().to_owned();
            let selection = byte_to_utf16_range(&raw, field.selection().range());
            (raw, selection)
        };
        let (text, selection) = RichTextMarkup::apply_format(format, &raw, selection);
        self.field.update(cx, |field, cx| field.set_text(&text, cx));
        self.note_applied_text(cx);
        self.parent.text.set(text.clone(), window, cx);
        self.restyle(true, cx);
        let clamped = Self::clamp(selection, &text);
        let bytes = utf16_to_byte_range(&text, clamped).unwrap_or(text.len()..text.len());
        self.field.update(cx, |field, cx| {
            field.set_selection(Selection::new(bytes.start, bytes.end), cx);
        });
        self.publish_height(window, cx);
    }

    fn restyle(&mut self, force: bool, cx: &mut Context<Self>) {
        let (raw, composing) = {
            let field = self.field.read(cx);
            (field.text().to_owned(), field.marked_range().is_some())
        };
        let needs = RichTextMarkup::has_any_markup(&raw);
        if !(needs || self.was_styled || force) {
            return;
        }
        if composing {
            return;
        }
        let attributed = if needs {
            self.was_styled = true;
            RichTextMarkup::editor_attributed(&raw, &self.editor_style())
        } else {
            self.was_styled = false;
            AttributedString::with_attributes(raw, self.base_typing_attributes())
        };
        let pieces: Vec<(String, RunAttributes)> = attributed
            .runs()
            .map(|(range, attributes)| {
                (attributed.characters()[range].to_owned(), RunAttributes::new(attributes.clone()))
            })
            .collect();
        let typing = RunAttributes::new(self.base_typing_attributes());
        self.field
            .update(cx, |field, cx| field.set_attributed_text(pieces, typing, cx));
        self.note_applied_text(cx);
    }

    fn clamp(range: NSRange, text: &str) -> NSRange {
        let length = length_utf16(text);
        let loc = range.location.min(length);
        let len = range.length.min(length - loc);
        NSRange::new(loc, len)
    }

    fn publish_height(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(on_height_change) = self.parent.on_height_change.clone() else {
            return;
        };
        let (line_count, line_height) = {
            let field = self.field.read(cx);
            (field.display().line_count(), field.line_height())
        };
        if line_height <= px(0.0) {
            return;
        }
        let inset = self.parent.text_container_inset;
        let measured = f32::from(line_height) * line_count as f32 + f32::from(inset.top) + f32::from(inset.bottom);
        if self.last_published_height == Some(measured) {
            return;
        }
        self.last_published_height = Some(measured);
        on_height_change(measured, window, cx);
    }

    fn consume_focus_token(&mut self, token: i64) -> bool {
        if token == self.last_focus_token {
            return false;
        }
        self.last_focus_token = token;
        token > 0
    }

    fn update_ui_view(&mut self, view: &RichTextInputView, window: &mut Window, cx: &mut Context<Self>) {
        self.parent = RichTextInputViewParent::from(view);
        let text = view.text.get(cx);
        let tint = view.tint_color;
        let is_scroll_enabled = view.is_scroll_enabled;
        let (current, composing, selection) = {
            let field = self.field.read(cx);
            (field.text().to_owned(), field.marked_range().is_some(), field.selection().range())
        };
        let external_change = current != text && !composing;
        if external_change {
            let carried = TextInputCaret::carried(byte_to_utf16_range(&current, selection), &current, &text);
            let bytes = utf16_to_byte_range(&text, carried).unwrap_or(text.len()..text.len());
            self.field.update(cx, |field, cx| {
                field.set_text(&text, cx);
                field.set_selection(Selection::new(bytes.start, bytes.end), cx);
            });
            self.note_applied_text(cx);
        }
        let color_changed = self.apply_text_color_if_changed(view.text_color, cx);
        if external_change || color_changed {
            self.restyle(true, cx);
        }
        let placeholder: SharedString = view.placeholder.clone().into();
        let placeholder_color = view.placeholder_color;
        let font_size = px(view.font_size);
        self.field.update(cx, |field, cx| {
            field.set_caret_color(Some(tint), cx);
            field.set_scroll_enabled(is_scroll_enabled, cx);
            if *field.placeholder() != placeholder {
                field.set_placeholder(placeholder, cx);
            }
            field.set_placeholder_color(Some(placeholder_color), cx);
            field.set_font_size(Some(font_size), cx);
        });
        if self.consume_focus_token(view.focus_token) {
            let handle = self.field.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| handle.focus(window, cx));
        }
        let coordinator = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            if let Some(coordinator) = coordinator.upgrade() {
                coordinator.update(cx, |coordinator, cx| coordinator.publish_height(window, cx));
            }
        });
    }
}

impl From<&RichTextInputView> for RichTextInputViewParent {
    fn from(view: &RichTextInputView) -> Self {
        Self {
            text: view.text.clone(),
            font_size: view.font_size,
            text_color: view.text_color,
            marker_color: view.marker_color,
            code_color: view.code_color,
            code_background: view.code_background,
            text_container_inset: view.text_container_inset,
            on_height_change: view.on_height_change.clone(),
            on_focus_change: view.on_focus_change.clone(),
        }
    }
}

impl RenderOnce for RichTextInputView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let parent = RichTextInputViewParent::from(&self);
        let seed = self.text.get(cx);
        let coordinator = window.use_keyed_state(self.id.clone(), cx, move |window, cx| {
            RichTextInputViewCoordinator::new(parent, seed, window, cx)
        });
        coordinator.update(cx, |coordinator, cx| coordinator.update_ui_view(&self, window, cx));
        let field = coordinator.read(cx).field.clone();
        let inset = self.text_container_inset;
        div()
            .w_full()
            .pt(inset.top)
            .pl(inset.left)
            .pb(inset.bottom)
            .pr(inset.right)
            .child(field)
    }
}

impl RichEditorStyle {
    pub fn make(font_size: f32, text_color: Hsla, marker_color: Hsla, code_color: Hsla, code_background: Hsla) -> Self {
        RichEditorStyle::new(
            RichFont::system(font_size),
            text_color,
            marker_color,
            RichFont::system_design(font_size * 0.95, FontDesign::Monospaced),
            code_color,
            code_background,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use gpui::{Render, TestAppContext, VisualTestContext, WindowHandle, black};
    use rise_theme::AppTheme;

    use super::*;

    struct TestHost;

    impl Render for TestHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    struct Mounted {
        window: WindowHandle<TestHost>,
        coordinator: Entity<RichTextInputViewCoordinator>,
        passes: Rc<Cell<usize>>,
    }

    fn settle(window: WindowHandle<TestHost>, cx: &mut TestAppContext) {
        VisualTestContext::from_window(window.into(), cx).run_until_parked();
    }

    /// counts the writes to the binding, one write is one did change pass
    fn mount(seed: &str, cx: &mut TestAppContext) -> Mounted {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                crate::install_theme(AppTheme::dark(), cx);
            }
        });
        let value = Rc::new(RefCell::new(seed.to_owned()));
        let passes = Rc::new(Cell::new(0));
        let (reader, writer, counter) = (value.clone(), value, passes.clone());
        let parent = RichTextInputViewParent {
            text: Binding::new(
                move |_| reader.borrow().clone(),
                move |text, _, _| {
                    counter.set(counter.get() + 1);
                    *writer.borrow_mut() = text;
                },
            ),
            font_size: 15.0,
            text_color: black(),
            marker_color: black(),
            code_color: black(),
            code_background: black(),
            text_container_inset: Edges::default(),
            on_height_change: None,
            on_focus_change: None,
        };
        let seed = seed.to_owned();
        let window = cx.add_window(|_, _| TestHost);
        let coordinator = window
            .update(cx, move |_, window, cx| {
                cx.new(|cx| RichTextInputViewCoordinator::new(parent, seed, window, cx))
            })
            .expect("window is open");
        settle(window, cx);
        Mounted { window, coordinator, passes }
    }

    #[gpui::test]
    fn styling_the_seed_is_not_an_edit(cx: &mut TestAppContext) {
        let mounted = mount("**bold**", cx);
        assert_eq!(mounted.passes.get(), 0);
    }

    #[gpui::test]
    fn an_edit_with_markup_takes_one_pass(cx: &mut TestAppContext) {
        let mounted = mount("**bold**", cx);
        let coordinator = mounted.coordinator.clone();
        mounted
            .window
            .update(cx, move |_, _, cx| {
                let field = coordinator.read(cx).field.clone();
                field.update(cx, |field, cx| field.set_text("**bold** more", cx));
            })
            .expect("window is open");
        settle(mounted.window, cx);
        assert_eq!(mounted.passes.get(), 1);
    }
}
