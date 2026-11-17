use std::rc::Rc;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, FontWeight, Global, Hsla, KeyBinding, KeyDownEvent, SharedString,
    Subscription, Window, actions, container_query, hsla, white,
};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_ui::keyboard_dismiss::{LayerFocus, ScreenFocus};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::input_ui::Confirm;
use rise_ui::input_ui::input_ui::InputUi;
use rise_ui::input_ui::input_ui_state::{InputUiEvent, InputUiState};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::swiftui::material::UIBlurEffectStyle;
use rise_ui::swiftui::prelude::*;
use rise_ui::theme;

actions!(editor_bottom_bar, [InsertLineBreak]);

const CAPTION_KEY_CONTEXT: &str = "EditorBottomBarCaption";

struct EditorCaptionKeyBindingsInstalled;

impl Global for EditorCaptionKeyBindingsInstalled {}

fn install_caption_key_bindings(cx: &mut App) {
    if cx.has_global::<EditorCaptionKeyBindingsInstalled>() {
        return;
    }
    cx.set_global(EditorCaptionKeyBindingsInstalled);
    cx.bind_keys([KeyBinding::new("shift-enter", InsertLineBreak, Some(CAPTION_KEY_CONTEXT))]);
}

/// esc leaves a panel, the desktop's way out besides the close button
/// the panel takes the keyboard when it appears and a control inside still gets its own keys first
pub(crate) fn editor_panel_escape(
    id: &'static str,
    appear_id: &'static str,
    on_close: ButtonAction,
    content: impl IntoElement,
    window: &mut Window,
    cx: &mut App,
) -> Lifecycle {
    let focus = window.use_keyed_state::<FocusHandle>(id, cx, |_, cx| cx.focus_handle()).read(cx).clone();
    let layer_focus = LayerFocus::keyed(appear_id, window, cx);
    let (layer_appearing, layer_leaving) = (layer_focus.clone(), layer_focus);
    let appearing = focus.clone();
    let leaving = focus.clone();
    div()
        .id(id)
        .w_full()
        .track_focus(&focus)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
                on_close(window, cx);
            }
        })
        .child(content)
        .on_appear(appear_id, move |window, cx| LayerFocus::did_appear(&layer_appearing, &appearing, window, cx))
        .and_on_disappear(move |window, cx| {
            if let Some(window) = window {
                LayerFocus::will_disappear(&layer_leaving, &leaving, window, cx);
            }
        })
}

#[derive(IntoElement)]
pub struct EditorBottomBar {
    pub caption: Binding<String>,
    pub shows_caption: bool,
    pub on_back: ButtonAction,
    pub on_send: ButtonAction,
    pub on_crop: ButtonAction,
    pub on_draw: ButtonAction,
    pub on_adjust: ButtonAction,
    pub has_attached_track: bool,
    pub on_music: Option<ButtonAction>,
    pub has_person_tags: bool,
    pub on_tag_people: Option<ButtonAction>,
    pub shows_crop_and_adjust: bool,
    pub on_add_photo: Option<ButtonAction>,
    pub add_photo_enabled: bool,
}

impl EditorBottomBar {
    const TOOL_MAX_WIDTH: f32 = 42.;
    const TOOL_MIN_WIDTH: f32 = 32.;
    const ROW_HEIGHT: f32 = 52.;
    const TOOLS_MIN_GAP: f32 = 8.;
    const TOOLS_INSET: f32 = 5.;

    pub fn new(
        caption: Binding<String>,
        on_back: impl Fn(&mut Window, &mut App) + 'static,
        on_send: impl Fn(&mut Window, &mut App) + 'static,
        on_crop: impl Fn(&mut Window, &mut App) + 'static,
        on_draw: impl Fn(&mut Window, &mut App) + 'static,
        on_adjust: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            caption,
            shows_caption: true,
            on_back: Rc::new(on_back),
            on_send: Rc::new(on_send),
            on_crop: Rc::new(on_crop),
            on_draw: Rc::new(on_draw),
            on_adjust: Rc::new(on_adjust),
            has_attached_track: false,
            on_music: None,
            has_person_tags: false,
            on_tag_people: None,
            shows_crop_and_adjust: true,
            on_add_photo: None,
            add_photo_enabled: true,
        }
    }

    pub fn shows_caption(mut self, shows_caption: bool) -> Self {
        self.shows_caption = shows_caption;
        self
    }

    pub fn has_attached_track(mut self, has_attached_track: bool) -> Self {
        self.has_attached_track = has_attached_track;
        self
    }

    pub fn on_music(mut self, on_music: Option<ButtonAction>) -> Self {
        self.on_music = on_music;
        self
    }

    pub fn has_person_tags(mut self, has_person_tags: bool) -> Self {
        self.has_person_tags = has_person_tags;
        self
    }

    pub fn on_tag_people(mut self, on_tag_people: Option<ButtonAction>) -> Self {
        self.on_tag_people = on_tag_people;
        self
    }

    pub fn shows_crop_and_adjust(mut self, shows_crop_and_adjust: bool) -> Self {
        self.shows_crop_and_adjust = shows_crop_and_adjust;
        self
    }

    pub fn on_add_photo(mut self, on_add_photo: Option<ButtonAction>) -> Self {
        self.on_add_photo = on_add_photo;
        self
    }

    pub fn add_photo_enabled(mut self, add_photo_enabled: bool) -> Self {
        self.add_photo_enabled = add_photo_enabled;
        self
    }

    fn tool_width(row_width: f32, tool_count: usize) -> f32 {
        let reserved = 2. * (Self::ROW_HEIGHT + Self::TOOLS_MIN_GAP + Self::TOOLS_INSET);
        ((row_width - reserved) / tool_count as f32).clamp(Self::TOOL_MIN_WIDTH, Self::TOOL_MAX_WIDTH)
    }

    fn tool_frame(icon: &str, width: f32) -> Div {
        div()
            .relative()
            .w(px(width))
            .h(px(Self::ROW_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(icon, px(16.), white(), false))
    }

    fn plain_icon(id: &'static str, name: &str, width: f32, action: ButtonAction) -> PlainButton {
        plain(id).on_press(move |window, cx| action(window, cx)).child(Self::tool_frame(name, width))
    }

    /// not plain_icon, the corner dot says the slide has a track or tags
    fn dotted_icon(
        id: &'static str,
        name: &str,
        width: f32,
        accessibility_label: String,
        has_dot: bool,
        dot_color: Hsla,
        action: ButtonAction,
    ) -> PlainButton {
        let frame = Self::tool_frame(name, width).when(has_dot, |frame| {
            frame.child(div().absolute().right(px(7.)).top(px(14.)).size(px(8.)).circle().bg(dot_color))
        });
        let hint = SharedString::from(accessibility_label.clone());
        plain(id).on_press(move |window, cx| action(window, cx)).child(
            div()
                .id(SharedString::from(format!("{id}-label")))
                .aria_label(accessibility_label)
                .tooltip_label(hint)
                .child(frame),
        )
    }
}

struct EditorCaptionSession {
    field: Entity<InputUiState>,
    caption: Option<Binding<String>>,
    _subscriptions: Vec<Subscription>,
}

impl EditorCaptionSession {
    const FONT_SIZE: f32 = 15.;
    const MAX_LINES: usize = 4;

    fn new(seed: String, tint: Hsla, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            let mut field = InputUiState::multi_line(cx);
            field.set_chromeless(true, cx);
            field.set_wraps(true, cx);
            field.set_max_visible_lines(Self::MAX_LINES, cx);
            field.set_font_size(Some(px(Self::FONT_SIZE)), cx);
            field.set_text_color(Some(white()), cx);
            field.set_placeholder(tr("editor_caption_placeholder"), cx);
            field.set_placeholder_color(Some(white().opacity(0.55)), cx);
            field.set_caret_color(Some(tint), cx);
            field.reset(seed, cx);
            field
        });
        let subscriptions = vec![cx.subscribe_in(&field, window, |this, field, event: &InputUiEvent, window, cx| {
            match event {
                InputUiEvent::Changed => {
                    let text = field.read(cx).text().to_owned();
                    if let Some(caption) = this.caption.clone() {
                        caption.set(text, window, cx);
                    }
                }
                InputUiEvent::Cancelled => ScreenFocus::release_text_field(window, cx),
                InputUiEvent::Submitted => {}
            }
        })];
        Self { field, caption: None, _subscriptions: subscriptions }
    }

    fn apply_external_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let field = self.field.read(cx);
        if field.text() == text || field.marked_range().is_some() {
            return;
        }
        self.field.update(cx, |field, cx| field.set_text(text, cx));
    }
}

impl RenderOnce for EditorBottomBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tint = theme(cx).primary._100;
        let caption_value = self.caption.get(cx);
        let seed = caption_value.clone();
        let session = window.use_keyed_state("editor-bottom-bar-caption", cx, move |window, cx| {
            EditorCaptionSession::new(seed, tint, window, cx)
        });
        let binding = self.caption.clone();
        session.update(cx, |session, cx| {
            session.caption = Some(binding);
            session.apply_external_text(&caption_value, cx);
        });
        let field = session.read(cx).field.clone();
        let focus = field.read(cx).focus_handle(cx);

        let on_back = self.on_back.clone();
        let back = LiquidGlassButton::new("editor-bottom-bar-back", LiquidGlassButtonShape::Circle)
            .size(52.)
            .accessibility_label(Some(tr("close")))
            .on_press(move |window, cx| {
                if focus.is_focused(window) {
                    ScreenFocus::release_text_field(window, cx);
                }
                on_back(window, cx);
            })
            .children(IconUi::sized("chevron.backward", px(20.), white(), false))
            .guide_anchor(RiseGuideAnchor::EditorBack);

        install_caption_key_bindings(cx);
        let key_send = self.on_send.clone();
        let line_break_field = field.downgrade();
        let on_send = self.on_send;
        let send = LiquidGlassButton::new("editor-bottom-bar-send", LiquidGlassButtonShape::Circle)
            .size(52.)
            .tint(Some(tint))
            .accessibility_label(Some(tr("media_picker_send")))
            .on_press(move |window, cx| on_send(window, cx))
            .children(IconUi::sized("arrow.up", px(19.), theme(cx).content_on(tint), false));

        let shows_crop_and_adjust = self.shows_crop_and_adjust;
        let tool_count = 1
            + 2 * usize::from(shows_crop_and_adjust)
            + usize::from(self.on_add_photo.is_some())
            + usize::from(self.on_tag_people.is_some())
            + usize::from(self.on_music.is_some());
        let on_add_photo = self.on_add_photo;
        let add_photo_enabled = self.add_photo_enabled;
        let on_crop = self.on_crop;
        let on_draw = self.on_draw;
        let on_adjust = self.on_adjust;
        let on_tag_people = self.on_tag_people;
        let on_music = self.on_music;
        let has_person_tags = self.has_person_tags;
        let has_attached_track = self.has_attached_track;
        let tools_row = container_query(move |size, _, _| {
            let width = Self::tool_width(f32::from(size.width), tool_count);
            let tools = hstack(0.)
                .when(shows_crop_and_adjust, |row| {
                    row.child(Self::plain_icon("editor-bottom-bar-crop", "crop", width, on_crop))
                })
                .child(Self::plain_icon("editor-bottom-bar-draw", "scribble.variable", width, on_draw))
                .when(shows_crop_and_adjust, |row| {
                    row.child(
                        Self::plain_icon("editor-bottom-bar-adjust", "slider.horizontal.3", width, on_adjust)
                            .guide_anchor(RiseGuideAnchor::EditorToolAdjust),
                    )
                })
                .when_some(on_add_photo, |row, action| {
                    row.child(
                        Self::dotted_icon(
                            "editor-bottom-bar-add-photo",
                            "photo.on.rectangle",
                            width,
                            tr("create_post_add_photos"),
                            false,
                            tint,
                            action,
                        )
                        .opacity(if add_photo_enabled { 1. } else { 0.4 }),
                    )
                })
                .when_some(on_tag_people, |row, action| {
                    row.child(Self::dotted_icon(
                        "editor-bottom-bar-tag-people",
                        "person.crop.circle.badge.plus",
                        width,
                        tr("media_editor_tag_people_tool"),
                        has_person_tags,
                        tint,
                        action,
                    ))
                })
                .when_some(on_music, |row, action| {
                    row.child(Self::dotted_icon(
                        "editor-bottom-bar-music",
                        "music.note",
                        width,
                        tr("post_attach_music"),
                        has_attached_track,
                        tint,
                        action,
                    ))
                })
                .px(px(Self::TOOLS_INSET))
                .h(px(Self::ROW_HEIGHT))
                .liquid_chrome_capsule_with(0.08, true, None, false)
                .guide_anchor(RiseGuideAnchor::EditorTools);
            hstack(0.)
                .w_full()
                .child(back)
                .child(spacer_min_w(Self::TOOLS_MIN_GAP))
                .child(div().flex_none().occlude().child(tools))
                .child(spacer_min_w(Self::TOOLS_MIN_GAP))
                .child(send)
        });

        vstack(10.)
            .w_full()
            .when(self.shows_caption, |bar| {
                bar.child(
                    div()
                        .key_context(CAPTION_KEY_CONTEXT)
                        .w_full()
                        .capture_action(move |_: &Confirm, window, cx| {
                            key_send(window, cx);
                            cx.stop_propagation();
                        })
                        .on_action(move |_: &InsertLineBreak, window, cx| {
                            if let Some(field) = line_break_field.upgrade() {
                                field.update(cx, |field, cx| field.confirm(&Confirm, window, cx));
                            }
                        })
                        .child(
                        div()
                            .w_full()
                            .occlude()
                            .px(px(16.))
                            .py(px(12.))
                            .child(InputUi::new(field))
                            .liquid_chrome_rounded_rectangle_with(18., 0.12, None, true),
                    ),
                )
            })
            .child(div().w_full().h(px(Self::ROW_HEIGHT)).child(tools_row))
            .px(px(12.))
            .pt(px(8.))
            .pb(px(22.))
    }
}

/// bottom bar of tagging mode
/// canvas belongs to taps on the frame here so no tools
/// only close and confirm like drawing and color use
#[derive(IntoElement)]
pub struct EditorTagPeoplePanel {
    pub tagged_count: i64,
    pub on_close: ButtonAction,
    pub on_confirm: ButtonAction,
}

impl EditorTagPeoplePanel {
    pub fn new(
        tagged_count: i64,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { tagged_count, on_close: Rc::new(on_close), on_confirm: Rc::new(on_confirm) }
    }
}

impl RenderOnce for EditorTagPeoplePanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let on_close = self.on_close.clone();
        let on_confirm = self.on_confirm;
        let content = hstack(14.)
            .w_full()
            .child(
                LiquidGlassButton::new("editor-tag-people-close", LiquidGlassButtonShape::Circle)
                    .size(48.)
                    .accessibility_label(Some(tr("close")))
                    .on_press(move |window, cx| on_close(window, cx))
                    .children(IconUi::sized("chevron.backward", px(17.), white(), false)),
            )
            .child(spacer_min_w(0.))
            .when(self.tagged_count > 0, |row| {
                row.child(
                    hstack(6.)
                        .text_color(white())
                        .children(IconUi::sized("person.crop.circle.badge.checkmark", px(14.), white(), false))
                        .child(
                            div()
                                .swiftui_font(Font::system(13., FontWeight::SEMIBOLD, FontDesign::Default))
                                .child(self.tagged_count.to_string()),
                        )
                        .px(px(14.))
                        .h(px(48.))
                        .liquid_chrome_capsule_with(0.08, true, None, false),
                )
            })
            .child(spacer_min_w(0.))
            .child(
                LiquidGlassButton::new("editor-tag-people-confirm", LiquidGlassButtonShape::Circle)
                    .size(48.)
                    .accessibility_label(Some(tr("done")))
                    .on_press(move |window, cx| on_confirm(window, cx))
                    .children(IconUi::sized("checkmark", px(16.), white(), false)),
            )
            .px(px(14.))
            .pt(px(6.))
            .pb(px(22.));
        editor_panel_escape("editor-tag-people-panel", "editor-tag-people-panel-appear", self.on_close, content, window, cx)
    }
}

#[derive(IntoElement)]
pub struct EditorBlurView {
    pub style: UIBlurEffectStyle,
}

impl EditorBlurView {
    pub fn new(style: UIBlurEffectStyle) -> Self {
        Self { style }
    }

    fn is_dark(style: UIBlurEffectStyle) -> bool {
        matches!(
            style,
            UIBlurEffectStyle::SystemUltraThinMaterialDark
                | UIBlurEffectStyle::SystemThinMaterialDark
                | UIBlurEffectStyle::SystemMaterialDark
                | UIBlurEffectStyle::SystemThickMaterialDark
                | UIBlurEffectStyle::SystemChromeMaterialDark
        )
    }
}

impl RenderOnce for EditorBlurView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let lightness = if Self::is_dark(self.style) { 0. } else { 1. };
        div().size_full().bg(hsla(0., 0., lightness, self.style.painted_opacity()))
    }
}
